use glyphnet_core::{Frame, FrameHeader, HEADER_LEN, SymbolMatrix, bitstream};
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
    let sampled_bytes = bitstream::bits_to_bytes(&bits);
    let header = FrameHeader::decode(&sampled_bytes)?;
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
