use glyphnet_core::LayoutFamily;
use glyphnet_decode::AutoDecodedSymbol;

use crate::{ScanRegion, ScannerError};

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
