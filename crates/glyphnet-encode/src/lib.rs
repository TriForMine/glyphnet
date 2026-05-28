//! Static and animated GlyphNet encoders.

use blake3::Hash;
use glyphnet_core::{
    Capability, CapabilitySet, ColorEncoding, EccLevel, Frame, GlyphError, LayoutFamily, ProfileId,
    ProtocolVersion, SymbolDescriptor, SymbolGeometry, SymbolMatrix, TransmissionMode, bitstream,
    choose_symbol_geometry, profile_spec,
};
use glyphnet_ecc::encode_for_mode;
use thiserror::Error;

/// Result type for encoder operations.
pub type Result<T> = std::result::Result<T, EncodeError>;

/// Encoder errors.
#[derive(Debug, Error)]
pub enum EncodeError {
    /// Wrapped core error.
    #[error(transparent)]
    Core(#[from] GlyphError),
}

/// Encoder configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderConfig {
    /// Transmission mode.
    pub mode: TransmissionMode,
    /// Error-correction level.
    pub ecc_level: EccLevel,
    /// Layout family.
    pub layout: LayoutFamily,
    /// Color encoding profile.
    pub color: ColorEncoding,
    /// Maximum raw payload bytes per burst frame.
    pub max_frame_payload: usize,
    /// Optional explicit module geometry override.
    pub geometry: Option<SymbolGeometry>,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            mode: TransmissionMode::Print,
            ecc_level: EccLevel::High,
            layout: LayoutFamily::RibbonWeave,
            color: ColorEncoding::Mono,
            max_frame_payload: 512,
            geometry: None,
        }
    }
}

impl EncoderConfig {
    /// Build an encoder configuration from a named protocol profile.
    pub fn for_profile(profile: ProfileId) -> Self {
        let spec = profile_spec(profile);
        Self {
            mode: spec.mode,
            ecc_level: spec.ecc_level,
            layout: spec.layout,
            color: spec.color,
            max_frame_payload: spec.max_frame_payload,
            geometry: None,
        }
    }
}

/// Encoded visual symbol and metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedSymbol {
    /// Machine-readable descriptor.
    pub descriptor: SymbolDescriptor,
    /// Module matrix ready for rendering.
    pub matrix: SymbolMatrix,
    /// Binary frame before ECC padding.
    pub frame: Frame,
    /// Wire bytes plus parity bytes.
    pub codewords: Vec<u8>,
}

/// Scheduled burst transmission entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BurstScheduleEntry {
    /// Zero-based transmit slot.
    pub slot: usize,
    /// Zero-based redundancy pass.
    pub pass: u16,
    /// Zero-based frame index in the source burst set.
    pub frame_index: u16,
    /// Encoded frame to render in this slot.
    pub symbol: EncodedSymbol,
}

/// Burst sender scheduling options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BurstScheduleConfig {
    /// Number of full round-robin passes over all burst frames.
    pub passes: u16,
}

impl Default for BurstScheduleConfig {
    fn default() -> Self {
        Self { passes: 1 }
    }
}

/// Reference encoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoder {
    config: EncoderConfig,
}

impl Encoder {
    /// Create an encoder from explicit configuration.
    pub const fn new(config: EncoderConfig) -> Self {
        Self { config }
    }

    /// Borrow the encoder configuration.
    pub const fn config(&self) -> &EncoderConfig {
        &self.config
    }

    /// Encode a complete static symbol.
    pub fn encode_static(&self, payload: &[u8]) -> Result<EncodedSymbol> {
        self.encode_frame(payload, 0, 1, stream_id(payload), self.config.mode)
    }

    /// Encode a single frame with explicit burst metadata.
    pub fn encode_frame(
        &self,
        payload: &[u8],
        frame_index: u16,
        frame_count: u16,
        stream_id: u64,
        mode: TransmissionMode,
    ) -> Result<EncodedSymbol> {
        let frame = Frame::new(
            mode,
            self.config.ecc_level,
            frame_index,
            frame_count,
            stream_id,
            payload.to_vec(),
        )?;
        let wire = frame.encode();
        let codewords = encode_for_mode(mode, self.config.ecc_level, &wire);
        let bits = bitstream::bytes_to_bits(&codewords);
        let geometry = if let Some(geometry) = self.config.geometry {
            geometry
        } else {
            choose_symbol_geometry(mode, self.config.layout, bits.len())?
        };
        if geometry.width == 0 || geometry.height == 0 {
            return Err(GlyphError::InvalidArgument("symbol geometry must be non-zero").into());
        }
        let mut matrix =
            SymbolMatrix::with_layout(geometry.width, geometry.height, self.config.layout);
        let capacity = matrix.data_capacity_bits();
        if bits.len() > capacity {
            return Err(GlyphError::CapacityExceeded {
                needed_bits: bits.len(),
                available_bits: capacity,
            }
            .into());
        }
        matrix.write_data_bits(bits);

        let descriptor = SymbolDescriptor {
            version: ProtocolVersion::CURRENT,
            mode,
            ecc_level: self.config.ecc_level,
            layout: self.config.layout,
            color: self.config.color,
            width: matrix.width(),
            height: matrix.height(),
            payload_len: payload.len(),
            stream_id,
            frame_index,
            frame_count,
            data_capacity_bits: capacity,
            capabilities: capabilities_for(mode, self.config.color),
        };

        Ok(EncodedSymbol {
            descriptor,
            matrix,
            frame,
            codewords,
        })
    }

    /// Encode payload bytes as a sequence of burst frames.
    pub fn encode_burst(&self, payload: &[u8]) -> Result<Vec<EncodedSymbol>> {
        let chunk_size = self.config.max_frame_payload.max(1);
        let frame_count = payload.len().div_ceil(chunk_size).max(1);
        if frame_count > usize::from(u16::MAX) {
            return Err(
                GlyphError::InvalidArgument("payload requires too many burst frames").into(),
            );
        }

        let id = stream_id(payload);
        let mut frames = Vec::with_capacity(frame_count);
        for frame_index in 0..frame_count {
            let start = frame_index * chunk_size;
            let end = ((frame_index + 1) * chunk_size).min(payload.len());
            frames.push(self.encode_frame(
                &payload[start..end],
                frame_index as u16,
                frame_count as u16,
                id,
                TransmissionMode::Burst,
            )?);
        }
        Ok(frames)
    }

    /// Encode and schedule burst frames for one-way repeated transmission.
    pub fn encode_burst_schedule(
        &self,
        payload: &[u8],
        schedule: BurstScheduleConfig,
    ) -> Result<Vec<BurstScheduleEntry>> {
        if schedule.passes == 0 {
            return Err(GlyphError::InvalidArgument("burst schedule passes must be >= 1").into());
        }
        let frames = self.encode_burst(payload)?;
        let frame_count = frames.len();
        let mut entries = Vec::with_capacity(frame_count * usize::from(schedule.passes));
        let mut slot = 0usize;
        for pass in 0..schedule.passes {
            for (frame_index, symbol) in frames.iter().cloned().enumerate() {
                entries.push(BurstScheduleEntry {
                    slot,
                    pass,
                    frame_index: frame_index as u16,
                    symbol,
                });
                slot += 1;
            }
        }
        Ok(entries)
    }
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new(EncoderConfig::default())
    }
}

/// Compute the deterministic stream identifier for payload bytes.
pub fn stream_id(payload: &[u8]) -> u64 {
    stream_id_from_hash(blake3::hash(payload))
}

fn stream_id_from_hash(hash: Hash) -> u64 {
    let bytes: [u8; 8] = hash.as_bytes()[..8]
        .try_into()
        .expect("BLAKE3 hashes are at least eight bytes");
    u64::from_be_bytes(bytes)
}

fn capabilities_for(mode: TransmissionMode, color: ColorEncoding) -> CapabilitySet {
    let mut capabilities = CapabilitySet::new();
    match mode {
        TransmissionMode::Print => capabilities.insert(Capability::PrintMode),
        TransmissionMode::Screen => capabilities.insert(Capability::ScreenMode),
        TransmissionMode::Burst => capabilities.insert(Capability::BurstMode),
    }
    if !matches!(color, ColorEncoding::Mono) {
        capabilities.insert(Capability::Color);
    }
    if matches!(mode, TransmissionMode::Burst) {
        capabilities.insert(Capability::FountainRecovery);
    }
    capabilities
}

#[cfg(test)]
mod tests {
    use glyphnet_core::{Frame, SymbolGeometry};

    use super::*;

    #[test]
    fn static_encoder_produces_decodable_wire_prefix() {
        let encoded = Encoder::default().encode_static(b"hello").unwrap();
        assert_eq!(
            Frame::decode(&encoded.codewords).unwrap().payload,
            b"hello".to_vec()
        );
        assert_ne!(encoded.descriptor.width, encoded.descriptor.height);
        assert!(encoded.descriptor.data_capacity_bits >= encoded.codewords.len() * 8);
    }

    #[test]
    fn burst_encoder_chunks_payload() {
        let encoder = Encoder::new(EncoderConfig {
            mode: TransmissionMode::Burst,
            max_frame_payload: 3,
            ..EncoderConfig::default()
        });
        let frames = encoder.encode_burst(b"abcdefg").unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].descriptor.frame_count, 3);
        assert_eq!(frames[2].frame.payload, b"g");
    }

    #[test]
    fn custom_geometry_overrides_default() {
        let encoder = Encoder::new(EncoderConfig {
            geometry: Some(SymbolGeometry::new(80, 80)),
            ..EncoderConfig::default()
        });
        let encoded = encoder.encode_static(b"hello").unwrap();
        assert_eq!(encoded.descriptor.width, 80);
        assert_eq!(encoded.descriptor.height, 80);
    }

    #[test]
    fn burst_schedule_round_robin_repeats_all_frames() {
        let encoder = Encoder::new(EncoderConfig {
            mode: TransmissionMode::Burst,
            max_frame_payload: 2,
            ..EncoderConfig::default()
        });
        let schedule = encoder
            .encode_burst_schedule(b"abcdef", BurstScheduleConfig { passes: 2 })
            .unwrap();

        assert_eq!(schedule.len(), 6);
        assert_eq!(schedule[0].pass, 0);
        assert_eq!(schedule[0].frame_index, 0);
        assert_eq!(schedule[1].frame_index, 1);
        assert_eq!(schedule[2].frame_index, 2);
        assert_eq!(schedule[3].pass, 1);
        assert_eq!(schedule[3].frame_index, 0);
        assert_eq!(schedule[5].frame_index, 2);
    }

    #[test]
    fn burst_schedule_rejects_zero_passes() {
        let encoder = Encoder::new(EncoderConfig {
            mode: TransmissionMode::Burst,
            ..EncoderConfig::default()
        });
        let err = encoder
            .encode_burst_schedule(b"abc", BurstScheduleConfig { passes: 0 })
            .unwrap_err();
        assert!(matches!(
            err,
            EncodeError::Core(GlyphError::InvalidArgument(_))
        ));
    }
}
