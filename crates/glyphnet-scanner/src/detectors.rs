use glyphnet_core::LayoutFamily;

use crate::ScanRegion;

/// Candidate detector family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateDetector {
    /// Clean rendered symbol on a simple background.
    GeneratedContent,
    /// Layout-agnostic dark-component and band detector.
    GenericBinary,
    /// Matrix finder-pattern detector.
    Matrix,
    /// RibbonWeave rail, totem, and wide-symbol recovery detector.
    RibbonWeave,
}

impl CandidateDetector {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::GeneratedContent => "generated-content",
            Self::GenericBinary => "generic-binary",
            Self::Matrix => "matrix",
            Self::RibbonWeave => "ribbon-weave",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScanCandidate {
    pub(crate) detector: CandidateDetector,
    pub(crate) layout_hint: Option<LayoutFamily>,
    pub(crate) stage: &'static str,
    pub(crate) region: ScanRegion,
}

impl ScanCandidate {
    pub(crate) const fn new(
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

pub(crate) fn push_unique_candidate(
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
