use glyphnet_core::{
    EccLevel, Frame, FrameHeader, GlyphError, HEADER_LEN, MAGIC, SymbolMatrix, TransmissionMode,
    WIRE_VERSION, bitstream,
};
use glyphnet_ecc::{
    RecoveryMethod, RecoveryTelemetry, try_recover_for_mode_with_suspects_and_telemetry,
    verify_for_mode,
};

use crate::{DecodeError, DecodedSymbol, Result};

pub(crate) const RECOVERY_MAX_ATTEMPTS: usize = 256;

pub(crate) fn decode_matrix_with_suspects(
    matrix: &SymbolMatrix,
    suspect_bytes: &[usize],
) -> Result<DecodedSymbol> {
    let bits = matrix.read_data_bits();
    let mut sampled_bytes = bitstream::bits_to_bytes(&bits);
    let header = match FrameHeader::decode(&sampled_bytes) {
        Ok(header) => header,
        Err(GlyphError::HeaderChecksumMismatch) => {
            let header = decode_header_fields_without_crc(&sampled_bytes)?;
            sampled_bytes[28..32].copy_from_slice(&header.header_crc.to_be_bytes());
            header
        }
        Err(error) => return Err(error.into()),
    };
    let data_len = HEADER_LEN + header.payload_len as usize;
    if verify_for_mode(header.mode, header.ecc_level, &sampled_bytes, data_len) {
        let frame = Frame::decode(&sampled_bytes)?;
        return Ok(DecodedSymbol {
            matrix: matrix.clone(),
            frame,
            sampled_bytes,
            recovery: RecoveryTelemetry {
                attempted: false,
                recovered: false,
                attempts: 0,
                method: RecoveryMethod::None,
                suspect_count: suspect_bytes.len(),
                max_attempts_exceeded: false,
            },
        });
    }

    let max_attempts = if suspect_bytes.is_empty() {
        usize::MAX
    } else {
        RECOVERY_MAX_ATTEMPTS
    };
    let (recovered, recovery) = try_recover_for_mode_with_suspects_and_telemetry(
        header.mode,
        header.ecc_level,
        &sampled_bytes,
        data_len,
        suspect_bytes,
        max_attempts,
    );
    if let Some(recovered_bytes) = recovered
        && verify_for_mode(header.mode, header.ecc_level, &recovered_bytes, data_len)
    {
        let recovered_frame = Frame::decode(&recovered_bytes)?;
        return Ok(DecodedSymbol {
            matrix: matrix.clone(),
            frame: recovered_frame,
            sampled_bytes: recovered_bytes,
            recovery,
        });
    }

    Err(DecodeError::EccMismatch)
}

fn decode_header_fields_without_crc(bytes: &[u8]) -> std::result::Result<FrameHeader, GlyphError> {
    if bytes.len() < HEADER_LEN {
        return Err(GlyphError::Truncated {
            needed: HEADER_LEN,
            actual: bytes.len(),
        });
    }
    if bytes[0..4] != MAGIC {
        return Err(GlyphError::InvalidMagic);
    }
    if bytes[4] != WIRE_VERSION {
        return Err(GlyphError::UnsupportedVersion(bytes[4]));
    }
    let mode = TransmissionMode::from_wire(bytes[5])?;
    let ecc_level = EccLevel::from_wire(bytes[6])?;
    let frame_index = u16::from_be_bytes([bytes[8], bytes[9]]);
    let frame_count = u16::from_be_bytes([bytes[10], bytes[11]]);
    let stream_id = u64::from_be_bytes([
        bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18], bytes[19],
    ]);
    let payload_len = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    let payload_crc = u32::from_be_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    FrameHeader::new(
        mode,
        ecc_level,
        frame_index,
        frame_count,
        stream_id,
        payload_len,
        payload_crc,
    )
}
