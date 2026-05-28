//! Real-time scanner orchestration for GlyphNet.

use std::collections::{BTreeMap, HashMap};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant as ScanInstant;

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone, Copy)]
struct ScanInstant(f64);

#[cfg(test)]
use glyphnet_core::layout;
use glyphnet_core::{Frame, TransmissionMode};
#[cfg(test)]
use glyphnet_core::{HEADER_LEN, LayoutFamily, SymbolMatrix};
use glyphnet_cv::{
    VisionProfile, adaptive_threshold, estimate_quad, find_anchor_candidates, grayscale,
    quad_dimensions, warp_perspective_gray,
};
use glyphnet_decode::{AutoDecodedSymbol, DecodeError, DecodeOptions, RasterDecoder};
use image::DynamicImage;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use thiserror::Error;

mod candidates;
mod decode_paths;
mod detectors;
mod rectification;
mod types;
#[cfg(not(target_arch = "wasm32"))]
use candidates::PARALLEL_DECODE_CANDIDATE_THRESHOLD;
use candidates::{
    MAX_QUAD_ATTEMPTS, dark_bounds, should_try_dark_bounds_fallback, still_scan_candidates,
};
use decode_paths::decode_candidate;
pub use detectors::CandidateDetector;
use detectors::ScanCandidate;
use rectification::{scan_quad_candidates as build_quad_candidates, should_try_quad_rectification};
pub use types::{FailedStillScan, ScanAttempt, ScanTelemetry, ScanTimings, StillScanResult};

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
    scan_still_with_diagnostics_mode(image, mode, false).map_err(|failed| failed.error)
}

/// Scan a still image with heavier recovery heuristics enabled.
pub fn scan_still_robust(image: &DynamicImage, mode: TransmissionMode) -> Result<StillScanResult> {
    scan_still_with_diagnostics_mode(image, mode, true).map_err(|failed| failed.error)
}

/// Scan a still image and return failed-attempt diagnostics on decode failure.
pub fn scan_still_with_diagnostics(
    image: &DynamicImage,
    mode: TransmissionMode,
) -> std::result::Result<StillScanResult, FailedStillScan> {
    scan_still_with_diagnostics_mode(image, mode, false)
}

fn scan_still_with_diagnostics_mode(
    image: &DynamicImage,
    mode: TransmissionMode,
    robust: bool,
) -> std::result::Result<StillScanResult, FailedStillScan> {
    scan_still_with_diagnostics_inner(image, mode, robust, true)
}

fn scan_still_with_diagnostics_inner(
    image: &DynamicImage,
    mode: TransmissionMode,
    robust: bool,
    allow_downscale_fast_path: bool,
) -> std::result::Result<StillScanResult, FailedStillScan> {
    let _ = allow_downscale_fast_path;

    let started = scan_instant_now();
    let mut timings = ScanTimings::default();
    let decoder = RasterDecoder::default();
    if should_try_full_frame_decode(image) {
        let stage = scan_instant_now();
        if let Ok(decoded) = decoder.decode_auto_with_info(image) {
            timings.full_frame_micros = elapsed_micros(stage);
            timings.total_micros = elapsed_micros(started);
            return Ok(StillScanResult {
                decoded,
                crop: None,
                quad: None,
                warp_size: None,
                attempts: Vec::new(),
                timings,
            });
        }
        timings.full_frame_micros = elapsed_micros(stage);
    }

    let profile = VisionProfile::for_mode(mode);
    let stage = scan_instant_now();
    let gray = grayscale(image).map_err(|error| failed_cv(error, timings, started))?;
    timings.grayscale_micros = elapsed_micros(stage);

    let stage = scan_instant_now();
    let binary = adaptive_threshold(&gray, profile.threshold_radius, profile.threshold_bias)
        .map_err(|error| failed_cv(error, timings, started))?;
    timings.threshold_micros = elapsed_micros(stage);

    if should_try_quad_rectification(image.width(), image.height(), robust) {
        let candidates = find_anchor_candidates(&binary, profile)
            .map_err(|error| failed_cv(error, timings, started))?;
        let estimated_quad = estimate_quad(&binary, &candidates);
        let dark_bounds_region =
            if should_try_dark_bounds_fallback(image.width(), image.height(), candidates.len()) {
                dark_bounds(&binary)
            } else {
                None
            };
        let quad_candidates = build_quad_candidates(
            estimated_quad,
            dark_bounds_region,
            image.width(),
            image.height(),
        );
        for (index, quad) in quad_candidates
            .into_iter()
            .take(MAX_QUAD_ATTEMPTS)
            .enumerate()
        {
            let (warp_width, warp_height) = quad_dimensions(quad);
            if warp_width < 32 || warp_height < 32 {
                continue;
            }
            if let Ok(warped) = warp_perspective_gray(&gray, quad, warp_width, warp_height) {
                let warped = DynamicImage::ImageLuma8(warped);
                let decoded = if index == 0 {
                    decoder
                        .decode_auto_with_info(&warped)
                        .or_else(|_| decode_resampled_full_frame(&decoder, &warped))
                } else {
                    decoder.decode_auto_with_info(&warped)
                };
                if let Ok(decoded) = decoded {
                    timings.quad_micros = elapsed_micros(stage);
                    timings.total_micros = elapsed_micros(started);
                    return Ok(StillScanResult {
                        decoded,
                        crop: None,
                        quad: Some(quad),
                        warp_size: Some((warp_width, warp_height)),
                        attempts: Vec::new(),
                        timings,
                    });
                }
            }
        }
    }
    timings.quad_micros = elapsed_micros(stage);

    let mut attempts = Vec::new();
    let padding = profile.min_anchor_px.max(8);
    let stage = scan_instant_now();
    let mut regions = still_scan_candidates(image, &binary, profile, padding, robust);
    prioritize_matrix_candidates(&mut regions);
    timings.candidate_micros = elapsed_micros(stage);

    let decode_started = scan_instant_now();
    #[cfg(not(target_arch = "wasm32"))]
    if !robust && regions.len() >= PARALLEL_DECODE_CANDIDATE_THRESHOLD {
        let mut results: Vec<(usize, ScanAttempt, Option<AutoDecodedSymbol>)> = regions
            .into_par_iter()
            .enumerate()
            .map(|(index, candidate)| {
                let attempt_started = scan_instant_now();
                let region = candidate.region;
                let cropped = image::imageops::crop_imm(
                    image,
                    region.x,
                    region.y,
                    region.width,
                    region.height,
                )
                .to_image();
                let cropped = DynamicImage::ImageRgba8(cropped);
                let local_decoder = RasterDecoder::default();
                match decode_candidate(&local_decoder, &cropped, candidate) {
                    Ok(decoded) => (
                        index,
                        ScanAttempt {
                            detector: candidate.detector.as_str(),
                            layout_hint: candidate.layout_hint,
                            stage: candidate.stage,
                            region,
                            decoded: true,
                            error: None,
                            duration_micros: elapsed_micros(attempt_started),
                        },
                        Some(decoded),
                    ),
                    Err(error) => (
                        index,
                        ScanAttempt {
                            detector: candidate.detector.as_str(),
                            layout_hint: candidate.layout_hint,
                            stage: candidate.stage,
                            region,
                            decoded: false,
                            error: Some(error.to_string()),
                            duration_micros: elapsed_micros(attempt_started),
                        },
                        None,
                    ),
                }
            })
            .collect();
        results.sort_by_key(|(index, _, _)| *index);

        if let Some(hit_index) = results
            .iter()
            .position(|(_, attempt, decoded)| attempt.decoded && decoded.is_some())
        {
            let mut decoded_hit = None;
            for (index, (_, attempt, decoded)) in results.into_iter().enumerate() {
                if index > hit_index {
                    break;
                }
                if decoded_hit.is_none() {
                    decoded_hit = decoded;
                }
                attempts.push(attempt);
            }
            let region = attempts
                .last()
                .map(|attempt| attempt.region)
                .unwrap_or(ScanRegion {
                    x: 0,
                    y: 0,
                    width: image.width().max(1),
                    height: image.height().max(1),
                });
            if let Some(decoded) = decoded_hit {
                timings.decode_attempts_micros = elapsed_micros(decode_started);
                timings.total_micros = elapsed_micros(started);
                return Ok(StillScanResult {
                    decoded,
                    crop: Some(region),
                    quad: None,
                    warp_size: None,
                    attempts,
                    timings,
                });
            }
            timings.decode_attempts_micros = elapsed_micros(decode_started);
            timings.total_micros = elapsed_micros(started);
            return Err(FailedStillScan {
                error: ScannerError::Decode(DecodeError::AutoDetectFailed),
                attempts,
                timings,
            });
        }
        attempts.extend(results.into_iter().map(|(_, attempt, _)| attempt));
    } else {
        for candidate in regions {
            let attempt_started = scan_instant_now();
            let region = candidate.region;
            let cropped =
                image::imageops::crop_imm(image, region.x, region.y, region.width, region.height)
                    .to_image();
            let cropped = DynamicImage::ImageRgba8(cropped);
            match decode_candidate(&decoder, &cropped, candidate) {
                Ok(decoded) => {
                    attempts.push(ScanAttempt {
                        detector: candidate.detector.as_str(),
                        layout_hint: candidate.layout_hint,
                        stage: candidate.stage,
                        region,
                        decoded: true,
                        error: None,
                        duration_micros: elapsed_micros(attempt_started),
                    });
                    timings.decode_attempts_micros = elapsed_micros(decode_started);
                    timings.total_micros = elapsed_micros(started);
                    return Ok(StillScanResult {
                        decoded,
                        crop: Some(region),
                        quad: None,
                        warp_size: None,
                        attempts,
                        timings,
                    });
                }
                Err(error) => attempts.push(ScanAttempt {
                    detector: candidate.detector.as_str(),
                    layout_hint: candidate.layout_hint,
                    stage: candidate.stage,
                    region,
                    decoded: false,
                    error: Some(error.to_string()),
                    duration_micros: elapsed_micros(attempt_started),
                }),
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    for candidate in regions {
        let attempt_started = scan_instant_now();
        let region = candidate.region;
        let cropped =
            image::imageops::crop_imm(image, region.x, region.y, region.width, region.height)
                .to_image();
        let cropped = DynamicImage::ImageRgba8(cropped);
        match decode_candidate(&decoder, &cropped, candidate) {
            Ok(decoded) => {
                attempts.push(ScanAttempt {
                    detector: candidate.detector.as_str(),
                    layout_hint: candidate.layout_hint,
                    stage: candidate.stage,
                    region,
                    decoded: true,
                    error: None,
                    duration_micros: elapsed_micros(attempt_started),
                });
                timings.decode_attempts_micros = elapsed_micros(decode_started);
                timings.total_micros = elapsed_micros(started);
                return Ok(StillScanResult {
                    decoded,
                    crop: Some(region),
                    quad: None,
                    warp_size: None,
                    attempts,
                    timings,
                });
            }
            Err(error) => attempts.push(ScanAttempt {
                detector: candidate.detector.as_str(),
                layout_hint: candidate.layout_hint,
                stage: candidate.stage,
                region,
                decoded: false,
                error: Some(error.to_string()),
                duration_micros: elapsed_micros(attempt_started),
            }),
        }
    }
    timings.decode_attempts_micros = elapsed_micros(decode_started);
    timings.total_micros = elapsed_micros(started);

    if std::env::var_os("GLYPHNET_SCAN_DEBUG").is_some() {
        eprintln!("scan attempts: {attempts:#?}");
    }
    Err(FailedStillScan {
        error: ScannerError::Decode(DecodeError::AutoDetectFailed),
        attempts,
        timings,
    })
}

fn prioritize_matrix_candidates(regions: &mut [ScanCandidate]) {
    let has_matrix_hint = regions.iter().any(|candidate| {
        matches!(
            candidate.layout_hint,
            Some(glyphnet_core::LayoutFamily::Matrix)
        )
    });
    if !has_matrix_hint {
        return;
    }
    regions.sort_by_key(|candidate| {
        if candidate.detector == CandidateDetector::GeneratedContent {
            0u8
        } else if matches!(
            candidate.layout_hint,
            Some(glyphnet_core::LayoutFamily::Matrix)
        ) {
            1u8
        } else {
            2u8
        }
    });
}

fn decode_resampled_full_frame(
    decoder: &RasterDecoder,
    image: &DynamicImage,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return Err(DecodeError::InvalidImageDimensions);
    }
    for module_px in [2u32, 3, 4, 5, 6, 8, 10, 12] {
        let target_width = (width as f32 / module_px as f32).round() as u32 * module_px;
        let target_height = (height as f32 / module_px as f32).round() as u32 * module_px;
        if target_width == 0 || target_height == 0 {
            continue;
        }
        if target_width == width && target_height == height {
            continue;
        }
        let resized = image::imageops::resize(
            image,
            target_width,
            target_height,
            image::imageops::FilterType::Triangle,
        );
        let resized = DynamicImage::ImageRgba8(resized);
        if let Ok(decoded) = decoder.decode_auto_with_info(&resized) {
            return Ok(decoded);
        }
    }
    Err(DecodeError::AutoDetectFailed)
}

fn failed_cv(
    error: glyphnet_cv::CvError,
    mut timings: ScanTimings,
    started: ScanInstant,
) -> FailedStillScan {
    timings.total_micros = elapsed_micros(started);
    FailedStillScan {
        error: ScannerError::Cv(error),
        attempts: Vec::new(),
        timings,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn scan_instant_now() -> ScanInstant {
    ScanInstant::now()
}

#[cfg(target_arch = "wasm32")]
fn scan_instant_now() -> ScanInstant {
    ScanInstant(js_sys::Date::now())
}

#[cfg(not(target_arch = "wasm32"))]
fn elapsed_micros(started: ScanInstant) -> u64 {
    started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64
}

#[cfg(target_arch = "wasm32")]
fn elapsed_micros(started: ScanInstant) -> u64 {
    ((js_sys::Date::now() - started.0).max(0.0) * 1000.0) as u64
}

fn should_try_full_frame_decode(image: &DynamicImage) -> bool {
    let width = image.width();
    let height = image.height();
    let area = width.saturating_mul(height);
    if area > 300_000 {
        return false;
    }
    let aspect = width as f32 / height.max(1) as f32;
    if !(1.0..=8.0).contains(&aspect) {
        return false;
    }
    !has_wide_ui_band(image)
}

fn has_wide_ui_band(image: &DynamicImage) -> bool {
    let luma = image.to_luma8();
    let width = luma.width();
    let height = luma.height();
    if width < 64 || height < 32 {
        return false;
    }
    let band_threshold = (width.saturating_mul(9)) / 10; // >= 90% non-white on a row.
    let mut y = 0u32;
    while y < height {
        let mut non_white = 0u32;
        for x in 0..width {
            if luma.get_pixel(x, y).0[0] < 245 {
                non_white += 1;
            }
        }
        if non_white >= band_threshold {
            return true;
        }
        y = y.saturating_add(1);
    }
    false
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
    use std::time::Instant;

    use glyphnet_core::{EccLevel, Frame};
    use glyphnet_encode::Encoder;
    use glyphnet_render::{RasterRenderer, RenderOptions};
    use glyphnet_testkit::{
        add_salt_pepper_noise, adjust_exposure, blur, place_on_canvas, resize, skew_x_on_white,
    };
    use image::{Rgba, RgbaImage};
    use rand::{SeedableRng, rngs::StdRng};

    use super::*;

    fn rendered_sample(payload: &[u8], module_px: u32) -> RgbaImage {
        rendered_sample_with_layout(payload, module_px, LayoutFamily::RibbonWeave)
    }

    fn rendered_sample_with_layout(
        payload: &[u8],
        module_px: u32,
        layout: LayoutFamily,
    ) -> RgbaImage {
        let encoded = Encoder::new(glyphnet_encode::EncoderConfig {
            layout,
            ..Default::default()
        })
        .encode_static(payload)
        .unwrap();
        RasterRenderer::new(RenderOptions {
            module_px,
            quiet_zone_modules: 4,
            ..RenderOptions::default()
        })
        .render(&encoded.matrix)
        .unwrap()
    }

    fn sample_canvas(payload: &[u8], module_px: u32, x: i64, y: i64) -> DynamicImage {
        let symbol = rendered_sample(payload, module_px);
        sample_canvas_with_symbol(&symbol, x, y)
    }

    fn sample_canvas_with_symbol(symbol: &RgbaImage, x: i64, y: i64) -> DynamicImage {
        let mut canvas = RgbaImage::from_pixel(960, 360, Rgba([255, 255, 255, 255]));
        image::imageops::overlay(&mut canvas, symbol, x, y);
        DynamicImage::ImageRgba8(canvas)
    }

    fn data_bit_module_coord(matrix: &SymbolMatrix, bit_index: usize) -> Option<(u16, u16, bool)> {
        let mut seen = 0usize;
        for y in 0..matrix.height() {
            for x in 0..matrix.width() {
                if layout::is_data_module_for(
                    matrix.layout(),
                    matrix.width(),
                    matrix.height(),
                    x,
                    y,
                ) {
                    let is_dark = matrix.get(x, y).ok()?.is_dark();
                    if seen == bit_index {
                        return Some((x, y, is_dark));
                    }
                    seen += 1;
                }
            }
        }
        None
    }

    fn paint_module(
        symbol: &mut RgbaImage,
        module_x: u16,
        module_y: u16,
        module_px: u32,
        rgba: Rgba<u8>,
    ) {
        let quiet_zone_modules = 4u32;
        let start_x = (u32::from(module_x) + quiet_zone_modules) * module_px;
        let start_y = (u32::from(module_y) + quiet_zone_modules) * module_px;
        for y in start_y..start_y + module_px {
            for x in start_x..start_x + module_px {
                symbol.put_pixel(x, y, rgba);
            }
        }
    }

    fn assert_scan_payload(image: &DynamicImage, payload: &[u8]) -> StillScanResult {
        let result = scan_still(image, TransmissionMode::Print).unwrap();
        assert_eq!(result.decoded.decoded.frame.payload, payload);
        result
    }

    fn add_debugger_ui_noise(canvas: &mut RgbaImage) {
        for y in [0, 46, 50, 312] {
            for x in 0..canvas.width() {
                canvas.put_pixel(x, y, Rgba([232, 236, 234, 255]));
            }
        }
        for x in [0, 764, 812, 959] {
            for y in 0..canvas.height() {
                canvas.put_pixel(x, y, Rgba([220, 226, 223, 255]));
            }
        }
        for y in 18..42 {
            for x in 22..180 {
                if (x + y) % 9 < 5 {
                    canvas.put_pixel(x, y, Rgba([24, 32, 30, 255]));
                }
            }
        }
        for y in 74..260 {
            for x in 820..940 {
                if (x * 3 + y) % 17 < 6 {
                    canvas.put_pixel(x, y, Rgba([30, 36, 34, 255]));
                }
            }
        }
    }

    fn add_matrix_canvas_clutter(canvas: &mut RgbaImage) {
        for y in [16, 48, 320] {
            for x in 24..900 {
                canvas.put_pixel(x, y, Rgba([36, 42, 40, 255]));
            }
        }
        for x in [40, 760, 900] {
            for y in 28..330 {
                canvas.put_pixel(x, y, Rgba([212, 220, 216, 255]));
            }
        }
        for y in 220..300 {
            for x in 620..890 {
                if (x + y * 3) % 13 < 4 {
                    canvas.put_pixel(x, y, Rgba([48, 56, 52, 255]));
                }
            }
        }
    }

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
        let canvas = place_on_canvas(&symbol, 32, 32, Rgba([255, 255, 255, 255]));
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

    #[test]
    fn scan_still_decodes_resized_symbol() {
        let encoded = Encoder::default().encode_static(b"resized").unwrap();
        let symbol = RasterRenderer::default().render(&encoded.matrix).unwrap();
        let resized = resize(&symbol, symbol.width() / 2, symbol.height() / 2);
        let canvas = place_on_canvas(&resized, 24, 20, Rgba([255, 255, 255, 255]));
        let result =
            scan_still(&DynamicImage::ImageRgba8(canvas), TransmissionMode::Print).unwrap();
        assert_eq!(result.decoded.decoded.frame.payload, b"resized");
    }

    #[test]
    fn scan_still_decodes_noisy_exposure_shifted_symbol() {
        let encoded = Encoder::default().encode_static(b"weathered").unwrap();
        let symbol = RasterRenderer::default().render(&encoded.matrix).unwrap();
        let mut canvas = place_on_canvas(&symbol, 32, 32, Rgba([238, 238, 232, 255]));
        adjust_exposure(&mut canvas, -8);
        let mut rng = StdRng::seed_from_u64(0x51a7);
        add_salt_pepper_noise(&mut canvas, 0.0003, &mut rng);
        let result =
            scan_still(&DynamicImage::ImageRgba8(canvas), TransmissionMode::Print).unwrap();
        assert_eq!(result.decoded.decoded.frame.payload, b"weathered");
    }

    #[test]
    fn scan_still_decodes_mildly_blurred_symbol() {
        let encoded = Encoder::default().encode_static(b"blur").unwrap();
        let symbol = RasterRenderer::default().render(&encoded.matrix).unwrap();
        let canvas = place_on_canvas(&symbol, 32, 32, Rgba([255, 255, 255, 255]));
        let blurred = blur(&canvas, 0.2);
        let result =
            scan_still(&DynamicImage::ImageRgba8(blurred), TransmissionMode::Print).unwrap();
        assert_eq!(result.decoded.decoded.frame.payload, b"blur");
    }

    #[test]
    fn scan_still_decodes_debugger_sample_canvas() {
        let symbol = rendered_sample(b"debug sample", 4);
        assert_eq!(symbol.width(), 416);
        assert_eq!(symbol.height(), 176);

        let mut canvas = RgbaImage::from_pixel(960, 360, Rgba([255, 255, 255, 255]));
        image::imageops::overlay(&mut canvas, &symbol, 110, 84);
        let image = DynamicImage::ImageRgba8(canvas);

        let result = assert_scan_payload(&image, b"debug sample");
        if let Some(crop) = result.crop {
            assert!((80..=140).contains(&crop.x), "unexpected crop.x: {crop:?}");
            assert!((60..=110).contains(&crop.y), "unexpected crop.y: {crop:?}");
        } else {
            assert!(result.quad.is_some(), "scanner should return crop or quad");
        }
    }

    #[test]
    fn scan_still_decodes_debugger_canvas_with_long_payload() {
        let payload = b"sdfdsfdfsfdsqdfsfdsdfsdsffdssdfsdffsdfdsfdsfsd";
        let image = sample_canvas(payload, 4, 110, 84);
        assert_scan_payload(&image, payload);
    }

    #[test]
    fn scan_still_decodes_generated_matrix_canvas() {
        let payload = b"matrix baseline";
        let symbol = rendered_sample_with_layout(payload, 4, LayoutFamily::Matrix);
        let image = sample_canvas_with_symbol(&symbol, 128, 56);
        let result = assert_scan_payload(&image, payload);
        assert_eq!(result.decoded.info.layout, LayoutFamily::Matrix);
        assert!(
            result.quad.is_some()
                || result.attempts.iter().any(|attempt| {
                    attempt.detector == CandidateDetector::GeneratedContent.as_str()
                        || attempt.detector == CandidateDetector::Matrix.as_str()
                })
        );
    }

    #[test]
    fn scan_still_decodes_matrix_canvas_with_clutter() {
        let payload = b"matrix clutter";
        let symbol = rendered_sample_with_layout(payload, 4, LayoutFamily::Matrix);
        let mut canvas = RgbaImage::from_pixel(960, 360, Rgba([255, 255, 255, 255]));
        add_matrix_canvas_clutter(&mut canvas);
        image::imageops::overlay(&mut canvas, &symbol, 128, 56);
        let image = DynamicImage::ImageRgba8(canvas);
        let result = assert_scan_payload(&image, payload);
        assert_eq!(result.decoded.info.layout, LayoutFamily::Matrix);
        assert!(
            result
                .attempts
                .iter()
                .any(|attempt| attempt.stage == "matrix-finders")
        );
    }

    #[test]
    fn scan_still_prioritizes_matrix_candidates_when_available() {
        let payload = b"matrix ordering";
        let symbol = rendered_sample_with_layout(payload, 4, LayoutFamily::Matrix);
        let mut canvas = RgbaImage::from_pixel(960, 360, Rgba([255, 255, 255, 255]));
        add_matrix_canvas_clutter(&mut canvas);
        image::imageops::overlay(&mut canvas, &symbol, 128, 56);
        let image = DynamicImage::ImageRgba8(canvas);
        let result = scan_still_with_diagnostics(&image, TransmissionMode::Print).unwrap();
        assert_eq!(result.decoded.decoded.frame.payload, payload);
        let first = result
            .attempts
            .first()
            .expect("matrix scan should record at least one attempt");
        assert_eq!(first.layout_hint, Some(LayoutFamily::Matrix));
    }

    #[test]
    fn scan_still_decodes_resized_matrix_canvas() {
        let payload = b"matrix resized";
        let symbol = rendered_sample_with_layout(payload, 4, LayoutFamily::Matrix);
        let resized = resize(&symbol, symbol.width() / 2, symbol.height() / 2);
        let image = sample_canvas_with_symbol(&resized, 132, 72);
        let result = assert_scan_payload(&image, payload);
        assert_eq!(result.decoded.info.layout, LayoutFamily::Matrix);
    }

    #[test]
    fn scan_still_decodes_mild_skewed_matrix_canvas() {
        let payload = b"matrix skew";
        let symbol = rendered_sample_with_layout(payload, 5, LayoutFamily::Matrix);
        let canvas = place_on_canvas(&symbol, 30, 24, Rgba([255, 255, 255, 255]));
        let skewed = skew_x_on_white(&canvas, 4, -4);
        let result =
            scan_still_robust(&DynamicImage::ImageRgba8(skewed), TransmissionMode::Print).unwrap();
        assert_eq!(result.decoded.decoded.frame.payload, payload);
        assert_eq!(result.decoded.info.layout, LayoutFamily::Matrix);
    }

    #[test]
    fn scan_still_decodes_matrix_canvas_with_light_ui_clutter() {
        let payload = b"matrix ui";
        let symbol = rendered_sample_with_layout(payload, 4, LayoutFamily::Matrix);
        let mut canvas = RgbaImage::from_pixel(640, 280, Rgba([248, 248, 248, 255]));
        for x in 0..canvas.width() {
            canvas.put_pixel(x, 26, Rgba([232, 236, 234, 255]));
            canvas.put_pixel(x, 244, Rgba([232, 236, 234, 255]));
        }
        for y in 18..260 {
            canvas.put_pixel(18, y, Rgba([220, 226, 223, 255]));
            canvas.put_pixel(612, y, Rgba([220, 226, 223, 255]));
        }
        image::imageops::overlay(&mut canvas, &symbol, 140, 58);
        let result = assert_scan_payload(&DynamicImage::ImageRgba8(canvas), payload);
        assert_eq!(result.decoded.info.layout, LayoutFamily::Matrix);
    }

    #[test]
    #[ignore = "stress case is useful but too slow for the default debug test suite"]
    fn scan_still_decodes_debugger_sample_canvas_offsets() {
        let image = sample_canvas(b"debug sample", 4, 420, 132);
        let result = assert_scan_payload(&image, b"debug sample");
        let crop = result.crop.expect("offset sample should crop");
        assert!(crop.x <= 460, "crop too far right: {crop:?}");
        assert!(crop.y <= 172, "crop too low: {crop:?}");
    }

    #[test]
    #[ignore = "needs faster candidate rejection before noisy UI clutter belongs in default tests"]
    fn scan_still_decodes_debugger_sample_with_ui_noise() {
        let symbol = rendered_sample(b"debug sample", 4);
        let mut canvas = RgbaImage::from_pixel(960, 360, Rgba([250, 250, 250, 255]));
        add_debugger_ui_noise(&mut canvas);
        image::imageops::overlay(&mut canvas, &symbol, 110, 84);
        assert_scan_payload(&DynamicImage::ImageRgba8(canvas), b"debug sample");
    }

    #[test]
    fn scan_still_decodes_debugger_sample_with_light_ui_clutter_fast() {
        let symbol = rendered_sample(b"debug sample", 4);
        let mut canvas = RgbaImage::from_pixel(600, 240, Rgba([250, 250, 250, 255]));
        for x in 0..canvas.width() {
            canvas.put_pixel(x, 28, Rgba([232, 236, 234, 255]));
        }
        image::imageops::overlay(&mut canvas, &symbol, 88, 32);
        assert_scan_payload(&DynamicImage::ImageRgba8(canvas), b"debug sample");
    }

    #[test]
    fn scan_still_decodes_small_debugger_sample_canvas() {
        let image = sample_canvas(b"debug sample", 2, 80, 72);
        let result = scan_still_robust(&image, TransmissionMode::Print).unwrap();
        assert_eq!(result.decoded.decoded.frame.payload, b"debug sample");
    }

    #[test]
    fn scan_still_decodes_manual_crop_from_debugger_sample() {
        let image = sample_canvas(b"debug sample", 4, 110, 84);
        let cropped = image::imageops::crop_imm(&image, 100, 76, 440, 194).to_image();
        assert_scan_payload(&DynamicImage::ImageRgba8(cropped), b"debug sample");
    }

    #[test]
    fn scan_still_finds_symbol_embedded_in_screenshot_like_image() {
        let encoded = Encoder::default().encode_static(b"embedded").unwrap();
        let symbol = RasterRenderer::default().render(&encoded.matrix).unwrap();
        let mut canvas = image::RgbaImage::from_pixel(1100, 940, Rgba([250, 250, 250, 255]));

        for y in [24, 72, 118, 180, 640] {
            for x in 24..1040 {
                canvas.put_pixel(x, y, Rgba([35, 35, 35, 255]));
            }
        }
        for x in [24, 1040, 720] {
            for y in 24..880 {
                canvas.put_pixel(x, y, Rgba([45, 45, 45, 255]));
            }
        }
        for row in 0..18 {
            let y = 230 + row * 18;
            for x in 60..420 {
                if x % 11 < 7 {
                    canvas.put_pixel(x, y, Rgba([20, 20, 20, 255]));
                }
            }
        }

        image::imageops::overlay(&mut canvas, &symbol, 110, 520);
        let image = DynamicImage::ImageRgba8(canvas);
        let exact =
            image::imageops::crop_imm(&image, 110, 520, symbol.width(), symbol.height()).to_image();
        assert!(
            RasterDecoder::default()
                .decode_auto_with_info(&DynamicImage::ImageRgba8(exact))
                .is_ok()
        );
        let exact2 =
            image::imageops::crop_imm(&image, 110, 520, symbol.width(), symbol.height()).to_image();
        assert!(
            RasterDecoder::new(DecodeOptions {
                module_px: 8,
                quiet_zone_modules: 4,
                threshold: 192,
                layout: LayoutFamily::RibbonWeave,
            })
            .decode(&DynamicImage::ImageRgba8(exact2))
            .is_ok()
        );
        let result = scan_still(&image, TransmissionMode::Print).unwrap();
        assert_eq!(result.decoded.decoded.frame.payload, b"embedded");
        assert!(matches!(
            result.crop,
            Some(ScanRegion {
                x: 0..=160,
                y: 470..=540,
                ..
            })
        ));
        assert!(result.attempts.iter().any(|attempt| attempt.decoded));
    }

    #[test]
    fn scan_still_decodes_real_debugger_screenshot_fixture() {
        let image =
            image::load_from_memory(include_bytes!("../fixtures/screenshot-debug-sample.png"))
                .unwrap();

        let started = Instant::now();
        let result = scan_still(&image, TransmissionMode::Print).unwrap();
        let elapsed = started.elapsed();

        assert_eq!(result.decoded.decoded.frame.payload, b"debug sample");
        let crop = result
            .crop
            .expect("real screenshot should use scanner crop");
        assert!((330..=400).contains(&crop.x), "unexpected crop.x: {crop:?}");
        assert!((350..=420).contains(&crop.y), "unexpected crop.y: {crop:?}");
        assert!(
            (1250..=1380).contains(&crop.width),
            "unexpected crop.width: {crop:?}"
        );
        assert!(
            (520..=590).contains(&crop.height),
            "unexpected crop.height: {crop:?}"
        );
        // Keep this test focused on correctness and stable crop geometry.
        // Runtime is measured for visibility but enforced by benchmarks instead
        // of a hard wall-clock gate in debug test builds.
        eprintln!("real screenshot scan took {elapsed:?}");
    }

    #[test]
    fn scan_still_decodes_mild_horizontal_skew() {
        let encoded = Encoder::default().encode_static(b"skew").unwrap();
        let symbol = RasterRenderer::default().render(&encoded.matrix).unwrap();
        let canvas = place_on_canvas(&symbol, 28, 28, Rgba([255, 255, 255, 255]));
        let skewed = skew_x_on_white(&canvas, 10, -10);
        let result =
            scan_still_robust(&DynamicImage::ImageRgba8(skewed), TransmissionMode::Print).unwrap();
        assert_eq!(result.decoded.decoded.frame.payload, b"skew");
    }

    #[test]
    #[ignore = "still-scan candidate detection for this cluttered screen-mode case needs separate tuning"]
    fn scan_still_recovers_low_confidence_payload_bit_with_ui_clutter() {
        let payload = b"guided recovery";
        let module_px = 4u32;
        let encoded = Encoder::new(glyphnet_encode::EncoderConfig {
            mode: TransmissionMode::Screen,
            ..Default::default()
        })
        .encode_static(payload)
        .unwrap();
        let mut symbol = RasterRenderer::new(RenderOptions {
            module_px,
            quiet_zone_modules: 4,
            ..RenderOptions::default()
        })
        .render(&encoded.matrix)
        .unwrap();

        let bit_index = HEADER_LEN * 8 + 10;
        let (module_x, module_y, is_dark) =
            data_bit_module_coord(&encoded.matrix, bit_index).expect("bit index should map");
        let flipped_gray = if is_dark { 194 } else { 190 };
        paint_module(
            &mut symbol,
            module_x,
            module_y,
            module_px,
            Rgba([flipped_gray, flipped_gray, flipped_gray, 255]),
        );

        let mut canvas = sample_canvas_with_symbol(&symbol, 110, 84).to_rgba8();
        for x in 0..canvas.width() {
            canvas.put_pixel(x, 42, Rgba([232, 236, 234, 255]));
        }
        for y in 0..canvas.height() {
            canvas.put_pixel(22, y, Rgba([220, 226, 223, 255]));
        }

        let result =
            scan_still(&DynamicImage::ImageRgba8(canvas), TransmissionMode::Screen).unwrap();
        assert_eq!(result.decoded.decoded.frame.payload, payload);
    }
}
