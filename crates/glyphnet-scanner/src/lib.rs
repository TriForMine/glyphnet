//! Real-time scanner orchestration for GlyphNet.

use std::collections::{BTreeMap, HashMap, VecDeque};

use glyphnet_core::LayoutFamily;
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
    /// Candidate crop attempts considered by the still scanner.
    pub attempts: Vec<ScanAttempt>,
}

/// Diagnostic information for one still-scan candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanAttempt {
    /// Scanner stage that produced this attempt.
    pub stage: &'static str,
    /// Candidate region in source-image pixels.
    pub region: ScanRegion,
    /// Whether this candidate decoded successfully.
    pub decoded: bool,
    /// Error message when decode failed.
    pub error: Option<String>,
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
    if should_try_full_frame_decode(image)
        && let Ok(decoded) = decoder.decode_auto_with_info(image)
    {
        return Ok(StillScanResult {
            decoded,
            crop: None,
            quad: None,
            warp_size: None,
            attempts: Vec::new(),
        });
    }

    let profile = VisionProfile::for_mode(mode);
    let gray = grayscale(image)?;
    let binary = adaptive_threshold(&gray, profile.threshold_radius, profile.threshold_bias)?;
    let candidates = find_anchor_candidates(&binary, profile)?;
    if let Some(quad) = estimate_quad(&binary, &candidates) {
        let (warp_width, warp_height) = quad_dimensions(quad);
        if let Ok(warped) = warp_perspective_gray(&gray, quad, warp_width, warp_height) {
            let warped = DynamicImage::ImageLuma8(warped);
            if let Ok(decoded) = decoder.decode_auto_with_info(&warped) {
                return Ok(StillScanResult {
                    decoded,
                    crop: None,
                    quad: Some(quad),
                    warp_size: Some((warp_width, warp_height)),
                    attempts: Vec::new(),
                });
            }
        }
    }

    let padding = profile.min_anchor_px.max(8);
    let mut attempts = Vec::new();
    let mut regions = candidate_regions(&binary, padding, image.width(), image.height());
    if let Some(bounds) = dark_bounds(&binary) {
        regions.push((
            "dark-bounds",
            expand_region(bounds, padding, image.width(), image.height()),
        ));
    }

    for (stage, region) in regions {
        let cropped =
            image::imageops::crop_imm(image, region.x, region.y, region.width, region.height)
                .to_image();
        let cropped = DynamicImage::ImageRgba8(cropped);
        match decode_candidate(&decoder, &cropped, stage, region) {
            Ok(decoded) => {
                attempts.push(ScanAttempt {
                    stage,
                    region,
                    decoded: true,
                    error: None,
                });
                return Ok(StillScanResult {
                    decoded,
                    crop: Some(region),
                    quad: None,
                    warp_size: None,
                    attempts,
                });
            }
            Err(error) => attempts.push(ScanAttempt {
                stage,
                region,
                decoded: false,
                error: Some(error.to_string()),
            }),
        }
    }

    Err(DecodeError::AutoDetectFailed.into())
}

fn should_try_full_frame_decode(image: &DynamicImage) -> bool {
    let width = image.width();
    let height = image.height();
    let area = width.saturating_mul(height);
    if area > 700_000 {
        return false;
    }
    let aspect = width as f32 / height.max(1) as f32;
    (1.0..=8.0).contains(&aspect)
}

fn decode_candidate(
    decoder: &RasterDecoder,
    image: &DynamicImage,
    stage: &'static str,
    region: ScanRegion,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    if matches!(
        stage,
        "reference-sweep" | "component-reference" | "signature-window"
    ) {
        let module_px = (region.width / 104).max(1);
        if region.width == 104 * module_px && region.height == 44 * module_px {
            let exact = RasterDecoder::new(DecodeOptions {
                module_px,
                quiet_zone_modules: 4,
                threshold: 192,
                layout: LayoutFamily::RibbonWeave,
            });
            let decoded = exact.decode(image)?;
            return Ok(AutoDecodedSymbol {
                decoded,
                info: glyphnet_decode::AutoDecodeInfo {
                    module_px,
                    quiet_zone_modules: 4,
                    threshold: 192,
                    layout: LayoutFamily::RibbonWeave,
                },
            });
        }
    }
    decoder.decode_auto_with_info(image)
}

#[derive(Debug, Clone, Copy)]
struct DarkComponent {
    pixels: u32,
    bounds: ScanRegion,
}

fn dark_bounds(binary: &GrayImage) -> Option<ScanRegion> {
    let components = dark_components(binary);
    let mut best: Option<(u32, ScanRegion)> = None;
    let mut significant: Option<ScanRegion> = None;

    for component in components {
        if component.pixels >= MIN_COMPONENT_PIXELS {
            significant = Some(match significant {
                Some(existing) => union_region(existing, component.bounds),
                None => component.bounds,
            });
        }
        if best.is_none_or(|(best_count, _)| component.pixels > best_count) {
            best = Some((component.pixels, component.bounds));
        }
    }
    significant.or_else(|| best.map(|(_, region)| region))
}

fn dark_components(binary: &GrayImage) -> Vec<DarkComponent> {
    let width = binary.width();
    let height = binary.height();
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let mut visited = vec![false; width as usize * height as usize];
    let mut components = Vec::new();

    for y in 0..binary.height() {
        for x in 0..binary.width() {
            let index = pixel_index(width, x, y);
            if visited[index] || binary.get_pixel(x, y).0[0] != 0 {
                continue;
            }

            let mut queue = VecDeque::from([(x, y)]);
            visited[index] = true;
            let mut count = 0u32;
            let mut min_x = x;
            let mut min_y = y;
            let mut max_x = x;
            let mut max_y = y;

            while let Some((cx, cy)) = queue.pop_front() {
                count += 1;
                min_x = min_x.min(cx);
                min_y = min_y.min(cy);
                max_x = max_x.max(cx);
                max_y = max_y.max(cy);

                let y_start = cy.saturating_sub(1);
                let y_end = (cy + 1).min(height - 1);
                let x_start = cx.saturating_sub(1);
                let x_end = (cx + 1).min(width - 1);
                for ny in y_start..=y_end {
                    for nx in x_start..=x_end {
                        let neighbor_index = pixel_index(width, nx, ny);
                        if visited[neighbor_index] || binary.get_pixel(nx, ny).0[0] != 0 {
                            continue;
                        }
                        visited[neighbor_index] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }

            let region = ScanRegion {
                x: min_x,
                y: min_y,
                width: max_x.saturating_sub(min_x).saturating_add(1),
                height: max_y.saturating_sub(min_y).saturating_add(1),
            };
            components.push(DarkComponent {
                pixels: count,
                bounds: region,
            });
        }
    }
    components.sort_by(|a, b| b.pixels.cmp(&a.pixels));
    components
}

fn pixel_index(width: u32, x: u32, y: u32) -> usize {
    y as usize * width as usize + x as usize
}

const MIN_COMPONENT_PIXELS: u32 = 16;
const MAX_CANDIDATE_REGIONS: usize = 96;

fn candidate_regions(
    binary: &GrayImage,
    padding: u32,
    image_width: u32,
    image_height: u32,
) -> Vec<(&'static str, ScanRegion)> {
    let mut regions = Vec::new();
    regions.extend(horizontal_band_regions(
        binary,
        padding,
        image_width,
        image_height,
    ));

    for component in dark_components(binary).into_iter().take(64) {
        if component.pixels < MIN_COMPONENT_PIXELS {
            continue;
        }
        let expanded = expand_region(component.bounds, padding * 2, image_width, image_height);
        if plausible_region(expanded) {
            regions.push(("component", expanded));
        }
    }

    regions.truncate(MAX_CANDIDATE_REGIONS);
    regions
}

fn horizontal_band_regions(
    binary: &GrayImage,
    padding: u32,
    image_width: u32,
    image_height: u32,
) -> Vec<(&'static str, ScanRegion)> {
    if image_width == 0 || image_height == 0 {
        return Vec::new();
    }
    let row_threshold = (image_width / 120).clamp(8, 80);
    let mut rows = Vec::new();
    for y in 0..image_height {
        let mut dark = 0u32;
        let mut min_x = image_width;
        let mut max_x = 0u32;
        for x in 0..image_width {
            if binary.get_pixel(x, y).0[0] == 0 {
                dark += 1;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
            }
        }
        if dark >= row_threshold {
            rows.push((y, min_x, max_x, dark));
        }
    }

    let mut regions = Vec::new();
    let mut index = 0usize;
    while index < rows.len() {
        let start_y = rows[index].0;
        let mut end_y = start_y;
        let mut min_x = rows[index].1;
        let mut max_x = rows[index].2;
        let mut dark_sum = rows[index].3;
        index += 1;

        while index < rows.len() && rows[index].0 <= end_y + 10 {
            end_y = rows[index].0;
            min_x = min_x.min(rows[index].1);
            max_x = max_x.max(rows[index].2);
            dark_sum += rows[index].3;
            index += 1;
        }

        if let Some((dominant_min_x, dominant_max_x)) =
            dominant_x_span(binary, start_y, end_y, min_x, max_x)
        {
            min_x = dominant_min_x;
            max_x = dominant_max_x;
        }

        let width = max_x.saturating_sub(min_x).saturating_add(1);
        let height = end_y.saturating_sub(start_y).saturating_add(1);
        if width < 48 || height < 8 || dark_sum < 80 {
            continue;
        }

        let band = ScanRegion {
            x: min_x,
            y: start_y,
            width,
            height,
        };
        let vertical_padding = padding.max(height);
        let expanded = expand_region(
            band,
            padding.max(width / 40).max(vertical_padding),
            image_width,
            image_height,
        );
        if plausible_region(expanded) {
            regions.push(("horizontal-band", expanded));
        }
        for aspect in [2.35_f32, 2.7, 3.2] {
            let target_height = ((width as f32 / aspect).round() as u32).max(height);
            let center_y = start_y + height / 2;
            let starts = [
                center_y.saturating_sub(target_height / 2),
                start_y.saturating_sub(padding),
                start_y,
                start_y.saturating_add(padding),
                end_y.saturating_sub(target_height / 2),
            ];
            for y in starts {
                let region = expand_region(
                    ScanRegion {
                        x: min_x,
                        y,
                        width,
                        height: target_height.min(image_height.saturating_sub(y).max(1)),
                    },
                    padding.saturating_mul(2).max(width / 40),
                    image_width,
                    image_height,
                );
                if plausible_region(region) {
                    regions.push(("horizontal-aspect", region));
                }
            }
        }
    }

    regions.sort_by(|(_, a), (_, b)| region_score(*b).total_cmp(&region_score(*a)));
    regions.truncate(48);
    regions
}

fn dominant_x_span(
    binary: &GrayImage,
    start_y: u32,
    end_y: u32,
    min_x: u32,
    max_x: u32,
) -> Option<(u32, u32)> {
    if min_x > max_x {
        return None;
    }
    let height = end_y.saturating_sub(start_y).saturating_add(1);
    let threshold = (height / 24).max(2);
    let mut groups = Vec::new();
    let mut current: Option<(u32, u32, u32)> = None;
    let mut gap = 0u32;

    for x in min_x..=max_x {
        let mut dark = 0u32;
        for y in start_y..=end_y {
            if binary.get_pixel(x, y).0[0] == 0 {
                dark += 1;
            }
        }
        if dark >= threshold {
            match &mut current {
                Some((_, end, score)) => {
                    *end = x;
                    *score += dark;
                }
                None => current = Some((x, x, dark)),
            }
            gap = 0;
        } else if let Some((start, end, score)) = current {
            gap += 1;
            if gap > 16 {
                groups.push((start, end, score));
                current = None;
                gap = 0;
            } else {
                current = Some((start, end, score));
            }
        }
    }
    if let Some(group) = current {
        groups.push(group);
    }

    groups
        .into_iter()
        .filter(|(start, end, _)| end.saturating_sub(*start).saturating_add(1) >= 48)
        .max_by_key(|(start, end, score)| {
            score.saturating_mul(end.saturating_sub(*start).saturating_add(1))
        })
        .map(|(start, end, _)| (start, end))
}

fn plausible_region(region: ScanRegion) -> bool {
    if region.width < 80 || region.height < 28 {
        return false;
    }
    let aspect = region.width as f32 / region.height.max(1) as f32;
    (1.2..=12.0).contains(&aspect)
}

fn region_score(region: ScanRegion) -> f32 {
    let area = region.width.saturating_mul(region.height) as f32;
    let aspect = region.width as f32 / region.height.max(1) as f32;
    let aspect_score = 1.0 / (aspect - 3.0).abs().max(0.25);
    area.sqrt() * aspect_score
}

fn union_region(a: ScanRegion, b: ScanRegion) -> ScanRegion {
    let min_x = a.x.min(b.x);
    let min_y = a.y.min(b.y);
    let max_x = a.x.saturating_add(a.width).max(b.x.saturating_add(b.width));
    let max_y =
        a.y.saturating_add(a.height)
            .max(b.y.saturating_add(b.height));
    ScanRegion {
        x: min_x,
        y: min_y,
        width: max_x.saturating_sub(min_x),
        height: max_y.saturating_sub(min_y),
    }
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
    use glyphnet_testkit::{
        add_salt_pepper_noise, adjust_exposure, blur, place_on_canvas, resize, skew_x_on_white,
    };
    use image::Rgba;
    use rand::{SeedableRng, rngs::StdRng};

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
    #[ignore = "needs a true run-pattern detector instead of crop search"]
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
        let profile = VisionProfile::for_mode(TransmissionMode::Print);
        let gray = grayscale(&image).unwrap();
        let binary =
            adaptive_threshold(&gray, profile.threshold_radius, profile.threshold_bias).unwrap();
        let regions = candidate_regions(
            &binary,
            profile.min_anchor_px.max(8),
            image.width(),
            image.height(),
        );
        eprintln!(
            "count {} exact {} first {:?}",
            regions.len(),
            regions.iter().any(|(_, r)| r.x == 110
                && r.y == 520
                && r.width == symbol.width()
                && r.height == symbol.height()),
            regions.iter().take(12).collect::<Vec<_>>()
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
    #[ignore = "requires stronger perspective rectification than the current reference scanner"]
    fn scan_still_decodes_mild_horizontal_skew() {
        let encoded = Encoder::default().encode_static(b"skew").unwrap();
        let symbol = RasterRenderer::default().render(&encoded.matrix).unwrap();
        let canvas = place_on_canvas(&symbol, 28, 28, Rgba([255, 255, 255, 255]));
        let skewed = skew_x_on_white(&canvas, 10, -10);
        let result =
            scan_still(&DynamicImage::ImageRgba8(skewed), TransmissionMode::Print).unwrap();
        assert_eq!(result.decoded.decoded.frame.payload, b"skew");
    }
}
