//! Real-time scanner orchestration for GlyphNet.

use std::collections::{BTreeMap, HashMap};

use glyphnet_core::{Frame, TransmissionMode};
use glyphnet_cv::{
    VisionProfile, adaptive_threshold, estimate_quad, find_anchor_candidates, grayscale,
    quad_dimensions, warp_perspective_gray,
};
use glyphnet_decode::{AutoDecodedSymbol, DecodeError, DecodeOptions, RasterDecoder};
use image::{DynamicImage, GrayImage};
use thiserror::Error;

/// Result type for scanner operations.
pub type Result<T> = std::result::Result<T, ScannerError>;

/// Scanner errors.
#[derive(Debug, Error)]
pub enum ScannerError {
    /// Wrapped decode error.
    #[error(transparent)]
    Decode(#[from] glyphnet_decode::DecodeError),
    /// Wrapped CV error.
    #[error(transparent)]
    Cv(#[from] glyphnet_cv::CvError),
    /// Conflicting burst metadata was observed.
    #[error("inconsistent burst metadata for stream {0}")]
    InconsistentBurst(u64),
}

/// Camera frame passed into the scanner.
#[derive(Debug, Clone)]
pub struct CameraFrame {
    /// Image payload.
    pub image: DynamicImage,
    /// Monotonic timestamp in microseconds.
    pub timestamp_micros: u64,
}

/// Pull-based camera source abstraction.
pub trait FrameSource {
    /// Return the next frame, or `None` when the source is exhausted.
    fn next_frame(&mut self) -> Option<CameraFrame>;
}

/// Scanner configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannerConfig {
    /// Expected mode used for CV tuning.
    pub mode: TransmissionMode,
    /// Raster decode options.
    pub decode: DecodeOptions,
    /// Maximum frames to consume in a single scan loop.
    pub max_frames: usize,
}

impl Default for ScannerConfig {
    fn default() -> Self {
        Self {
            mode: TransmissionMode::Print,
            decode: DecodeOptions::default(),
            max_frames: 120,
        }
    }
}

/// Result of scanning one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanEvent {
    /// Decoded protocol frame.
    pub frame: Frame,
    /// Complete payload when a static symbol or full burst stream is available.
    pub complete_payload: Option<Vec<u8>>,
    /// Capture timestamp for diagnostics.
    pub timestamp_micros: u64,
}

/// Axis-aligned scan region in pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanRegion {
    /// Left pixel.
    pub x: u32,
    /// Top pixel.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// Result of scanning a still image.
#[derive(Debug, Clone, PartialEq)]
pub struct StillScanResult {
    /// Auto-decoded symbol and inferred parameters.
    pub decoded: AutoDecodedSymbol,
    /// Crop region used for decoding, if any.
    pub crop: Option<ScanRegion>,
    /// Perspective quad used for rectification, if any.
    pub quad: Option<glyphnet_cv::Quad>,
    /// Output warp size when rectification is applied.
    pub warp_size: Option<(u32, u32)>,
}

/// Stateful real-time scanner.
#[derive(Debug)]
pub struct Scanner {
    config: ScannerConfig,
    decoder: RasterDecoder,
    bursts: HashMap<u64, BurstAssembler>,
}

impl Scanner {
    /// Create a scanner from configuration.
    pub fn new(config: ScannerConfig) -> Self {
        Self {
            decoder: RasterDecoder::new(config.decode.clone()),
            config,
            bursts: HashMap::new(),
        }
    }

    /// Borrow the active vision profile.
    pub fn vision_profile(&self) -> VisionProfile {
        VisionProfile::for_mode(self.config.mode)
    }

    /// Scan a single camera frame.
    pub fn scan_frame(&mut self, frame: CameraFrame) -> Result<ScanEvent> {
        let decoded = self.decoder.decode(&frame.image)?;
        let protocol_frame = decoded.frame;
        let complete_payload = if protocol_frame.header.frame_count == 1 {
            Some(protocol_frame.payload.clone())
        } else {
            let assembler = self
                .bursts
                .entry(protocol_frame.header.stream_id)
                .or_insert_with(|| BurstAssembler::new(protocol_frame.header.frame_count));
            assembler.push(&protocol_frame)?
        };

        Ok(ScanEvent {
            frame: protocol_frame,
            complete_payload,
            timestamp_micros: frame.timestamp_micros,
        })
    }

    /// Consume frames until a payload is complete or the configured frame limit is reached.
    pub fn scan_source<S: FrameSource>(&mut self, source: &mut S) -> Result<Option<Vec<u8>>> {
        for _ in 0..self.config.max_frames {
            let Some(frame) = source.next_frame() else {
                return Ok(None);
            };
            let event = self.scan_frame(frame)?;
            if let Some(payload) = event.complete_payload {
                return Ok(Some(payload));
            }
        }
        Ok(None)
    }
}

impl Default for Scanner {
    fn default() -> Self {
        Self::new(ScannerConfig::default())
    }
}

/// Scan a still image by attempting auto-decode, then a coarse crop if needed.
pub fn scan_still(image: &DynamicImage, mode: TransmissionMode) -> Result<StillScanResult> {
    let decoder = RasterDecoder::default();
    if let Ok(decoded) = decoder.decode_auto_with_info(image) {
        return Ok(StillScanResult {
            decoded,
            crop: None,
            quad: None,
            warp_size: None,
        });
    }

    let profile = VisionProfile::for_mode(mode);
    let gray = grayscale(image)?;
    let binary = adaptive_threshold(&gray, profile.threshold_radius, profile.threshold_bias)?;
    let candidates = find_anchor_candidates(&binary, profile)?;
    if let Some(quad) = estimate_quad(&binary, &candidates) {
        let (warp_width, warp_height) = quad_dimensions(quad);
        let warped = warp_perspective_gray(&gray, quad, warp_width, warp_height)?;
        let warped = DynamicImage::ImageLuma8(warped);
        let decoded = decoder.decode_auto_with_info(&warped)?;
        return Ok(StillScanResult {
            decoded,
            crop: None,
            quad: Some(quad),
            warp_size: Some((warp_width, warp_height)),
        });
    }

    let bounds = match dark_bounds(&binary) {
        Some(bounds) => bounds,
        None => return Err(DecodeError::AutoDetectFailed.into()),
    };
    let padding = profile.min_anchor_px.max(8);
    let expanded = expand_region(bounds, padding, image.width(), image.height());
    let cropped = image::imageops::crop_imm(
        image,
        expanded.x,
        expanded.y,
        expanded.width,
        expanded.height,
    )
    .to_image();
    let cropped = DynamicImage::ImageRgba8(cropped);
    let decoded = decoder.decode_auto_with_info(&cropped)?;

    Ok(StillScanResult {
        decoded,
        crop: Some(expanded),
        quad: None,
        warp_size: None,
    })
}

fn dark_bounds(binary: &GrayImage) -> Option<ScanRegion> {
    let mut min_x = binary.width();
    let mut min_y = binary.height();
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;
    for y in 0..binary.height() {
        for x in 0..binary.width() {
            if binary.get_pixel(x, y).0[0] == 0 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                found = true;
            }
        }
    }
    if !found {
        return None;
    }
    let width = max_x.saturating_sub(min_x).saturating_add(1);
    let height = max_y.saturating_sub(min_y).saturating_add(1);
    Some(ScanRegion {
        x: min_x,
        y: min_y,
        width,
        height,
    })
}

fn expand_region(region: ScanRegion, padding: u32, width: u32, height: u32) -> ScanRegion {
    let x = region.x.saturating_sub(padding);
    let y = region.y.saturating_sub(padding);
    let max_x = region
        .x
        .saturating_add(region.width)
        .saturating_add(padding)
        .min(width);
    let max_y = region
        .y
        .saturating_add(region.height)
        .saturating_add(padding)
        .min(height);
    ScanRegion {
        x,
        y,
        width: max_x.saturating_sub(x).max(1),
        height: max_y.saturating_sub(y).max(1),
    }
}

/// Burst frame assembly state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BurstAssembler {
    frame_count: u16,
    frames: BTreeMap<u16, Vec<u8>>,
}

impl BurstAssembler {
    /// Create burst assembly state.
    pub const fn new(frame_count: u16) -> Self {
        Self {
            frame_count,
            frames: BTreeMap::new(),
        }
    }

    /// Push a frame and return the full payload once all frames are present.
    pub fn push(&mut self, frame: &Frame) -> Result<Option<Vec<u8>>> {
        if frame.header.frame_count != self.frame_count {
            return Err(ScannerError::InconsistentBurst(frame.header.stream_id));
        }
        self.frames
            .entry(frame.header.frame_index)
            .or_insert_with(|| frame.payload.clone());

        if self.frames.len() == usize::from(self.frame_count) {
            let mut payload = Vec::new();
            for index in 0..self.frame_count {
                if let Some(chunk) = self.frames.get(&index) {
                    payload.extend_from_slice(chunk);
                }
            }
            return Ok(Some(payload));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use glyphnet_core::{EccLevel, Frame};
    use glyphnet_encode::Encoder;
    use glyphnet_render::RasterRenderer;
    use image::{Rgba, RgbaImage};

    use super::*;

    #[test]
    fn burst_assembler_returns_payload_when_complete() {
        let mut assembler = BurstAssembler::new(2);
        let first = Frame::new(
            TransmissionMode::Burst,
            EccLevel::Low,
            0,
            2,
            7,
            b"ab".to_vec(),
        )
        .unwrap();
        let second = Frame::new(
            TransmissionMode::Burst,
            EccLevel::Low,
            1,
            2,
            7,
            b"cd".to_vec(),
        )
        .unwrap();
        assert!(assembler.push(&second).unwrap().is_none());
        assert_eq!(assembler.push(&first).unwrap(), Some(b"abcd".to_vec()));
    }

    #[test]
    fn scan_still_crops_and_decodes() {
        let encoded = Encoder::default().encode_static(b"scan").unwrap();
        let symbol = RasterRenderer::default().render(&encoded.matrix).unwrap();
        let padding = 32;
        let mut canvas = RgbaImage::from_pixel(
            symbol.width() + padding * 2,
            symbol.height() + padding * 2,
            Rgba([255, 255, 255, 255]),
        );
        image::imageops::overlay(&mut canvas, &symbol, i64::from(padding), i64::from(padding));
        let image = DynamicImage::ImageRgba8(canvas);
        let result = scan_still(&image, TransmissionMode::Print).unwrap();
        assert_eq!(result.decoded.decoded.frame.payload, b"scan");
        if let Some(crop) = result.crop {
            assert!(crop.width <= image.width());
            assert!(crop.height <= image.height());
        }
        if let Some(quad) = result.quad {
            assert!(quad.area() > 0.0);
        }
        if let Some((width, height)) = result.warp_size {
            assert!(width > 0);
            assert!(height > 0);
        }
    }
}
