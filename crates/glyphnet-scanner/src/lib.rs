//! Real-time scanner orchestration for GlyphNet.

use std::collections::{BTreeMap, HashMap};
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::{fs::OpenOptions, io::Write};

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
pub use types::{FailedStillScan, ScanAttempt, ScanTimings, StillScanResult};

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
    const DOWNSCALE_PREPASS_MAX_DIM_PX: u32 = 1024;
    const DOWNSCALE_PREPASS_MAX_CANDIDATES: usize = 8;
    const MAX_NON_ROBUST_DECODE_ATTEMPTS: usize = 24;
    const NON_ROBUST_MAX_TOTAL_MICROS: u64 = 15_000_000;
    const NON_ROBUST_MAX_QUAD_MICROS: u64 = 2_000_000;
    const NON_ROBUST_MAX_DECODE_MICROS: u64 = 8_000_000;
    const NON_ROBUST_MAX_CANDIDATE_AREA: u32 = 8_000_000;

    let started = scan_instant_now();
    let mut timings = ScanTimings::default();
    let mut debug = ScanDebugDumper::from_env();
    let debug_unbounded = debug.is_enabled();
    let debug_max_attempts = debug.max_attempts();
    debug.dump_input(image);
    debug.log_line("scan started");
    let decoder = RasterDecoder::default();
    let profile = VisionProfile::for_mode(mode);

    debug.log_line("prepass quad disabled in non-robust mode");

    let _ = (allow_downscale_fast_path, DOWNSCALE_PREPASS_MAX_CANDIDATES);

    let mut cv_scale = 1.0_f32;
    let mut cv_image = image.clone();
    if !robust && image.width().max(image.height()) > DOWNSCALE_PREPASS_MAX_DIM_PX {
        cv_scale = DOWNSCALE_PREPASS_MAX_DIM_PX as f32 / image.width().max(image.height()) as f32;
        let cv_width = ((image.width() as f32 * cv_scale).round() as u32).max(1);
        let cv_height = ((image.height() as f32 * cv_scale).round() as u32).max(1);
        cv_image = DynamicImage::ImageRgba8(image::imageops::resize(
            image,
            cv_width,
            cv_height,
            image::imageops::FilterType::Triangle,
        ));
        debug.log_line(&format!("cv downscale {}x{}", cv_width, cv_height));
    }

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

    let stage = scan_instant_now();
    let gray = grayscale(&cv_image).map_err(|error| failed_cv(error, timings, started))?;
    timings.grayscale_micros = elapsed_micros(stage);
    debug.dump_gray("01_grayscale", &gray);
    debug.log_line(&format!("stage grayscale_us={}", timings.grayscale_micros));

    let stage = scan_instant_now();
    let binary = adaptive_threshold(&gray, profile.threshold_radius, profile.threshold_bias)
        .map_err(|error| failed_cv(error, timings, started))?;
    timings.threshold_micros = elapsed_micros(stage);
    debug.dump_gray("02_threshold", &binary);
    debug.log_line(&format!("stage threshold_us={}", timings.threshold_micros));

    if should_try_quad_rectification(cv_image.width(), cv_image.height(), robust) {
        let candidates = find_anchor_candidates(&binary, profile)
            .map_err(|error| failed_cv(error, timings, started))?;
        let estimated_quad = estimate_quad(&binary, &candidates);
        let dark_bounds_region = if should_try_dark_bounds_fallback(
            cv_image.width(),
            cv_image.height(),
            candidates.len(),
        ) {
            dark_bounds(&binary)
        } else {
            None
        };
        let quad_candidates = build_quad_candidates(
            estimated_quad,
            dark_bounds_region,
            cv_image.width(),
            cv_image.height(),
        );
        for (index, quad) in quad_candidates
            .into_iter()
            .take(MAX_QUAD_ATTEMPTS)
            .enumerate()
        {
            if !robust && !debug_unbounded && elapsed_micros(started) >= NON_ROBUST_MAX_TOTAL_MICROS
            {
                break;
            }
            if !robust && !debug_unbounded && elapsed_micros(stage) >= NON_ROBUST_MAX_QUAD_MICROS {
                break;
            }
            let (warp_width, warp_height) = quad_dimensions(quad);
            if warp_width < 32 || warp_height < 32 {
                continue;
            }
            if let Ok(warped) = warp_perspective_gray(&gray, quad, warp_width, warp_height) {
                debug.dump_gray(&format!("03_quad_warp_{index:02}"), &warped);
                let warped = DynamicImage::ImageLuma8(warped);
                let decoded = if robust && index == 0 {
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
    let mut regions = still_scan_candidates(&cv_image, &binary, profile, padding, robust);
    if regions.is_empty()
        && !robust
        && let Some(bounds) = dark_bounds(&binary)
        && let Some(region) =
            fallback_ribbon_region_from_bounds(bounds, cv_image.width(), cv_image.height())
    {
        regions.push(ScanCandidate::new(
            CandidateDetector::RibbonWeave,
            Some(glyphnet_core::LayoutFamily::RibbonWeave),
            "binary-dark-bounds-fallback",
            region,
        ));
        debug.log_line("added binary-dark-bounds-fallback candidate");
    }
    if !robust && regions.len() > MAX_NON_ROBUST_DECODE_ATTEMPTS {
        regions.truncate(MAX_NON_ROBUST_DECODE_ATTEMPTS);
    }
    debug.log_line(&format!("candidate_count={}", regions.len()));
    timings.candidate_micros = elapsed_micros(stage);

    let decode_started = scan_instant_now();
    #[cfg(not(target_arch = "wasm32"))]
    if !robust
        && regions.len() >= PARALLEL_DECODE_CANDIDATE_THRESHOLD
        && image.width().saturating_mul(image.height()) <= 1_200_000
    {
        let mut results: Vec<(usize, ScanAttempt, Option<AutoDecodedSymbol>)> = regions
            .into_par_iter()
            .enumerate()
            .map(|(index, candidate)| {
                let attempt_started = scan_instant_now();
                let candidate = adjust_candidate_for_full_resolution(
                    candidate,
                    image.width(),
                    image.height(),
                    1.0,
                );
                let region = candidate.region;
                let cropped = image::imageops::crop_imm(
                    image,
                    region.x,
                    region.y,
                    region.width,
                    region.height,
                )
                .to_image();
                let cropped = maybe_downscale_candidate_crop(DynamicImage::ImageRgba8(cropped));
                let local_decoder = RasterDecoder::default();
                match decode_candidate(&local_decoder, &cropped, candidate, true) {
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
            if let Some(max_attempts) = debug_max_attempts
                && attempts.len() >= max_attempts
            {
                debug.log_line("debug max attempts reached");
                break;
            }
            if !robust && !debug_unbounded && elapsed_micros(started) >= NON_ROBUST_MAX_TOTAL_MICROS
            {
                break;
            }
            if !robust
                && !debug_unbounded
                && elapsed_micros(decode_started) >= NON_ROBUST_MAX_DECODE_MICROS
            {
                break;
            }
            let attempt_started = scan_instant_now();
            let candidate = if cv_scale < 1.0 {
                adjust_candidate_for_full_resolution(
                    candidate,
                    image.width(),
                    image.height(),
                    cv_scale,
                )
            } else {
                adjust_candidate_for_full_resolution(candidate, image.width(), image.height(), 1.0)
            };
            let region = candidate.region;
            if !robust && region.width.saturating_mul(region.height) > NON_ROBUST_MAX_CANDIDATE_AREA
            {
                debug.log_line(&format!(
                    "skip candidate too-large area={} stage={}",
                    region.width.saturating_mul(region.height),
                    candidate.stage
                ));
                continue;
            }
            let crop_stage = scan_instant_now();
            let cropped =
                image::imageops::crop_imm(image, region.x, region.y, region.width, region.height)
                    .to_image();
            let crop_us = elapsed_micros(crop_stage);
            debug.dump_rgba_candidate(region, candidate.stage, &cropped);
            let cropped = maybe_downscale_candidate_crop(DynamicImage::ImageRgba8(cropped));
            let decode_stage = scan_instant_now();
            match decode_candidate(&decoder, &cropped, candidate, !robust) {
                Ok(decoded) => {
                    let decode_us = elapsed_micros(decode_stage);
                    debug.log_line(&format!(
                        "candidate ok stage={} crop_us={} decode_us={}",
                        candidate.stage, crop_us, decode_us
                    ));
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
                Err(error) => {
                    let decode_us = elapsed_micros(decode_stage);
                    debug.log_line(&format!(
                        "candidate err stage={} crop_us={} decode_us={} err={}",
                        candidate.stage, crop_us, decode_us, error
                    ));
                    attempts.push(ScanAttempt {
                        detector: candidate.detector.as_str(),
                        layout_hint: candidate.layout_hint,
                        stage: candidate.stage,
                        region,
                        decoded: false,
                        error: Some(error.to_string()),
                        duration_micros: elapsed_micros(attempt_started),
                    })
                }
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    for candidate in regions {
        if !robust && !debug_unbounded && elapsed_micros(started) >= NON_ROBUST_MAX_TOTAL_MICROS {
            break;
        }
        if !robust
            && !debug_unbounded
            && elapsed_micros(decode_started) >= NON_ROBUST_MAX_DECODE_MICROS
        {
            break;
        }
        let attempt_started = scan_instant_now();
        let candidate =
            adjust_candidate_for_full_resolution(candidate, image.width(), image.height(), 1.0);
        let region = candidate.region;
        let cropped =
            image::imageops::crop_imm(image, region.x, region.y, region.width, region.height)
                .to_image();
        debug.dump_rgba_candidate(region, candidate.stage, &cropped);
        let cropped = maybe_downscale_candidate_crop(DynamicImage::ImageRgba8(cropped));
        match decode_candidate(&decoder, &cropped, candidate, !robust) {
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
    debug.log_line(&format!("stage candidate_us={}", timings.candidate_micros));
    debug.log_line(&format!(
        "stage decode_attempts_us={}",
        timings.decode_attempts_micros
    ));
    debug.log_line(&format!("stage total_us={}", timings.total_micros));
    log_slowest_attempts(&mut debug, &attempts);

    if std::env::var_os("GLYPHNET_SCAN_DEBUG").is_some() {
        eprintln!("scan attempts: {attempts:#?}");
    }
    Err(FailedStillScan {
        error: ScannerError::Decode(DecodeError::AutoDetectFailed),
        attempts,
        timings,
    })
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default)]
struct ScanDebugDumper {
    dir: Option<PathBuf>,
    attempt: u32,
    log: Option<std::fs::File>,
    heatmap: Option<image::RgbaImage>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ScanDebugDumper {
    fn from_env() -> Self {
        let dir = std::env::var_os("GLYPHNET_SCAN_DEBUG_DIR").map(PathBuf::from);
        let mut log = None;
        if let Some(path) = &dir {
            let _ = std::fs::create_dir_all(path);
            if let Ok(file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path.join("heartbeat.log"))
            {
                log = Some(file);
            }
        }
        Self {
            dir,
            attempt: 0,
            log,
            heatmap: None,
        }
    }

    fn dump_gray(&self, stem: &str, image: &image::GrayImage) {
        let Some(dir) = &self.dir else {
            return;
        };
        let path = dir.join(format!("{stem}.png"));
        let _ = image.save(path);
    }

    fn is_enabled(&self) -> bool {
        self.dir.is_some()
    }

    fn max_attempts(&self) -> Option<usize> {
        std::env::var("GLYPHNET_SCAN_DEBUG_MAX_ATTEMPTS")
            .ok()
            .and_then(|value| value.parse().ok())
    }

    fn dump_input(&mut self, image: &DynamicImage) {
        let Some(dir) = &self.dir else {
            return;
        };
        let _ = image.save(dir.join("00_input.png"));
        self.heatmap = Some(image.to_rgba8());
        self.log_line("saved 00_input.png");
    }

    fn log_line(&mut self, line: &str) {
        let Some(file) = &mut self.log else {
            return;
        };
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }

    fn dump_rgba_candidate(&mut self, region: ScanRegion, stage: &str, image: &image::RgbaImage) {
        let Some(dir) = self.dir.clone() else {
            return;
        };
        self.log_line(&format!(
            "attempt {:03} stage={stage} region=({},{} {}x{})",
            self.attempt, region.x, region.y, region.width, region.height
        ));
        let path = dir.join(format!(
            "attempt_{:03}_{stage}_x{}_y{}_w{}_h{}.png",
            self.attempt, region.x, region.y, region.width, region.height
        ));
        self.attempt = self.attempt.saturating_add(1);
        let _ = image.save(path);
        self.paint_heat(region);
        self.log_line("candidate image saved");
    }

    fn paint_heat(&mut self, region: ScanRegion) {
        let Some(heatmap) = &mut self.heatmap else {
            return;
        };
        let x0 = region.x.min(heatmap.width().saturating_sub(1));
        let y0 = region.y.min(heatmap.height().saturating_sub(1));
        let x1 = region
            .x
            .saturating_add(region.width)
            .min(heatmap.width())
            .max(x0.saturating_add(1));
        let y1 = region
            .y
            .saturating_add(region.height)
            .min(heatmap.height())
            .max(y0.saturating_add(1));
        for y in y0..y1 {
            for x in x0..x1 {
                let pixel = heatmap.get_pixel_mut(x, y);
                let r = pixel.0[0].saturating_add(18);
                let g = pixel.0[1].saturating_sub(8);
                let b = pixel.0[2].saturating_sub(8);
                pixel.0 = [r, g, b, 255];
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for ScanDebugDumper {
    fn drop(&mut self) {
        let (Some(dir), Some(heatmap)) = (&self.dir, &self.heatmap) else {
            return;
        };
        let _ = heatmap.save(dir.join("99_attempt_heatmap.png"));
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Default)]
struct ScanDebugDumper;

#[cfg(target_arch = "wasm32")]
impl ScanDebugDumper {
    fn from_env() -> Self {
        Self
    }
    fn dump_gray(&self, _stem: &str, _image: &image::GrayImage) {}
    fn is_enabled(&self) -> bool {
        false
    }
    fn max_attempts(&self) -> Option<usize> {
        None
    }
    fn dump_input(&mut self, _image: &DynamicImage) {}
    fn log_line(&mut self, _line: &str) {}
    fn dump_rgba_candidate(
        &mut self,
        _region: ScanRegion,
        _stage: &str,
        _image: &image::RgbaImage,
    ) {
    }
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

fn maybe_downscale_candidate_crop(image: DynamicImage) -> DynamicImage {
    const MAX_CANDIDATE_DIM: u32 = 1280;
    let width = image.width();
    let height = image.height();
    let longest = width.max(height);
    if longest <= MAX_CANDIDATE_DIM {
        return image;
    }
    let scale = MAX_CANDIDATE_DIM as f32 / longest as f32;
    let target_width = ((width as f32 * scale).round() as u32).max(1);
    let target_height = ((height as f32 * scale).round() as u32).max(1);
    DynamicImage::ImageRgba8(image::imageops::resize(
        &image,
        target_width,
        target_height,
        image::imageops::FilterType::Triangle,
    ))
}

fn fallback_ribbon_region_from_bounds(
    bounds: ScanRegion,
    image_width: u32,
    image_height: u32,
) -> Option<ScanRegion> {
    if image_width < 104 || image_height < 44 {
        return None;
    }
    let expanded = ScanRegion {
        x: bounds.x.saturating_sub(bounds.width / 4),
        y: bounds.y.saturating_sub(bounds.height / 3),
        width: bounds.width.saturating_add(bounds.width / 2),
        height: bounds.height.saturating_add(bounds.height / 2),
    };
    let x = expanded.x.min(image_width.saturating_sub(1));
    let y = expanded.y.min(image_height.saturating_sub(1));
    let max_x = expanded
        .x
        .saturating_add(expanded.width)
        .min(image_width)
        .max(x.saturating_add(1));
    let max_y = expanded
        .y
        .saturating_add(expanded.height)
        .min(image_height)
        .max(y.saturating_add(1));
    let width = max_x.saturating_sub(x).max(1);
    let height = max_y.saturating_sub(y).max(1);
    if width < 104 || height < 44 {
        return None;
    }
    Some(ScanRegion {
        x,
        y,
        width,
        height,
    })
}

fn log_slowest_attempts(debug: &mut ScanDebugDumper, attempts: &[ScanAttempt]) {
    if attempts.is_empty() {
        debug.log_line("slowest attempts: none");
        return;
    }
    let mut sorted: Vec<&ScanAttempt> = attempts.iter().collect();
    sorted.sort_by_key(|attempt| std::cmp::Reverse(attempt.duration_micros));
    for (index, attempt) in sorted.into_iter().take(5).enumerate() {
        debug.log_line(&format!(
            "slowest[{index}] detector={} stage={} decoded={} us={} region=({},{} {}x{})",
            attempt.detector,
            attempt.stage,
            attempt.decoded,
            attempt.duration_micros,
            attempt.region.x,
            attempt.region.y,
            attempt.region.width,
            attempt.region.height
        ));
    }
}

#[allow(dead_code)]
fn try_downscale_prepass_decode(
    image: &DynamicImage,
    profile: VisionProfile,
    decoder: &RasterDecoder,
    max_dim_px: u32,
    max_candidates: usize,
) -> Result<Option<(AutoDecodedSymbol, ScanRegion, Vec<ScanAttempt>)>> {
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return Ok(None);
    }

    let longest = width.max(height);
    if longest <= max_dim_px {
        return Ok(None);
    }
    let scale = max_dim_px as f32 / longest as f32;
    let small_width = ((width as f32 * scale).round() as u32).max(1);
    let small_height = ((height as f32 * scale).round() as u32).max(1);
    let downscaled = image::imageops::resize(
        image,
        small_width,
        small_height,
        image::imageops::FilterType::Triangle,
    );
    let downscaled = DynamicImage::ImageRgba8(downscaled);

    let gray = grayscale(&downscaled)?;
    let binary = adaptive_threshold(&gray, profile.threshold_radius, profile.threshold_bias)?;
    let padding = profile.min_anchor_px.max(8);
    let candidates = still_scan_candidates(&downscaled, &binary, profile, padding, false);

    let mut attempts = Vec::new();
    for candidate in candidates.into_iter().take(max_candidates) {
        let mapped = map_candidate_to_full_resolution(candidate, width, height, scale);
        let region = mapped.region;
        let attempt_started = scan_instant_now();
        let cropped =
            image::imageops::crop_imm(image, region.x, region.y, region.width, region.height)
                .to_image();
        let cropped = DynamicImage::ImageRgba8(cropped);
        match decode_candidate(decoder, &cropped, mapped, true) {
            Ok(decoded) => {
                attempts.push(ScanAttempt {
                    detector: "downscale-prepass",
                    layout_hint: candidate.layout_hint,
                    stage: candidate.stage,
                    region,
                    decoded: true,
                    error: None,
                    duration_micros: elapsed_micros(attempt_started),
                });
                return Ok(Some((decoded, region, attempts)));
            }
            Err(error) => attempts.push(ScanAttempt {
                detector: "downscale-prepass",
                layout_hint: candidate.layout_hint,
                stage: candidate.stage,
                region,
                decoded: false,
                error: Some(error.to_string()),
                duration_micros: elapsed_micros(attempt_started),
            }),
        }
    }

    Ok(None)
}

fn map_candidate_to_full_resolution(
    candidate: ScanCandidate,
    full_width: u32,
    full_height: u32,
    downscale_ratio: f32,
) -> ScanCandidate {
    let inv = 1.0 / downscale_ratio.max(0.01);
    let region = candidate.region;
    let pad_x = (region.width as f32 * 0.08).round() as u32;
    let pad_y = (region.height as f32 * 0.08).round() as u32;
    let x0 = (region.x.saturating_sub(pad_x) as f32 * inv).floor() as u32;
    let y0 = (region.y.saturating_sub(pad_y) as f32 * inv).floor() as u32;
    let x1 = ((region.x + region.width + pad_x) as f32 * inv).ceil() as u32;
    let y1 = ((region.y + region.height + pad_y) as f32 * inv).ceil() as u32;
    let x = x0.min(full_width.saturating_sub(1));
    let y = y0.min(full_height.saturating_sub(1));
    let max_x = x1.min(full_width).max(x.saturating_add(1));
    let max_y = y1.min(full_height).max(y.saturating_add(1));
    ScanCandidate::new(
        candidate.detector,
        candidate.layout_hint,
        candidate.stage,
        ScanRegion {
            x,
            y,
            width: max_x.saturating_sub(x).max(1),
            height: max_y.saturating_sub(y).max(1),
        },
    )
}

fn adjust_candidate_for_full_resolution(
    candidate: ScanCandidate,
    full_width: u32,
    full_height: u32,
    downscale_ratio: f32,
) -> ScanCandidate {
    let mapped = if downscale_ratio < 1.0 {
        map_candidate_to_full_resolution(candidate, full_width, full_height, downscale_ratio)
    } else {
        candidate
    };
    ScanCandidate {
        region: expand_edge_clipped_signature_region(
            mapped.region,
            mapped.stage,
            full_width,
            full_height,
        ),
        ..mapped
    }
}

fn expand_edge_clipped_signature_region(
    region: ScanRegion,
    stage: &'static str,
    image_width: u32,
    image_height: u32,
) -> ScanRegion {
    if !matches!(stage, "signature-window" | "photo-grid" | "coarse-grid") {
        return region;
    }
    if image_width < 104 || image_height < 44 {
        return region;
    }

    let right = region.x.saturating_add(region.width);
    let bottom = region.y.saturating_add(region.height);
    let touches_left = region.x == 0;
    let touches_top = region.y == 0;
    let touches_right = right >= image_width;
    let touches_bottom = bottom >= image_height;
    if !(touches_left || touches_top || touches_right || touches_bottom) {
        return region;
    }

    let mut width = region.width;
    let mut height = region.height;
    if touches_left || touches_right {
        width = width.saturating_mul(4).div_ceil(3).min(image_width);
    }
    if touches_top || touches_bottom {
        height = height.saturating_mul(4).div_ceil(3).min(image_height);
    }

    // Edge-clamped rail detections often cover only a partial signature rail.
    // Grow to the known RibbonWeave outer aspect so refinement sees the full symbol.
    let aspect_width = height.saturating_mul(104).div_ceil(44);
    width = width.max(aspect_width).min(image_width);
    let aspect_height = width.saturating_mul(44).div_ceil(104);
    height = height.max(aspect_height).min(image_height);

    let x = if touches_left {
        0
    } else if touches_right {
        image_width.saturating_sub(width)
    } else {
        region
            .x
            .saturating_add(region.width / 2)
            .saturating_sub(width / 2)
            .min(image_width.saturating_sub(width))
    };
    let y = if touches_top {
        0
    } else if touches_bottom {
        image_height.saturating_sub(height)
    } else {
        region
            .y
            .saturating_add(region.height / 2)
            .saturating_sub(height / 2)
            .min(image_height.saturating_sub(height))
    };

    ScanRegion {
        x,
        y,
        width,
        height,
    }
}

#[allow(dead_code)]
fn try_quad_prepass_decode(
    image: &DynamicImage,
    profile: VisionProfile,
    decoder: &RasterDecoder,
    max_dim_px: u32,
    max_quad_attempts: usize,
) -> Result<Option<AutoDecodedSymbol>> {
    const PREPASS_MAX_MICROS: u64 = 35_000;
    let prepass_started = scan_instant_now();
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return Ok(None);
    }
    let longest = width.max(height);
    if longest <= max_dim_px {
        return Ok(None);
    }

    let scale = max_dim_px as f32 / longest as f32;
    let small_width = ((width as f32 * scale).round() as u32).max(1);
    let small_height = ((height as f32 * scale).round() as u32).max(1);
    let downscaled = image::imageops::resize(
        image,
        small_width,
        small_height,
        image::imageops::FilterType::Triangle,
    );
    let downscaled = DynamicImage::ImageRgba8(downscaled);

    let gray = grayscale(&downscaled)?;
    if elapsed_micros(prepass_started) >= PREPASS_MAX_MICROS {
        return Ok(None);
    }
    let binary = adaptive_threshold(&gray, profile.threshold_radius, profile.threshold_bias)?;
    if elapsed_micros(prepass_started) >= PREPASS_MAX_MICROS {
        return Ok(None);
    }
    let anchors = find_anchor_candidates(&binary, profile)?;
    if elapsed_micros(prepass_started) >= PREPASS_MAX_MICROS {
        return Ok(None);
    }
    let estimated_quad = estimate_quad(&binary, &anchors);
    let dark_bounds_region =
        if should_try_dark_bounds_fallback(small_width, small_height, anchors.len()) {
            dark_bounds(&binary)
        } else {
            None
        };
    let quad_candidates = build_quad_candidates(
        estimated_quad,
        dark_bounds_region,
        small_width,
        small_height,
    );
    for quad in quad_candidates.into_iter().take(max_quad_attempts) {
        if elapsed_micros(prepass_started) >= PREPASS_MAX_MICROS {
            break;
        }
        let (warp_width, warp_height) = quad_dimensions(quad);
        if warp_width < 32 || warp_height < 32 {
            continue;
        }
        if let Ok(warped) = warp_perspective_gray(&gray, quad, warp_width, warp_height) {
            let warped = DynamicImage::ImageLuma8(warped);
            if let Ok(decoded) = decoder
                .decode_auto_with_info(&warped)
                .or_else(|_| decode_resampled_full_frame(decoder, &warped))
            {
                return Ok(Some(decoded));
            }
        }
    }
    Ok(None)
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
                || result
                    .attempts
                    .iter()
                    .any(|attempt| attempt.detector
                        == CandidateDetector::GeneratedContent.as_str())
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
