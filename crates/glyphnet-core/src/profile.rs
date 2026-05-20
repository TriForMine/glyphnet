use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{ColorEncoding, EccLevel, GlyphError, LayoutFamily, Result, TransmissionMode};

/// Named protocol profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileId {
    /// General physical print profile with a distinctive ribbon-weave visual form.
    RibbonPrint,
    /// Color display profile for dense screen-to-camera transfer.
    SpectralScreen,
    /// High-speed animated optical transfer profile.
    PulseBurst,
    /// Robust label/card profile based on off-corner anchors.
    ConstellationPrint,
    /// Square matrix compatibility baseline for benchmarks only.
    MatrixCompat,
}

impl ProfileId {
    /// Stable kebab-case identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RibbonPrint => "ribbon-print",
            Self::SpectralScreen => "spectral-screen",
            Self::PulseBurst => "pulse-burst",
            Self::ConstellationPrint => "constellation-print",
            Self::MatrixCompat => "matrix-compat",
        }
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProfileId {
    type Err = GlyphError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "ribbon-print" => Ok(Self::RibbonPrint),
            "spectral-screen" => Ok(Self::SpectralScreen),
            "pulse-burst" => Ok(Self::PulseBurst),
            "constellation-print" => Ok(Self::ConstellationPrint),
            "matrix-compat" => Ok(Self::MatrixCompat),
            _ => Err(GlyphError::InvalidArgument("unknown profile id")),
        }
    }
}

/// Intended use case for a protocol profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UseCase {
    /// Paper, stickers, cards, posters, and packaging.
    Print,
    /// Emissive displays scanned by phones/webcams.
    Screen,
    /// Animated high-speed optical transfer.
    Burst,
    /// Compatibility and benchmark baseline.
    Baseline,
}

/// Benchmark objective for a profile.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkTarget {
    /// Payload size used by baseline benchmark vectors.
    pub payload_bytes: usize,
    /// Target decode success rate under the profile's degradation suite.
    pub decode_success_rate: f64,
    /// Target scanner-side decode budget in milliseconds per frame.
    pub max_decode_ms: f64,
    /// Target optical throughput in bytes per second.
    pub target_bytes_per_second: usize,
}

/// Complete profile specification.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProfileSpec {
    /// Stable identifier.
    pub id: ProfileId,
    /// Human-readable name.
    pub name: &'static str,
    /// Main use case.
    pub use_case: UseCase,
    /// Transmission mode.
    pub mode: TransmissionMode,
    /// Visual layout family.
    pub layout: LayoutFamily,
    /// Color modulation.
    pub color: ColorEncoding,
    /// ECC level.
    pub ecc_level: EccLevel,
    /// Maximum payload bytes per burst frame.
    pub max_frame_payload: usize,
    /// Suggested display frame rate for animated profiles.
    pub target_fps: Option<u16>,
    /// Short implementation note.
    pub note: &'static str,
    /// Benchmark target.
    pub benchmark: BenchmarkTarget,
}

/// Return the static profile catalog.
pub const fn profile_catalog() -> &'static [ProfileSpec] {
    &PROFILE_CATALOG
}

/// Look up a profile spec.
pub fn profile_spec(id: ProfileId) -> ProfileSpec {
    PROFILE_CATALOG[id as usize]
}

const PROFILE_CATALOG: [ProfileSpec; 5] = [
    ProfileSpec {
        id: ProfileId::RibbonPrint,
        name: "Ribbon Print",
        use_case: UseCase::Print,
        mode: TransmissionMode::Print,
        layout: LayoutFamily::RibbonWeave,
        color: ColorEncoding::Mono,
        ecc_level: EccLevel::High,
        max_frame_payload: 512,
        target_fps: None,
        note: "Default print profile; continuous ribbon strokes avoid QR/Data Matrix confusion.",
        benchmark: BenchmarkTarget {
            payload_bytes: 256,
            decode_success_rate: 0.995,
            max_decode_ms: 25.0,
            target_bytes_per_second: 0,
        },
    },
    ProfileSpec {
        id: ProfileId::SpectralScreen,
        name: "Spectral Screen",
        use_case: UseCase::Screen,
        mode: TransmissionMode::Screen,
        layout: LayoutFamily::SpectralMesh,
        color: ColorEncoding::Rgb,
        ecc_level: EccLevel::Medium,
        max_frame_payload: 1024,
        target_fps: Some(30),
        note: "Color-calibrated screen profile with interleaved colored ribbon lanes.",
        benchmark: BenchmarkTarget {
            payload_bytes: 1024,
            decode_success_rate: 0.99,
            max_decode_ms: 16.0,
            target_bytes_per_second: 30_000,
        },
    },
    ProfileSpec {
        id: ProfileId::PulseBurst,
        name: "Pulse Burst",
        use_case: UseCase::Burst,
        mode: TransmissionMode::Burst,
        layout: LayoutFamily::PulseStream,
        color: ColorEncoding::Adaptive,
        ecc_level: EccLevel::Adaptive,
        max_frame_payload: 1400,
        target_fps: Some(60),
        note: "Animated high-throughput profile for video/display optical streams.",
        benchmark: BenchmarkTarget {
            payload_bytes: 64 * 1024,
            decode_success_rate: 0.985,
            max_decode_ms: 10.0,
            target_bytes_per_second: 84_000,
        },
    },
    ProfileSpec {
        id: ProfileId::ConstellationPrint,
        name: "Constellation Print",
        use_case: UseCase::Print,
        mode: TransmissionMode::Print,
        layout: LayoutFamily::Constellation,
        color: ColorEncoding::LimitedPalette,
        ecc_level: EccLevel::High,
        max_frame_payload: 512,
        target_fps: None,
        note: "Experimental robust print profile with off-corner halo anchors.",
        benchmark: BenchmarkTarget {
            payload_bytes: 384,
            decode_success_rate: 0.997,
            max_decode_ms: 30.0,
            target_bytes_per_second: 0,
        },
    },
    ProfileSpec {
        id: ProfileId::MatrixCompat,
        name: "Matrix Compatibility Baseline",
        use_case: UseCase::Baseline,
        mode: TransmissionMode::Print,
        layout: LayoutFamily::Matrix,
        color: ColorEncoding::Mono,
        ecc_level: EccLevel::High,
        max_frame_payload: 512,
        target_fps: None,
        note: "Square matrix baseline used only for compatibility and comparative benchmarks.",
        benchmark: BenchmarkTarget {
            payload_bytes: 256,
            decode_success_rate: 0.995,
            max_decode_ms: 20.0,
            target_bytes_per_second: 0,
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_ids_parse_from_catalog_names() {
        for profile in profile_catalog() {
            assert_eq!(
                profile.id.as_str().parse::<ProfileId>().unwrap(),
                profile.id
            );
        }
    }

    #[test]
    fn burst_profile_has_throughput_target() {
        let profile = profile_spec(ProfileId::PulseBurst);
        assert_eq!(profile.mode, TransmissionMode::Burst);
        assert!(profile.target_fps.unwrap() >= 60);
        assert!(profile.benchmark.target_bytes_per_second > 0);
    }
}
