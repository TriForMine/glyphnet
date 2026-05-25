//! Real-time scanner orchestration for GlyphNet.

use std::collections::{BTreeMap, HashMap, VecDeque};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant as ScanInstant;

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone, Copy)]
struct ScanInstant(f64);

use glyphnet_core::LayoutFamily;
use glyphnet_core::{
    Cell, Frame, FrameHeader, HEADER_LEN, SymbolMatrix, TransmissionMode, bitstream, layout,
};
use glyphnet_cv::{
    VisionProfile, adaptive_threshold, estimate_quad, find_anchor_candidates, grayscale,
    quad_dimensions, warp_perspective_gray,
};
use glyphnet_decode::{
    AutoDecodedSymbol, DecodeError, DecodeOptions, RasterDecoder, decode_matrix,
};
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
    /// Scanner stage timing diagnostics.
    pub timings: ScanTimings,
}

/// Diagnostic information for one still-scan candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanAttempt {
    /// Detector family that produced this attempt.
    pub detector: &'static str,
    /// Layout expected by the detector, when known.
    pub layout_hint: Option<LayoutFamily>,
    /// Scanner stage that produced this attempt.
    pub stage: &'static str,
    /// Candidate region in source-image pixels.
    pub region: ScanRegion,
    /// Whether this candidate decoded successfully.
    pub decoded: bool,
    /// Error message when decode failed.
    pub error: Option<String>,
    /// Candidate decode duration in microseconds.
    pub duration_micros: u64,
}

/// Timing diagnostics for one still scan.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanTimings {
    /// Complete still-scan duration.
    pub total_micros: u64,
    /// Full-frame decode attempt duration.
    pub full_frame_micros: u64,
    /// Grayscale conversion duration.
    pub grayscale_micros: u64,
    /// Adaptive threshold duration.
    pub threshold_micros: u64,
    /// Anchor and quad estimation duration.
    pub quad_micros: u64,
    /// Candidate region generation duration.
    pub candidate_micros: u64,
    /// Candidate crop/decode loop duration.
    pub decode_attempts_micros: u64,
}

/// Failed still-scan diagnostics.
#[derive(Debug)]
pub struct FailedStillScan {
    /// User-facing decode error.
    pub error: ScannerError,
    /// Candidate crop attempts considered by the still scanner.
    pub attempts: Vec<ScanAttempt>,
    /// Scanner stage timing diagnostics.
    pub timings: ScanTimings,
}

/// Candidate detector family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateDetector {
    /// Clean rendered symbol on a simple background.
    GeneratedContent,
    /// Layout-agnostic dark-component and band detector.
    GenericBinary,
    /// RibbonWeave rail, totem, and wide-symbol recovery detector.
    RibbonWeave,
}

impl CandidateDetector {
    const fn as_str(self) -> &'static str {
        match self {
            Self::GeneratedContent => "generated-content",
            Self::GenericBinary => "generic-binary",
            Self::RibbonWeave => "ribbon-weave",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScanCandidate {
    detector: CandidateDetector,
    layout_hint: Option<LayoutFamily>,
    stage: &'static str,
    region: ScanRegion,
}

impl ScanCandidate {
    const fn new(
        detector: CandidateDetector,
        layout_hint: Option<LayoutFamily>,
        stage: &'static str,
        region: ScanRegion,
    ) -> Self {
        Self {
            detector,
            layout_hint,
            stage,
            region,
        }
    }
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
    scan_still_with_diagnostics(image, mode).map_err(|failed| failed.error)
}

/// Scan a still image and return failed-attempt diagnostics on decode failure.
pub fn scan_still_with_diagnostics(
    image: &DynamicImage,
    mode: TransmissionMode,
) -> std::result::Result<StillScanResult, FailedStillScan> {
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
        if let Ok(decoded) = decode_resampled_full_frame(&decoder, image) {
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

    let stage = scan_instant_now();
    let candidates = find_anchor_candidates(&binary, profile)
        .map_err(|error| failed_cv(error, timings, started))?;
    if let Some(quad) = estimate_quad(&binary, &candidates) {
        let (warp_width, warp_height) = quad_dimensions(quad);
        if let Ok(warped) = warp_perspective_gray(&gray, quad, warp_width, warp_height) {
            let warped = DynamicImage::ImageLuma8(warped);
            if let Ok(decoded) = decoder.decode_auto_with_info(&warped) {
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
    timings.quad_micros = elapsed_micros(stage);

    let padding = profile.min_anchor_px.max(8);
    let mut attempts = Vec::new();
    let stage = scan_instant_now();
    let regions = still_scan_candidates(image, &binary, profile, padding);
    timings.candidate_micros = elapsed_micros(stage);

    let decode_started = scan_instant_now();
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

fn should_try_dark_bounds_fallback(
    image_width: u32,
    image_height: u32,
    candidate_count: usize,
) -> bool {
    image_width.saturating_mul(image_height) <= 900_000 || candidate_count == 0
}

fn still_scan_candidates(
    image: &DynamicImage,
    binary: &GrayImage,
    profile: VisionProfile,
    padding: u32,
) -> Vec<ScanCandidate> {
    let image_width = image.width();
    let image_height = image.height();
    let mut candidates = Vec::new();

    if let Some(bounds) = content_bounds(image) {
        candidates.extend(content_symbol_regions(
            bounds,
            image_width,
            image_height,
            profile.min_anchor_px,
        ));
    }

    candidates.extend(ribbon_weave_candidates(binary, image_width, image_height));

    if image_width.saturating_mul(image_height) <= 900_000 {
        candidates.extend(generic_binary_candidates(
            binary,
            padding,
            image_width,
            image_height,
        ));
    }

    if should_try_dark_bounds_fallback(image_width, image_height, candidates.len())
        && let Some(bounds) = dark_bounds(binary)
    {
        candidates.extend(ribbon_dark_bounds_candidates(
            bounds,
            padding,
            image_width,
            image_height,
        ));
    }

    candidates.truncate(MAX_CANDIDATE_REGIONS);
    candidates
}

fn content_bounds(image: &DynamicImage) -> Option<ScanRegion> {
    let rgba = image.to_rgba8();
    let mut min_x = image.width();
    let mut min_y = image.height();
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;

    for (x, y, pixel) in rgba.enumerate_pixels() {
        let [red, green, blue, alpha] = pixel.0;
        if alpha > 0 && (red < 245 || green < 245 || blue < 245) {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            found = true;
        }
    }

    found.then_some(ScanRegion {
        x: min_x,
        y: min_y,
        width: max_x.saturating_sub(min_x).saturating_add(1),
        height: max_y.saturating_sub(min_y).saturating_add(1),
    })
}

fn content_symbol_regions(
    bounds: ScanRegion,
    image_width: u32,
    image_height: u32,
    min_module_px: u32,
) -> Vec<ScanCandidate> {
    let mut regions = Vec::new();
    for module_px in 1..=16 {
        if module_px < (min_module_px / 8).max(1) {
            continue;
        }
        if bounds.width % module_px != 0 || bounds.height % module_px != 0 {
            continue;
        }
        let symbol_width = bounds.width / module_px;
        let symbol_height = bounds.height / module_px;
        if !reference_generated_geometry(symbol_width, symbol_height) {
            continue;
        }
        let quiet_px = module_px.saturating_mul(4);
        let region = ScanRegion {
            x: bounds.x.saturating_sub(quiet_px),
            y: bounds.y.saturating_sub(quiet_px),
            width: bounds.width.saturating_add(quiet_px.saturating_mul(2)),
            height: bounds.height.saturating_add(quiet_px.saturating_mul(2)),
        };
        regions.push(ScanCandidate::new(
            CandidateDetector::GeneratedContent,
            None,
            "content-bounds",
            clamp_region(region, image_width, image_height),
        ));
    }
    for module_px in 1..=16 {
        if module_px < (min_module_px / 8).max(1) {
            continue;
        }
        let expected_content_width = 96 * module_px;
        if bounds.width.abs_diff(expected_content_width) > module_px {
            continue;
        }
        let quiet_px = module_px.saturating_mul(4);
        for extra_top_px in [0, 1, module_px / 2, module_px] {
            let region = ScanRegion {
                x: bounds.x.saturating_sub(quiet_px),
                y: bounds
                    .y
                    .saturating_sub(quiet_px.saturating_add(extra_top_px)),
                width: 104 * module_px,
                height: 44 * module_px,
            };
            regions.push(ScanCandidate::new(
                CandidateDetector::GeneratedContent,
                Some(LayoutFamily::RibbonWeave),
                "content-reference",
                clamp_region(region, image_width, image_height),
            ));
        }
    }
    for module_px in 1..=16 {
        if module_px < (min_module_px / 8).max(1) {
            continue;
        }
        if bounds.width % module_px != 0 || bounds.height % module_px != 0 {
            continue;
        }
        let symbol_width = bounds.width / module_px;
        let symbol_height = bounds.height / module_px;
        if !reference_ribbon_geometry(symbol_width, symbol_height) {
            continue;
        }
        let quiet_px = module_px.saturating_mul(4);
        let region = ScanRegion {
            x: bounds.x.saturating_sub(quiet_px),
            y: bounds.y.saturating_sub(quiet_px),
            width: bounds.width.saturating_add(quiet_px.saturating_mul(2)),
            height: bounds.height.saturating_add(quiet_px.saturating_mul(2)),
        };
        regions.push(ScanCandidate::new(
            CandidateDetector::GeneratedContent,
            Some(LayoutFamily::RibbonWeave),
            "content-bounds",
            clamp_region(region, image_width, image_height),
        ));
    }
    regions
}

fn reference_generated_geometry(width: u32, height: u32) -> bool {
    if width == height && width >= 29 {
        return true;
    }
    let aspect = width as f32 / height.max(1) as f32;
    width >= 48
        && height >= 28
        && width % 2 == 0
        && height % 2 == 0
        && (1.0..=8.0).contains(&aspect)
}

fn ribbon_dark_bounds_candidates(
    bounds: ScanRegion,
    padding: u32,
    image_width: u32,
    image_height: u32,
) -> Vec<ScanCandidate> {
    let mut candidates = Vec::new();
    if let Some(region) = ribbon_region_from_dark_bounds(bounds, image_width, image_height) {
        candidates.push(ScanCandidate::new(
            CandidateDetector::RibbonWeave,
            Some(LayoutFamily::RibbonWeave),
            "dark-ribbon",
            region,
        ));
    }
    let expanded = expand_region(bounds, padding, image_width, image_height);
    candidates.push(ScanCandidate::new(
        CandidateDetector::RibbonWeave,
        Some(LayoutFamily::RibbonWeave),
        "dark-bounds",
        expanded,
    ));
    if let Some(region) = ribbon_aspect_region(expanded, image_width, image_height) {
        candidates.push(ScanCandidate::new(
            CandidateDetector::RibbonWeave,
            Some(LayoutFamily::RibbonWeave),
            "signature-window",
            region,
        ));
    }
    candidates
}

fn reference_ribbon_geometry(width: u32, height: u32) -> bool {
    width >= 96
        && height >= 28
        && width % 4 == 0
        && height % 4 == 0
        && (2.0..=8.0).contains(&(width as f32 / height.max(1) as f32))
}

fn clamp_region(region: ScanRegion, image_width: u32, image_height: u32) -> ScanRegion {
    let x = region.x.min(image_width.saturating_sub(1));
    let y = region.y.min(image_height.saturating_sub(1));
    let max_x = region.x.saturating_add(region.width).min(image_width);
    let max_y = region.y.saturating_add(region.height).min(image_height);
    ScanRegion {
        x,
        y,
        width: max_x.saturating_sub(x).max(1),
        height: max_y.saturating_sub(y).max(1),
    }
}

fn ribbon_aspect_region(
    region: ScanRegion,
    image_width: u32,
    image_height: u32,
) -> Option<ScanRegion> {
    let target_height = ((region.width as f32 * 44.0 / 104.0).round() as u32).max(region.height);
    let target_width = ((region.height as f32 * 104.0 / 44.0).round() as u32).max(region.width);
    let width_first = centered_region(
        region,
        region.width,
        target_height,
        image_width,
        image_height,
    );
    let height_first = centered_region(
        region,
        target_width,
        region.height,
        image_width,
        image_height,
    );

    [width_first, height_first]
        .into_iter()
        .flatten()
        .filter(|candidate| plausible_region(*candidate))
        .min_by_key(|candidate| {
            candidate
                .width
                .saturating_mul(candidate.height)
                .saturating_sub(region.width.saturating_mul(region.height))
        })
}

fn ribbon_region_from_dark_bounds(
    bounds: ScanRegion,
    image_width: u32,
    image_height: u32,
) -> Option<ScanRegion> {
    if bounds.width < 48 || bounds.height < 24 {
        return None;
    }
    let total_width = ((bounds.width as f32 * 104.0 / 96.0).round() as u32).max(bounds.width);
    let total_height = ((total_width as f32 * 44.0 / 104.0).round() as u32).max(bounds.height);
    centered_region(bounds, total_width, total_height, image_width, image_height)
        .filter(|candidate| plausible_region(*candidate))
}

fn centered_region(
    region: ScanRegion,
    width: u32,
    height: u32,
    image_width: u32,
    image_height: u32,
) -> Option<ScanRegion> {
    if width > image_width || height > image_height {
        return None;
    }
    let center_x = region.x.saturating_add(region.width / 2);
    let center_y = region.y.saturating_add(region.height / 2);
    let x = center_x
        .saturating_sub(width / 2)
        .min(image_width.saturating_sub(width));
    let y = center_y
        .saturating_sub(height / 2)
        .min(image_height.saturating_sub(height));
    Some(ScanRegion {
        x,
        y,
        width,
        height,
    })
}

fn should_try_full_frame_decode(image: &DynamicImage) -> bool {
    let width = image.width();
    let height = image.height();
    let area = width.saturating_mul(height);
    if area > 300_000 {
        return false;
    }
    let aspect = width as f32 / height.max(1) as f32;
    (1.0..=8.0).contains(&aspect)
}

fn decode_candidate(
    decoder: &RasterDecoder,
    image: &DynamicImage,
    candidate: ScanCandidate,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    let region = candidate.region;
    if candidate.stage == "signature-window" {
        if let Ok(decoded) = decode_exact_ribbon_candidate(image, region) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_fractional_ribbon_candidate(image) {
            return Ok(decoded);
        }
        let target_module_px = 4;
        let resized = image::imageops::resize(
            image,
            104 * target_module_px,
            44 * target_module_px,
            image::imageops::FilterType::Triangle,
        );
        let resized = DynamicImage::ImageRgba8(resized);
        let normalized_region = ScanRegion {
            x: 0,
            y: 0,
            width: 104 * target_module_px,
            height: 44 * target_module_px,
        };
        if let Ok(decoded) = decode_exact_ribbon_candidate(&resized, normalized_region) {
            return Ok(decoded);
        }
        return Err(DecodeError::AutoDetectFailed);
    }

    if matches!(
        candidate.stage,
        "reference-sweep" | "component-reference" | "dark-bounds" | "dark-ribbon"
    ) {
        if let Ok(decoded) = decode_exact_ribbon_candidate(image, region) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_fractional_ribbon_candidate(image) {
            return Ok(decoded);
        }
    }
    decoder.decode_auto_with_info(image)
}

fn decode_exact_ribbon_candidate(
    image: &DynamicImage,
    region: ScanRegion,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    if region.width >= 104 && region.height >= 44 {
        let module_px = (region.width / 104).max(1);
        if region.width == 104 * module_px && region.height == 44 * module_px {
            for threshold in [160, 192, 224] {
                let exact = RasterDecoder::new(DecodeOptions {
                    module_px,
                    quiet_zone_modules: 4,
                    threshold,
                    layout: LayoutFamily::RibbonWeave,
                });
                if let Ok(decoded) = exact.decode(image) {
                    return Ok(AutoDecodedSymbol {
                        decoded,
                        info: glyphnet_decode::AutoDecodeInfo {
                            module_px,
                            quiet_zone_modules: 4,
                            threshold,
                            layout: LayoutFamily::RibbonWeave,
                        },
                    });
                }
            }
        }
    }
    Err(DecodeError::AutoDetectFailed)
}

fn decode_fractional_ribbon_candidate(
    image: &DynamicImage,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    const SYMBOL_WIDTH: u16 = 96;
    const SYMBOL_HEIGHT: u16 = 36;
    const TOTAL_WIDTH_MODULES: f32 = 104.0;
    const TOTAL_HEIGHT_MODULES: f32 = 44.0;
    const QUIET_MODULES: f32 = 4.0;

    let luma = image.to_luma8();
    if luma.width() < 104 || luma.height() < 44 {
        return Err(DecodeError::AutoDetectFailed);
    }
    let base_scale_x = luma.width() as f32 / TOTAL_WIDTH_MODULES;
    let base_scale_y = luma.height() as f32 / TOTAL_HEIGHT_MODULES;
    if base_scale_x < 1.0 || base_scale_y < 1.0 {
        return Err(DecodeError::AutoDetectFailed);
    }

    let otsu = fractional_threshold(&luma);
    let integral = IntegralGray::new(&luma);
    let mut thresholds = vec![otsu, 160, 192, 224];
    thresholds.sort_unstable();
    thresholds.dedup();

    for scale_adjust in [1.0_f32, 0.985, 1.015, 0.97, 1.03] {
        let scale_x = base_scale_x * scale_adjust;
        let scale_y = base_scale_y * scale_adjust;
        if scale_x < 1.0 || scale_y < 1.0 {
            continue;
        }
        for y_shift in module_shifts(3) {
            for x_shift in module_shifts(2) {
                let origin_x = QUIET_MODULES + x_shift;
                let origin_y = QUIET_MODULES + y_shift;
                if origin_x < -2.0 || origin_y < -8.0 {
                    continue;
                }
                if !fractional_grid_fits(
                    &luma,
                    origin_x,
                    origin_y,
                    scale_x,
                    scale_y,
                    SYMBOL_WIDTH,
                    SYMBOL_HEIGHT,
                ) {
                    continue;
                }
                for &threshold in &thresholds {
                    if !fractional_header_precheck(
                        &integral, origin_x, origin_y, scale_x, scale_y, threshold,
                    ) {
                        continue;
                    }
                    if let Ok(decoded) = decode_fractional_with_params(
                        &integral, origin_x, origin_y, scale_x, scale_y, threshold,
                    ) {
                        return Ok(decoded);
                    }
                }
            }
        }
    }

    Err(DecodeError::AutoDetectFailed)
}

fn module_shifts(radius: i32) -> impl Iterator<Item = f32> {
    (-radius * 2..=radius * 2).map(|value| value as f32 * 0.5)
}

fn fractional_header_precheck(
    integral: &IntegralGray,
    origin_x_modules: f32,
    origin_y_modules: f32,
    scale_x: f32,
    scale_y: f32,
    threshold: u8,
) -> bool {
    const SYMBOL_WIDTH: u16 = 96;
    const SYMBOL_HEIGHT: u16 = 36;

    let mut bits = Vec::with_capacity(HEADER_LEN * 8);
    'rows: for y in 0..SYMBOL_HEIGHT {
        for x in 0..SYMBOL_WIDTH {
            if !layout::is_data_module_for(
                LayoutFamily::RibbonWeave,
                SYMBOL_WIDTH,
                SYMBOL_HEIGHT,
                x,
                y,
            ) {
                continue;
            }
            let avg = fractional_module_luma(
                integral,
                origin_x_modules + f32::from(x),
                origin_y_modules + f32::from(y),
                scale_x,
                scale_y,
            );
            bits.push(avg < threshold);
            if bits.len() == HEADER_LEN * 8 {
                break 'rows;
            }
        }
    }
    if bits.len() < HEADER_LEN * 8 {
        return false;
    }
    FrameHeader::decode(&bitstream::bits_to_bytes(&bits)).is_ok()
}

fn fractional_grid_fits(
    luma: &GrayImage,
    origin_x_modules: f32,
    origin_y_modules: f32,
    scale_x: f32,
    scale_y: f32,
    symbol_width: u16,
    symbol_height: u16,
) -> bool {
    let min_x = origin_x_modules * scale_x;
    let min_y = origin_y_modules * scale_y;
    let max_x = (origin_x_modules + f32::from(symbol_width)) * scale_x;
    let max_y = (origin_y_modules + f32::from(symbol_height)) * scale_y;
    min_x >= -scale_x
        && min_y >= -scale_y
        && max_x < luma.width() as f32 + scale_x
        && max_y < luma.height() as f32 + scale_y
}

fn decode_fractional_with_params(
    integral: &IntegralGray,
    origin_x_modules: f32,
    origin_y_modules: f32,
    scale_x: f32,
    scale_y: f32,
    threshold: u8,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    const SYMBOL_WIDTH: u16 = 96;
    const SYMBOL_HEIGHT: u16 = 36;

    let mut matrix =
        SymbolMatrix::with_layout(SYMBOL_WIDTH, SYMBOL_HEIGHT, LayoutFamily::RibbonWeave);
    for y in 0..SYMBOL_HEIGHT {
        for x in 0..SYMBOL_WIDTH {
            if let Some(cell) = layout::function_cell_for(
                LayoutFamily::RibbonWeave,
                SYMBOL_WIDTH,
                SYMBOL_HEIGHT,
                x,
                y,
            ) {
                matrix.set(x, y, cell)?;
                continue;
            }
            let avg = fractional_module_luma(
                integral,
                origin_x_modules + f32::from(x),
                origin_y_modules + f32::from(y),
                scale_x,
                scale_y,
            );
            matrix.set(x, y, Cell::Data(avg < threshold))?;
        }
    }

    let decoded = decode_matrix(&matrix)?;
    Ok(AutoDecodedSymbol {
        decoded,
        info: glyphnet_decode::AutoDecodeInfo {
            module_px: scale_x.round().max(1.0) as u32,
            quiet_zone_modules: 4,
            threshold,
            layout: LayoutFamily::RibbonWeave,
        },
    })
}

fn fractional_threshold(luma: &GrayImage) -> u8 {
    let mut histogram = [0u32; 256];
    for pixel in luma.pixels() {
        histogram[pixel[0] as usize] += 1;
    }
    let total = u64::from(luma.width()).saturating_mul(u64::from(luma.height()));
    if total == 0 {
        return 128;
    }
    let mut sum_total = 0u64;
    for (value, count) in histogram.iter().enumerate() {
        sum_total += value as u64 * u64::from(*count);
    }
    let mut sum_background = 0u64;
    let mut weight_background = 0u64;
    let mut best_variance = -1.0f64;
    let mut threshold = 128u8;
    for (value, count) in histogram.iter().enumerate() {
        weight_background += u64::from(*count);
        if weight_background == 0 {
            continue;
        }
        let weight_foreground = total - weight_background;
        if weight_foreground == 0 {
            break;
        }
        sum_background += value as u64 * u64::from(*count);
        let mean_background = sum_background as f64 / weight_background as f64;
        let mean_foreground = (sum_total - sum_background) as f64 / weight_foreground as f64;
        let diff = mean_background - mean_foreground;
        let variance = weight_background as f64 * weight_foreground as f64 * diff * diff;
        if variance > best_variance {
            best_variance = variance;
            threshold = value as u8;
        }
    }
    threshold
}

fn fractional_module_luma(
    integral: &IntegralGray,
    module_x: f32,
    module_y: f32,
    scale_x: f32,
    scale_y: f32,
) -> u8 {
    let center_x = (module_x + 0.5) * scale_x;
    let center_y = (module_y + 0.5) * scale_y;
    let half_x = (scale_x * 0.28).max(0.75);
    let half_y = (scale_y * 0.28).max(0.75);
    let start_x = (center_x - half_x).floor().max(0.0) as u32;
    let end_x = (center_x + half_x)
        .ceil()
        .min(integral.width.saturating_sub(1) as f32) as u32;
    let start_y = (center_y - half_y).floor().max(0.0) as u32;
    let end_y = (center_y + half_y)
        .ceil()
        .min(integral.height.saturating_sub(1) as f32) as u32;

    let count = end_x
        .saturating_sub(start_x)
        .saturating_add(1)
        .saturating_mul(end_y.saturating_sub(start_y).saturating_add(1));
    if count == 0 {
        return 255;
    }
    let sum = integral.sum_inclusive(start_x, start_y, end_x, end_y);
    (sum / count) as u8
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IntegralGray {
    width: u32,
    height: u32,
    stride: usize,
    sums: Vec<u32>,
}

impl IntegralGray {
    fn new(luma: &GrayImage) -> Self {
        let width = luma.width();
        let height = luma.height();
        let stride = width as usize + 1;
        let mut sums = vec![0u32; stride * (height as usize + 1)];

        for y in 0..height {
            let mut row_sum = 0u32;
            for x in 0..width {
                row_sum += u32::from(luma.get_pixel(x, y).0[0]);
                let index = (y as usize + 1) * stride + x as usize + 1;
                let above = y as usize * stride + x as usize + 1;
                sums[index] = sums[above] + row_sum;
            }
        }

        Self {
            width,
            height,
            stride,
            sums,
        }
    }

    fn sum_inclusive(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> u32 {
        let x0 = x0.min(self.width) as usize;
        let y0 = y0.min(self.height) as usize;
        let x1 = x1.saturating_add(1).min(self.width) as usize;
        let y1 = y1.saturating_add(1).min(self.height) as usize;
        let a = self.sums[y0 * self.stride + x0];
        let b = self.sums[y0 * self.stride + x1];
        let c = self.sums[y1 * self.stride + x0];
        let d = self.sums[y1 * self.stride + x1];
        d + a - b - c
    }
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
    components.sort_by_key(|component| std::cmp::Reverse(component.pixels));
    components
}

fn pixel_index(width: u32, x: u32, y: u32) -> usize {
    y as usize * width as usize + x as usize
}

const MIN_COMPONENT_PIXELS: u32 = 16;
const MAX_CANDIDATE_REGIONS: usize = 96;

fn ribbon_weave_candidates(
    binary: &GrayImage,
    image_width: u32,
    image_height: u32,
) -> Vec<ScanCandidate> {
    let mut regions = Vec::new();
    regions.extend(ribbon_totem_regions(binary, image_width, image_height));
    regions.extend(ribbon_rail_regions(binary, image_width, image_height));
    regions.truncate(MAX_CANDIDATE_REGIONS);
    regions
}

fn generic_binary_candidates(
    binary: &GrayImage,
    padding: u32,
    image_width: u32,
    image_height: u32,
) -> Vec<ScanCandidate> {
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
            push_unique_candidate(
                &mut regions,
                CandidateDetector::GenericBinary,
                None,
                "component",
                expanded,
            );
        }
    }

    regions.truncate(MAX_CANDIDATE_REGIONS);
    regions
}

#[derive(Debug, Clone, Copy)]
struct RailRow {
    y: u32,
    min_x: u32,
    max_x: u32,
    transitions: u32,
    dark: u32,
}

#[derive(Debug, Clone, Copy)]
struct RailGroup {
    y: u32,
    min_x: u32,
    max_x: u32,
    transitions: u32,
    dark: u32,
    rows: u32,
}

#[derive(Debug, Clone, Copy)]
struct TotemGroup {
    x: u32,
    min_y: u32,
    max_y: u32,
    dark: u32,
    columns: u32,
}

fn ribbon_totem_regions(
    binary: &GrayImage,
    image_width: u32,
    image_height: u32,
) -> Vec<ScanCandidate> {
    if image_width < 160 || image_height < 80 {
        return Vec::new();
    }

    let groups = totem_groups(binary, image_width, image_height);
    let mut regions = Vec::new();

    for left in &groups {
        for right in &groups {
            if right.x <= left.x + 120 {
                continue;
            }
            let overlap_y = left
                .max_y
                .min(right.max_y)
                .saturating_sub(left.min_y.max(right.min_y));
            if overlap_y < 120 {
                continue;
            }
            let dx = right.x.saturating_sub(left.x) as f32;
            for module_span in [83.0_f32, 85.0] {
                let scale = dx / module_span;
                if !(2.0..=32.0).contains(&scale) {
                    continue;
                }
                let left_x = left.x as f32;
                let top_y = left.min_y.min(right.min_y) as f32;
                for left_module_x in [9.0_f32, 11.0] {
                    for totem_top_module_y in [7.0_f32, 9.0] {
                        push_fractional_region(
                            &mut regions,
                            left_x - left_module_x * scale,
                            top_y - totem_top_module_y * scale,
                            104.0 * scale,
                            44.0 * scale,
                            image_width,
                            image_height,
                        );
                    }
                }
            }
        }
    }

    regions.truncate(12);
    regions
}

fn totem_groups(binary: &GrayImage, image_width: u32, image_height: u32) -> Vec<TotemGroup> {
    let mut columns = Vec::new();
    for x in 0..image_width {
        let mut runs = Vec::new();
        let mut run_start = None;
        for y in 0..image_height {
            let is_dark = binary.get_pixel(x, y).0[0] == 0;
            match (run_start, is_dark) {
                (None, true) => run_start = Some(y),
                (Some(start), false) => {
                    if y.saturating_sub(start) >= 2 {
                        runs.push((start, y - 1));
                    }
                    run_start = None;
                }
                _ => {}
            }
        }
        if let Some(start) = run_start
            && image_height.saturating_sub(start) >= 2
        {
            runs.push((start, image_height - 1));
        }

        let Some((min_y, max_y, dark, transitions)) = best_totem_column_cluster(&runs) else {
            continue;
        };
        let span = max_y.saturating_sub(min_y).saturating_add(1);
        let density = dark as f32 / span as f32;
        if span >= 160 && transitions >= 12 && (0.08..=0.55).contains(&density) {
            columns.push(TotemGroup {
                x,
                min_y,
                max_y,
                dark,
                columns: 1,
            });
        }
    }

    let mut groups = Vec::new();
    let mut current: Option<TotemGroup> = None;
    for column in columns {
        match current {
            Some(mut group) if column.x <= group.x + 4 => {
                let total = group.columns + 1;
                group.x = (group.x * group.columns + column.x) / total;
                group.min_y = group.min_y.min(column.min_y);
                group.max_y = group.max_y.max(column.max_y);
                group.dark += column.dark;
                group.columns = total;
                current = Some(group);
            }
            Some(group) => {
                groups.push(group);
                current = Some(column);
            }
            None => current = Some(column),
        }
    }
    if let Some(group) = current {
        groups.push(group);
    }

    groups.sort_by(|a, b| {
        let a_span = a.max_y.saturating_sub(a.min_y);
        let b_span = b.max_y.saturating_sub(b.min_y);
        b_span.cmp(&a_span).then_with(|| b.dark.cmp(&a.dark))
    });
    groups.truncate(48);
    groups
}

fn best_totem_column_cluster(runs: &[(u32, u32)]) -> Option<(u32, u32, u32, u32)> {
    let mut best: Option<(u32, u32, u32, u32)> = None;
    let mut index = 0usize;
    while index < runs.len() {
        let min_y = runs[index].0;
        let mut max_y = runs[index].1;
        let mut dark = runs[index]
            .1
            .saturating_sub(runs[index].0)
            .saturating_add(1);
        let mut transitions = 2u32;
        index += 1;
        while index < runs.len() && runs[index].0 <= max_y + 64 {
            max_y = runs[index].1;
            dark += runs[index]
                .1
                .saturating_sub(runs[index].0)
                .saturating_add(1);
            transitions += 2;
            index += 1;
        }

        let span = max_y.saturating_sub(min_y).saturating_add(1);
        let score = span.saturating_mul(transitions).saturating_mul(dark);
        let best_score = best
            .map(|(best_min, best_max, best_dark, best_transitions)| {
                best_max
                    .saturating_sub(best_min)
                    .saturating_add(1)
                    .saturating_mul(best_transitions)
                    .saturating_mul(best_dark)
            })
            .unwrap_or(0);
        if score > best_score {
            best = Some((min_y, max_y, dark, transitions));
        }
    }
    best
}

fn ribbon_rail_regions(
    binary: &GrayImage,
    image_width: u32,
    image_height: u32,
) -> Vec<ScanCandidate> {
    if image_width < 160 || image_height < 80 {
        return Vec::new();
    }

    let groups = rail_groups(binary, image_width, image_height);
    let mut regions = Vec::new();

    for group in &groups {
        let span = group.max_x.saturating_sub(group.min_x).saturating_add(1);
        if span < 120 {
            continue;
        }

        let estimated_module = (span as f32 / 68.0).round() as i32;
        for module_px in (estimated_module - 1)..=(estimated_module + 1) {
            if !(2..=24).contains(&module_px) {
                continue;
            }
            let module_px = module_px as u32;
            for rail_module_y in [7_u32, 9] {
                let base_x = group.min_x.saturating_sub(20 * module_px).saturating_add(2);
                let base_y = group
                    .y
                    .saturating_sub(rail_module_y * module_px + module_px / 2 + 2);
                for x_nudge in [0, 2] {
                    for y_nudge in [0, module_px / 2 + 1] {
                        let region = ScanRegion {
                            x: base_x.saturating_add(x_nudge),
                            y: base_y.saturating_add(y_nudge),
                            width: 104 * module_px,
                            height: 44 * module_px,
                        };
                        if region_fits(region, image_width, image_height) {
                            push_unique_candidate(
                                &mut regions,
                                CandidateDetector::RibbonWeave,
                                Some(LayoutFamily::RibbonWeave),
                                "signature-window",
                                region,
                            );
                        }
                    }
                }
            }
        }

        push_scaled_signature_regions(
            &mut regions,
            group.min_x,
            group.y,
            span,
            image_width,
            image_height,
        );
    }

    for upper in &groups {
        for lower in &groups {
            if lower.y <= upper.y + 48 {
                continue;
            }

            let upper_span = upper.max_x.saturating_sub(upper.min_x).saturating_add(1);
            let lower_span = lower.max_x.saturating_sub(lower.min_x).saturating_add(1);
            let overlap = upper
                .max_x
                .min(lower.max_x)
                .saturating_sub(upper.min_x.max(lower.min_x));
            if overlap < upper_span.min(lower_span) / 2 {
                continue;
            }

            let span = upper_span.max(lower_span);
            let estimated_module = (span as f32 / 68.0).round() as i32;
            for module_px in (estimated_module - 1)..=(estimated_module + 1) {
                if !(2..=24).contains(&module_px) {
                    continue;
                }
                let module_px = module_px as u32;
                let expected_symbol_height = 44 * module_px;
                let vertical_distance = lower.y.saturating_sub(upper.y);
                if vertical_distance + 10 * module_px < 20 * module_px
                    || vertical_distance > expected_symbol_height
                {
                    continue;
                }
                for rail_module_y in [7_u32, 9] {
                    let min_x = upper.min_x.min(lower.min_x);
                    let base_x = min_x.saturating_sub(20 * module_px).saturating_add(2);
                    let base_y = upper
                        .y
                        .saturating_sub(rail_module_y * module_px + module_px / 2 + 2);
                    for x_nudge in [0, 2] {
                        for y_nudge in [0, module_px / 2 + 1] {
                            let region = ScanRegion {
                                x: base_x.saturating_add(x_nudge),
                                y: base_y.saturating_add(y_nudge),
                                width: 104 * module_px,
                                height: expected_symbol_height,
                            };
                            if region_fits(region, image_width, image_height) {
                                push_unique_candidate(
                                    &mut regions,
                                    CandidateDetector::RibbonWeave,
                                    Some(LayoutFamily::RibbonWeave),
                                    "signature-window",
                                    region,
                                );
                            }
                        }
                    }
                }
            }

            push_scaled_signature_regions(
                &mut regions,
                upper.min_x.min(lower.min_x),
                upper.y,
                span,
                image_width,
                image_height,
            );
        }
    }

    regions.truncate(32);
    regions
}

fn push_scaled_signature_regions(
    regions: &mut Vec<ScanCandidate>,
    rail_min_x: u32,
    rail_y: u32,
    rail_span: u32,
    image_width: u32,
    image_height: u32,
) {
    let scale = rail_span as f32 / 68.0;
    if scale < 1.5 {
        return;
    }
    let width = (104.0 * scale).round().max(104.0) as u32;
    let height = (44.0 * scale).round().max(44.0) as u32;
    let x_base = rail_min_x as i64 - (18.0 * scale).round() as i64;
    for rail_module_y in [7.0_f32, 9.0] {
        let y_base = rail_y as i64 - (rail_module_y * scale).round() as i64;
        for x_nudge in [-2_i64, 0, 2] {
            for y_nudge in [-2_i64, 0, 2] {
                let x = (x_base + x_nudge).max(0) as u32;
                let y = (y_base + y_nudge).max(0) as u32;
                let region = ScanRegion {
                    x,
                    y,
                    width,
                    height,
                };
                if region_fits(region, image_width, image_height) {
                    push_unique_candidate(
                        regions,
                        CandidateDetector::RibbonWeave,
                        Some(LayoutFamily::RibbonWeave),
                        "signature-window",
                        region,
                    );
                }
            }
        }
    }
}

fn push_fractional_region(
    regions: &mut Vec<ScanCandidate>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    image_width: u32,
    image_height: u32,
) {
    let region = ScanRegion {
        x: x.round().max(0.0) as u32,
        y: y.round().max(0.0) as u32,
        width: width.round().max(104.0) as u32,
        height: height.round().max(44.0) as u32,
    };
    if region_fits(region, image_width, image_height) {
        push_unique_candidate(
            regions,
            CandidateDetector::RibbonWeave,
            Some(LayoutFamily::RibbonWeave),
            "signature-window",
            region,
        );
    }
}

fn rail_groups(binary: &GrayImage, image_width: u32, image_height: u32) -> Vec<RailGroup> {
    let mut rows = Vec::new();
    for y in 0..image_height {
        if let Some(row) = rail_row(binary, image_width, y) {
            rows.push(row);
        }
    }

    let mut groups = Vec::new();
    let mut current: Option<RailGroup> = None;
    for row in rows {
        match current {
            Some(mut group) if row.y <= group.y + 4 => {
                let total_rows = group.rows + 1;
                group.y = (group.y * group.rows + row.y) / total_rows;
                group.min_x = group.min_x.min(row.min_x);
                group.max_x = group.max_x.max(row.max_x);
                group.transitions += row.transitions;
                group.dark += row.dark;
                group.rows = total_rows;
                current = Some(group);
            }
            Some(group) => {
                if group.rows >= 2 {
                    groups.push(group);
                }
                current = Some(RailGroup {
                    y: row.y,
                    min_x: row.min_x,
                    max_x: row.max_x,
                    transitions: row.transitions,
                    dark: row.dark,
                    rows: 1,
                });
            }
            None => {
                current = Some(RailGroup {
                    y: row.y,
                    min_x: row.min_x,
                    max_x: row.max_x,
                    transitions: row.transitions,
                    dark: row.dark,
                    rows: 1,
                });
            }
        }
    }
    if let Some(group) = current
        && group.rows >= 2
    {
        groups.push(group);
    }

    groups.sort_by(|a, b| rail_group_score(*a).total_cmp(&rail_group_score(*b)));
    groups.truncate(32);
    groups
}

fn rail_group_score(group: RailGroup) -> f32 {
    let span = group.max_x.saturating_sub(group.min_x).saturating_add(1);
    let module_px = (span as f32 / 68.0).round().max(1.0);
    let residual = (span as f32 - 68.0 * module_px).abs() / module_px;
    let density = group.dark as f32 / (span.saturating_mul(group.rows).max(1)) as f32;
    let density_error = (density - 0.25).abs() * 24.0;
    density_error + residual
}

fn rail_row(binary: &GrayImage, image_width: u32, y: u32) -> Option<RailRow> {
    let mut dark = 0u32;
    let mut transition_positions = Vec::new();
    let mut last_dark = false;
    let mut seen = false;

    for x in 0..image_width {
        let is_dark = binary.get_pixel(x, y).0[0] == 0;
        if is_dark {
            dark += 1;
        }
        if seen && is_dark != last_dark {
            transition_positions.push(x);
        }
        seen = true;
        last_dark = is_dark;
    }

    if dark == 0 || transition_positions.len() < 14 {
        return None;
    }

    let mut best: Option<(usize, usize)> = None;
    let mut start = 0usize;
    for index in 1..transition_positions.len() {
        if transition_positions[index].saturating_sub(transition_positions[index - 1]) > 64 {
            best = choose_transition_cluster(best, start, index - 1);
            start = index;
        }
    }
    best = choose_transition_cluster(best, start, transition_positions.len() - 1);

    let (start, end) = best?;
    let transitions = (end - start + 1) as u32;
    if transitions < 14 {
        return None;
    }

    let min_x = transition_positions[start].saturating_sub(12);
    let max_x = transition_positions[end]
        .saturating_add(12)
        .min(image_width.saturating_sub(1));
    let span = max_x.saturating_sub(min_x).saturating_add(1);
    if span < 120 {
        return None;
    }

    let mut cluster_dark = 0u32;
    for x in min_x..=max_x {
        if binary.get_pixel(x, y).0[0] == 0 {
            cluster_dark += 1;
        }
    }
    let density = cluster_dark as f32 / span as f32;
    if !(0.18..=0.78).contains(&density) {
        return None;
    }

    Some(RailRow {
        y,
        min_x,
        max_x,
        transitions,
        dark: cluster_dark,
    })
}

fn choose_transition_cluster(
    best: Option<(usize, usize)>,
    start: usize,
    end: usize,
) -> Option<(usize, usize)> {
    if end < start {
        return best;
    }
    match best {
        Some((best_start, best_end)) if best_end - best_start >= end - start => {
            Some((best_start, best_end))
        }
        _ => Some((start, end)),
    }
}

fn region_fits(region: ScanRegion, image_width: u32, image_height: u32) -> bool {
    region.width >= 104
        && region.height >= 44
        && region.x.saturating_add(region.width) <= image_width
        && region.y.saturating_add(region.height) <= image_height
}

fn push_unique_candidate(
    regions: &mut Vec<ScanCandidate>,
    detector: CandidateDetector,
    layout_hint: Option<LayoutFamily>,
    stage: &'static str,
    region: ScanRegion,
) {
    if !regions.iter().any(|candidate| candidate.region == region) {
        regions.push(ScanCandidate::new(detector, layout_hint, stage, region));
    }
}

fn horizontal_band_regions(
    binary: &GrayImage,
    padding: u32,
    image_width: u32,
    image_height: u32,
) -> Vec<ScanCandidate> {
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
            push_unique_candidate(
                &mut regions,
                CandidateDetector::GenericBinary,
                None,
                "horizontal-band",
                expanded,
            );
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
                    push_unique_candidate(
                        &mut regions,
                        CandidateDetector::GenericBinary,
                        None,
                        "horizontal-aspect",
                        region,
                    );
                }
            }
        }
    }

    regions.sort_by(|a, b| region_score(b.region).total_cmp(&region_score(a.region)));
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
        assert!(matches!(
            result.crop,
            Some(ScanRegion {
                x: 80..=140,
                y: 60..=110,
                ..
            })
        ));
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
            result
                .attempts
                .iter()
                .any(|attempt| attempt.detector == CandidateDetector::GeneratedContent.as_str())
        );
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
    #[ignore = "module_px=2 embedded samples need stronger low-resolution signature detection"]
    fn scan_still_decodes_small_debugger_sample_canvas() {
        let image = sample_canvas(b"debug sample", 2, 80, 72);
        assert_scan_payload(&image, b"debug sample");
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
        assert!(
            elapsed.as_secs_f32() < 20.0,
            "real screenshot scan took {elapsed:?}"
        );
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
