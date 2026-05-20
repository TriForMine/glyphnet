use serde::{Deserialize, Serialize};

use crate::{EccLevel, TransmissionMode};

/// Semantic protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolVersion {
    /// Major version.
    pub major: u8,
    /// Minor version.
    pub minor: u8,
    /// Patch version.
    pub patch: u8,
}

impl ProtocolVersion {
    /// Current protocol version implemented by this workspace.
    pub const CURRENT: Self = Self {
        major: 0,
        minor: 1,
        patch: 0,
    };
}

/// Logical symbol layout family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutFamily {
    /// Continuous ribbon lanes with side totems and chevron signatures; the default layout.
    RibbonWeave,
    /// Color-calibrated interleaved ribbon mesh for screens.
    SpectralMesh,
    /// Wide temporal ribbon strips for animated optical transmission.
    PulseStream,
    /// Non-corner halo anchors with diagonal timing spines.
    Constellation,
    /// Asymmetric rectangular frame grid retained for earlier v0 experiments.
    FrameGrid,
    /// Square-compatible matrix grid retained for baseline compatibility tests.
    Matrix,
    /// Hexagonal module packing reserved for future high-density screen mode.
    Hexagonal,
    /// Radial rings reserved for lens-distortion tolerant symbols.
    Radial,
}

/// Color modulation strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorEncoding {
    /// Monochrome dark/light modules.
    Mono,
    /// Calibrated limited palette for print or e-ink.
    LimitedPalette,
    /// RGB subchannel modulation for high-quality displays.
    Rgb,
    /// Per-frame color calibration in burst transfers.
    Adaptive,
}

/// Optional implementation capability advertised by an encoder or scanner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    /// Static print-mode symbols.
    PrintMode,
    /// Static screen-mode symbols.
    ScreenMode,
    /// Animated burst-mode streams.
    BurstMode,
    /// Color module support.
    Color,
    /// Camera/display calibration support.
    Calibration,
    /// GPU accelerated sampling or rendering.
    GpuAcceleration,
    /// Temporal fountain-style recovery.
    FountainRecovery,
}

/// Compact capability set used for negotiation and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CapabilitySet {
    capabilities: Vec<Capability>,
}

impl CapabilitySet {
    /// Create an empty capability set.
    pub const fn new() -> Self {
        Self {
            capabilities: Vec::new(),
        }
    }

    /// Create a capability set from an iterator.
    pub fn from_iterable(values: impl IntoIterator<Item = Capability>) -> Self {
        let mut set = Self::new();
        for capability in values {
            set.insert(capability);
        }
        set
    }

    /// Insert a capability if it is not already present.
    pub fn insert(&mut self, capability: Capability) {
        if !self.capabilities.contains(&capability) {
            self.capabilities.push(capability);
        }
    }

    /// Test whether a capability is present.
    pub fn contains(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Iterate over the contained capabilities.
    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.capabilities.iter().copied()
    }
}

impl FromIterator<Capability> for CapabilitySet {
    fn from_iter<T: IntoIterator<Item = Capability>>(iter: T) -> Self {
        Self::from_iterable(iter)
    }
}

/// Human and machine readable description of an encoded symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolDescriptor {
    /// Protocol version used to encode the symbol.
    pub version: ProtocolVersion,
    /// Transmission mode.
    pub mode: TransmissionMode,
    /// Error correction level.
    pub ecc_level: EccLevel,
    /// Geometric layout family.
    pub layout: LayoutFamily,
    /// Color encoding profile.
    pub color: ColorEncoding,
    /// Symbol width in modules.
    pub width: u16,
    /// Symbol height in modules.
    pub height: u16,
    /// Payload length before ECC bytes.
    pub payload_len: usize,
    /// Stream identifier for burst or multi-frame payloads.
    pub stream_id: u64,
    /// Zero-based frame index.
    pub frame_index: u16,
    /// Total frame count.
    pub frame_count: u16,
    /// Data-module capacity in bits.
    pub data_capacity_bits: usize,
    /// Optional features required to decode this symbol.
    pub capabilities: CapabilitySet,
}
