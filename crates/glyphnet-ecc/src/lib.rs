//! Error-correction and erasure-recovery primitives.
//!
//! The reference implementation starts with deterministic parity and XOR
//! erasure recovery so the protocol stack has stable behavior and tests today.
//! The crate boundary is designed to host LDPC, fountain, and RaptorQ-like
//! implementations without changing encoder/decoder APIs.

use glyphnet_core::{EccLevel, Frame, GlyphError, TransmissionMode};
use reed_solomon_erasure::galois_8::ReedSolomon;
use thiserror::Error;

/// Result type for ECC operations.
pub type Result<T> = std::result::Result<T, EccError>;

/// ECC-specific errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EccError {
    /// A caller requested an impossible shard layout.
    #[error("invalid shard configuration")]
    InvalidShardConfiguration,
    /// The configured recovery code cannot recover the requested erasure set.
    #[error("too many erasures to recover")]
    TooManyErasures,
    /// Wrapped core protocol error.
    #[error(transparent)]
    Core(#[from] GlyphError),
}

/// Block-code interface used by static encoders and decoders.
pub trait BlockCode {
    /// Append parity/redundancy bytes to a data block.
    fn encode(&self, data: &[u8]) -> Vec<u8>;

    /// Validate a data block that already includes parity bytes.
    fn verify(&self, encoded: &[u8], data_len: usize) -> bool;
}

/// Concrete ECC scheme used for a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EccScheme {
    /// Deterministic parity reference implementation.
    Parity,
    /// Reed-Solomon shard parity for print-mode robustness.
    ReedSolomon,
    /// LDPC-style profile wiring for screen-mode payloads.
    ///
    /// Current implementation keeps parity-compatible semantics while the
    /// dedicated LDPC codec is integrated.
    #[cfg(feature = "ldpc")]
    Ldpc,
}

/// Recovery strategy used by ECC repair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryMethod {
    /// No repair strategy was applied.
    None,
    /// Reed-Solomon single-byte erasure reconstruction.
    ReedSolomonSingle,
    /// Reed-Solomon two-byte erasure reconstruction.
    ReedSolomonPair,
    /// Rebuilt parity tail from data bytes.
    ParityTailRebuild,
    /// Brute-force parity-guided data-byte mutation.
    ParityByteSearch,
}

/// Recovery telemetry emitted by ECC repair paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryTelemetry {
    /// Whether ECC recovery logic was attempted.
    pub attempted: bool,
    /// Whether recovery succeeded and produced a valid frame.
    pub recovered: bool,
    /// Number of candidate mutations/reconstructions evaluated.
    pub attempts: usize,
    /// Recovery strategy used by the successful candidate.
    pub method: RecoveryMethod,
    /// Number of suspect byte indexes provided by caller.
    pub suspect_count: usize,
    /// Whether recovery stopped because `max_attempts` was exceeded.
    pub max_attempts_exceeded: bool,
}

/// Deterministic parity bytes used by the reference encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParityCode {
    parity_len: usize,
}

impl ParityCode {
    /// Create a parity code with an explicit parity byte count.
    pub const fn new(parity_len: usize) -> Self {
        Self { parity_len }
    }

    /// Select a parity code from a protocol ECC level and data length.
    pub fn from_level(level: EccLevel, data_len: usize) -> Self {
        let (num, den) = level.parity_ratio();
        let parity_len = ((data_len * num).div_ceil(den)).max(4);
        Self::new(parity_len)
    }

    /// Number of parity bytes produced by this code.
    pub const fn parity_len(self) -> usize {
        self.parity_len
    }

    fn parity_bytes(self, data: &[u8]) -> Vec<u8> {
        let mut parity = vec![0u8; self.parity_len];
        if parity.is_empty() {
            return parity;
        }

        for (index, byte) in data.iter().enumerate() {
            let lane = index % parity.len();
            let rotated = byte.rotate_left((index % 8) as u32);
            parity[lane] ^= rotated;
            parity[(lane * 7 + 3) % self.parity_len] =
                parity[(lane * 7 + 3) % self.parity_len].wrapping_add(*byte);
        }
        parity
    }
}

impl BlockCode for ParityCode {
    fn encode(&self, data: &[u8]) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(data.len() + self.parity_len);
        encoded.extend_from_slice(data);
        encoded.extend_from_slice(&self.parity_bytes(data));
        encoded
    }

    fn verify(&self, encoded: &[u8], data_len: usize) -> bool {
        if encoded.len() < data_len + self.parity_len {
            return false;
        }
        let expected = self.parity_bytes(&encoded[..data_len]);
        encoded[data_len..data_len + self.parity_len] == expected
    }
}

/// Reed-Solomon parity bytes over one-byte shards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReedSolomonCode {
    parity_shards: usize,
}

impl ReedSolomonCode {
    /// Create a Reed-Solomon code with explicit parity shard count.
    pub const fn new(parity_shards: usize) -> Self {
        Self { parity_shards }
    }

    /// Select a parity shard count from ECC level and payload length.
    pub fn from_level(level: EccLevel, data_len: usize) -> Self {
        let (num, den) = level.parity_ratio();
        let parity_shards = ((data_len * num).div_ceil(den)).max(8);
        Self::new(parity_shards)
    }

    fn is_supported(self, data_len: usize) -> bool {
        data_len > 0 && data_len.saturating_add(self.parity_shards) <= 255
    }

    fn parity_bytes(self, data: &[u8]) -> Option<Vec<u8>> {
        if data.is_empty() || self.parity_shards == 0 {
            return Some(Vec::new());
        }
        let data_shards = data.len();
        let rs = ReedSolomon::new(data_shards, self.parity_shards).ok()?;
        let mut shards: Vec<Vec<u8>> = data.iter().map(|byte| vec![*byte]).collect();
        shards.extend((0..self.parity_shards).map(|_| vec![0u8]));
        let mut refs: Vec<_> = shards.iter_mut().map(Vec::as_mut_slice).collect();
        rs.encode(&mut refs).ok()?;
        Some(
            refs[data_shards..]
                .iter()
                .map(|shard| shard[0])
                .collect::<Vec<u8>>(),
        )
    }

    fn recover_one_data_shard(
        self,
        encoded: &[u8],
        data_len: usize,
        missing_index: usize,
    ) -> Option<Vec<u8>> {
        if missing_index >= data_len || encoded.len() < data_len + self.parity_shards {
            return None;
        }
        if !self.is_supported(data_len) {
            return None;
        }
        let rs = ReedSolomon::new(data_len, self.parity_shards).ok()?;
        let mut shards: Vec<Option<Vec<u8>>> = Vec::with_capacity(data_len + self.parity_shards);
        for (index, value) in encoded.iter().enumerate().take(data_len) {
            if index == missing_index {
                shards.push(None);
            } else {
                shards.push(Some(vec![*value]));
            }
        }
        for index in 0..self.parity_shards {
            shards.push(Some(vec![encoded[data_len + index]]));
        }
        rs.reconstruct(&mut shards).ok()?;
        let recovered_data_byte = shards[missing_index].as_ref()?.first().copied()?;
        let mut recovered = encoded.to_vec();
        recovered[missing_index] = recovered_data_byte;
        Some(recovered)
    }

    fn recover_data_shards(
        self,
        encoded: &[u8],
        data_len: usize,
        missing_indexes: &[usize],
    ) -> Option<Vec<u8>> {
        if missing_indexes.is_empty()
            || missing_indexes.iter().any(|&index| index >= data_len)
            || encoded.len() < data_len + self.parity_shards
        {
            return None;
        }
        if !self.is_supported(data_len) || missing_indexes.len() > self.parity_shards {
            return None;
        }
        let rs = ReedSolomon::new(data_len, self.parity_shards).ok()?;
        let mut missing = vec![false; data_len];
        for &index in missing_indexes {
            missing[index] = true;
        }
        let mut shards: Vec<Option<Vec<u8>>> = Vec::with_capacity(data_len + self.parity_shards);
        for (index, value) in encoded.iter().enumerate().take(data_len) {
            if missing[index] {
                shards.push(None);
            } else {
                shards.push(Some(vec![*value]));
            }
        }
        for index in 0..self.parity_shards {
            shards.push(Some(vec![encoded[data_len + index]]));
        }
        rs.reconstruct(&mut shards).ok()?;
        let mut recovered = encoded.to_vec();
        for &missing_index in missing_indexes {
            recovered[missing_index] = shards[missing_index].as_ref()?.first().copied()?;
        }
        Some(recovered)
    }
}

impl BlockCode for ReedSolomonCode {
    fn encode(&self, data: &[u8]) -> Vec<u8> {
        let parity = self.parity_bytes(data).unwrap_or_default();
        let mut encoded = Vec::with_capacity(data.len() + parity.len());
        encoded.extend_from_slice(data);
        encoded.extend_from_slice(&parity);
        encoded
    }

    fn verify(&self, encoded: &[u8], data_len: usize) -> bool {
        if encoded.len() < data_len + self.parity_shards {
            return false;
        }
        let Some(expected) = self.parity_bytes(&encoded[..data_len]) else {
            return false;
        };
        encoded[data_len..data_len + self.parity_shards] == expected
    }
}

/// Select the ECC scheme for a frame mode and level.
pub const fn scheme_for_mode(mode: TransmissionMode, _level: EccLevel) -> EccScheme {
    match mode {
        TransmissionMode::Print => EccScheme::ReedSolomon,
        TransmissionMode::Screen => screen_scheme(),
        TransmissionMode::Burst => EccScheme::Parity,
    }
}

#[cfg(feature = "ldpc")]
const fn screen_scheme() -> EccScheme {
    EccScheme::Ldpc
}

#[cfg(not(feature = "ldpc"))]
const fn screen_scheme() -> EccScheme {
    EccScheme::Parity
}

/// Interleave stride policy by mode.
pub const fn interleave_stride_for_mode(mode: TransmissionMode, _level: EccLevel) -> usize {
    match mode {
        // Keep print mode neutral while RS rollout stabilizes.
        TransmissionMode::Print => 1,
        // Screen mode benefits from moderate parity spreading.
        TransmissionMode::Screen => 4,
        // Burst mode tolerates stronger parity spreading across frames.
        TransmissionMode::Burst => 8,
    }
}

/// Encode wire bytes with the selected ECC scheme.
pub fn encode_for_mode(mode: TransmissionMode, level: EccLevel, wire: &[u8]) -> Vec<u8> {
    let encoded = match scheme_for_mode(mode, level) {
        EccScheme::Parity => ParityCode::from_level(level, wire.len()).encode(wire),
        EccScheme::ReedSolomon => {
            let rs = ReedSolomonCode::from_level(level, wire.len());
            if rs.is_supported(wire.len()) {
                rs.encode(wire)
            } else {
                // Keep print-mode encodes viable for larger frames that exceed
                // byte-shard Reed-Solomon limits.
                ParityCode::from_level(level, wire.len()).encode(wire)
            }
        }
        #[cfg(feature = "ldpc")]
        EccScheme::Ldpc => {
            // Compatibility fallback until the dedicated LDPC codec lands.
            ParityCode::from_level(level, wire.len()).encode(wire)
        }
    };
    let data_len = wire.len();
    if encoded.len() <= data_len {
        return encoded;
    }
    let stride = interleave_stride_for_mode(mode, level);
    if stride <= 1 {
        return encoded;
    }
    let mut out = Vec::with_capacity(encoded.len());
    out.extend_from_slice(&encoded[..data_len]);
    out.extend_from_slice(&interleave(&encoded[data_len..], stride));
    out
}

/// Verify wire bytes with the selected ECC scheme.
pub fn verify_for_mode(
    mode: TransmissionMode,
    level: EccLevel,
    encoded: &[u8],
    data_len: usize,
) -> bool {
    if encoded.len() < data_len {
        return false;
    }
    let stride = interleave_stride_for_mode(mode, level);
    let normalized = if stride > 1 && encoded.len() > data_len {
        let mut out = Vec::with_capacity(encoded.len());
        out.extend_from_slice(&encoded[..data_len]);
        out.extend_from_slice(&deinterleave(&encoded[data_len..], stride));
        out
    } else {
        encoded.to_vec()
    };
    match scheme_for_mode(mode, level) {
        EccScheme::ReedSolomon => {
            // Backward compatibility: accept legacy parity-encoded print fixtures
            // while migrating new print encodes to Reed-Solomon.
            ReedSolomonCode::from_level(level, data_len).verify(&normalized, data_len)
                || ParityCode::from_level(level, data_len).verify(&normalized, data_len)
        }
        EccScheme::Parity => {
            let parity = ParityCode::from_level(level, data_len);
            // Backward compatibility: accept both current interleaved parity
            // layout and legacy non-interleaved parity layout.
            parity.verify(&normalized, data_len) || parity.verify(encoded, data_len)
        }
        #[cfg(feature = "ldpc")]
        EccScheme::Ldpc => {
            let parity = ParityCode::from_level(level, data_len);
            // Compatibility fallback until the dedicated LDPC codec lands.
            parity.verify(&normalized, data_len) || parity.verify(encoded, data_len)
        }
    }
}

/// Attempt to recover encoded bytes for modes backed by parity code.
///
/// This currently targets single-byte corruption in the data region and
/// deterministic parity repair. It is intentionally conservative and only
/// returns a candidate when parity validation succeeds after repair.
pub fn try_recover_for_mode(
    mode: TransmissionMode,
    level: EccLevel,
    encoded: &[u8],
    data_len: usize,
) -> Option<Vec<u8>> {
    try_recover_for_mode_with_suspects(mode, level, encoded, data_len, &[], usize::MAX)
}

/// Attempt to recover encoded bytes with optional suspect-byte prioritization.
///
/// `suspects` should contain likely-corrupted byte indexes in the data region.
/// Recovery will try these first, then continue with full search until
/// `max_attempts` candidate mutations have been evaluated.
pub fn try_recover_for_mode_with_suspects(
    mode: TransmissionMode,
    level: EccLevel,
    encoded: &[u8],
    data_len: usize,
    suspects: &[usize],
    max_attempts: usize,
) -> Option<Vec<u8>> {
    try_recover_for_mode_with_suspects_and_telemetry(
        mode,
        level,
        encoded,
        data_len,
        suspects,
        max_attempts,
    )
    .0
}

/// Attempt to recover encoded bytes and return recovery telemetry.
pub fn try_recover_for_mode_with_suspects_and_telemetry(
    mode: TransmissionMode,
    level: EccLevel,
    encoded: &[u8],
    data_len: usize,
    suspects: &[usize],
    max_attempts: usize,
) -> (Option<Vec<u8>>, RecoveryTelemetry) {
    let mut telemetry = RecoveryTelemetry {
        attempted: false,
        recovered: false,
        attempts: 0,
        method: RecoveryMethod::None,
        suspect_count: suspects.len(),
        max_attempts_exceeded: false,
    };
    if encoded.len() < data_len {
        return (None, telemetry);
    }
    let stride = interleave_stride_for_mode(mode, level);
    let normalized = if stride > 1 && encoded.len() > data_len {
        let mut out = Vec::with_capacity(encoded.len());
        out.extend_from_slice(&encoded[..data_len]);
        out.extend_from_slice(&deinterleave(&encoded[data_len..], stride));
        out
    } else {
        encoded.to_vec()
    };
    let to_encoded_layout = |candidate: Vec<u8>| {
        if stride > 1 && candidate.len() > data_len {
            let mut out = Vec::with_capacity(candidate.len());
            out.extend_from_slice(&candidate[..data_len]);
            out.extend_from_slice(&interleave(&candidate[data_len..], stride));
            out
        } else {
            candidate
        }
    };

    // Print mode: attempt actual Reed-Solomon one-erasure recovery first when
    // this frame size is supported by the byte-shard RS layout.
    if matches!(mode, TransmissionMode::Print) {
        let rs = ReedSolomonCode::from_level(level, data_len);
        if rs.is_supported(data_len) {
            telemetry.attempted = true;
            let mut attempts = 0usize;
            let mut tried_index = vec![false; data_len];
            for &index in suspects {
                if index < data_len {
                    tried_index[index] = true;
                    attempts += 1;
                    if attempts > max_attempts {
                        telemetry.attempts = attempts;
                        telemetry.max_attempts_exceeded = true;
                        return (None, telemetry);
                    }
                    if let Some(candidate) = rs.recover_one_data_shard(&normalized, data_len, index)
                        && rs.verify(&candidate, data_len)
                        && Frame::decode(&candidate).is_ok()
                    {
                        telemetry.recovered = true;
                        telemetry.attempts = attempts;
                        telemetry.method = RecoveryMethod::ReedSolomonSingle;
                        return (Some(to_encoded_layout(candidate)), telemetry);
                    }
                }
            }
            // Runtime guard: skip hintless 2-byte search to avoid O(n^2)
            // combinatorics in scanner/test paths. Pair recovery is only
            // attempted when upstream supplies likely byte positions.
            if !suspects.is_empty() {
                let mut suspect_pool = Vec::new();
                for &index in suspects {
                    if index < data_len && !suspect_pool.contains(&index) {
                        suspect_pool.push(index);
                    }
                }
                if suspect_pool.len() < 2 {
                    for index in 0..data_len {
                        if !suspect_pool.contains(&index) {
                            suspect_pool.push(index);
                        }
                        if suspect_pool.len() >= 8 {
                            break;
                        }
                    }
                }
                for i in 0..suspect_pool.len() {
                    for j in (i + 1)..suspect_pool.len() {
                        attempts += 1;
                        if attempts > max_attempts {
                            telemetry.attempts = attempts;
                            telemetry.max_attempts_exceeded = true;
                            return (None, telemetry);
                        }
                        let pair = [suspect_pool[i], suspect_pool[j]];
                        if let Some(candidate) =
                            rs.recover_data_shards(&normalized, data_len, &pair)
                            && rs.verify(&candidate, data_len)
                            && Frame::decode(&candidate).is_ok()
                        {
                            telemetry.recovered = true;
                            telemetry.attempts = attempts;
                            telemetry.method = RecoveryMethod::ReedSolomonPair;
                            return (Some(to_encoded_layout(candidate)), telemetry);
                        }
                    }
                }
            }
            for (index, already_tried) in tried_index.iter().enumerate().take(data_len) {
                if *already_tried {
                    continue;
                }
                attempts += 1;
                if attempts > max_attempts {
                    telemetry.attempts = attempts;
                    telemetry.max_attempts_exceeded = true;
                    return (None, telemetry);
                }
                if let Some(candidate) = rs.recover_one_data_shard(&normalized, data_len, index)
                    && rs.verify(&candidate, data_len)
                    && Frame::decode(&candidate).is_ok()
                {
                    telemetry.recovered = true;
                    telemetry.attempts = attempts;
                    telemetry.method = RecoveryMethod::ReedSolomonSingle;
                    return (Some(to_encoded_layout(candidate)), telemetry);
                }
            }
            telemetry.attempts = attempts;
            return (None, telemetry);
        }
    }

    let parity = ParityCode::from_level(level, data_len);
    let parity_len = parity.parity_len();
    if encoded.len() < data_len + parity_len {
        return (None, telemetry);
    }

    // Print mode uses Reed-Solomon when supported; do not attempt parity
    // recovery there. For oversized print frames we already fall back to parity.
    if matches!(mode, TransmissionMode::Print) {
        let rs = ReedSolomonCode::from_level(level, data_len);
        if rs.is_supported(data_len) {
            return (None, telemetry);
        }
    }

    // Fast path: data appears intact and parity tail is corrupted/noisy.
    telemetry.attempted = true;
    let mut repaired = normalized.to_vec();
    let expected = parity.parity_bytes(&repaired[..data_len]);
    repaired[data_len..data_len + parity_len].copy_from_slice(&expected);
    if parity.verify(&repaired, data_len) && Frame::decode(&repaired).is_ok() {
        telemetry.recovered = true;
        telemetry.method = RecoveryMethod::ParityTailRebuild;
        return (Some(to_encoded_layout(repaired)), telemetry);
    }

    // Recovery path: brute-force one corrupted data byte.
    let mut attempts = 0usize;
    let mut candidate = normalized.to_vec();
    let mut tried_index = vec![false; data_len];

    for &index in suspects {
        if index < data_len {
            tried_index[index] = true;
            let original = candidate[index];
            for value in 0u8..=u8::MAX {
                if value == original {
                    continue;
                }
                attempts += 1;
                if attempts > max_attempts {
                    telemetry.attempts = attempts;
                    telemetry.max_attempts_exceeded = true;
                    return (None, telemetry);
                }
                candidate[index] = value;
                if parity.verify(&candidate, data_len) && Frame::decode(&candidate).is_ok() {
                    telemetry.recovered = true;
                    telemetry.attempts = attempts;
                    telemetry.method = RecoveryMethod::ParityByteSearch;
                    return (Some(to_encoded_layout(candidate)), telemetry);
                }
            }
            candidate[index] = original;
        }
    }

    for index in 0..data_len {
        if tried_index[index] {
            continue;
        }
        let original = candidate[index];
        for value in 0u8..=u8::MAX {
            if value == original {
                continue;
            }
            attempts += 1;
            if attempts > max_attempts {
                telemetry.attempts = attempts;
                telemetry.max_attempts_exceeded = true;
                return (None, telemetry);
            }
            candidate[index] = value;
            if parity.verify(&candidate, data_len) && Frame::decode(&candidate).is_ok() {
                telemetry.recovered = true;
                telemetry.attempts = attempts;
                telemetry.method = RecoveryMethod::ParityByteSearch;
                return (Some(to_encoded_layout(candidate)), telemetry);
            }
        }
        candidate[index] = original;
    }
    telemetry.attempts = attempts;
    (None, telemetry)
}

/// Fixed-size XOR erasure recovery over equal-length shards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XorShardCode {
    data_shards: usize,
}

impl XorShardCode {
    /// Create an XOR shard code for `data_shards` data shards plus one parity shard.
    pub const fn new(data_shards: usize) -> Self {
        Self { data_shards }
    }

    /// Split bytes into equal-size data shards and append one parity shard.
    pub fn encode_shards(self, data: &[u8]) -> Result<ShardSet> {
        if self.data_shards == 0 {
            return Err(EccError::InvalidShardConfiguration);
        }

        let shard_len = data.len().div_ceil(self.data_shards).max(1);
        let mut shards = vec![vec![0u8; shard_len]; self.data_shards + 1];
        for (index, byte) in data.iter().enumerate() {
            shards[index / shard_len][index % shard_len] = *byte;
        }

        for offset in 0..shard_len {
            let mut parity = 0u8;
            for shard in shards.iter().take(self.data_shards) {
                parity ^= shard[offset];
            }
            shards[self.data_shards][offset] = parity;
        }

        Ok(ShardSet {
            original_len: data.len(),
            data_shards: self.data_shards,
            shard_len,
            shards,
        })
    }

    /// Recover exactly one missing shard from an XOR shard set.
    pub fn recover_one(self, set: &ShardSet, missing_index: usize) -> Result<Vec<u8>> {
        if set.data_shards != self.data_shards || missing_index >= set.shards.len() {
            return Err(EccError::InvalidShardConfiguration);
        }

        let mut recovered = vec![0u8; set.shard_len];
        for (index, shard) in set.shards.iter().enumerate() {
            if index == missing_index {
                continue;
            }
            for (offset, byte) in shard.iter().enumerate() {
                recovered[offset] ^= byte;
            }
        }
        Ok(recovered)
    }
}

/// Encoded equal-size shards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardSet {
    /// Original byte length before padding.
    pub original_len: usize,
    /// Number of data shards.
    pub data_shards: usize,
    /// Bytes in each shard after padding.
    pub shard_len: usize,
    /// Data shards followed by one parity shard.
    pub shards: Vec<Vec<u8>>,
}

impl ShardSet {
    /// Reassemble data shards into the original byte string.
    pub fn reassemble(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.data_shards * self.shard_len);
        for shard in self.shards.iter().take(self.data_shards) {
            bytes.extend_from_slice(shard);
        }
        bytes.truncate(self.original_len);
        bytes
    }
}

/// Interleave bytes to distribute local damage across distant codewords.
pub fn interleave(bytes: &[u8], stride: usize) -> Vec<u8> {
    if stride <= 1 || bytes.is_empty() {
        return bytes.to_vec();
    }

    let mut out = Vec::with_capacity(bytes.len());
    for lane in 0..stride {
        let mut index = lane;
        while index < bytes.len() {
            out.push(bytes[index]);
            index += stride;
        }
    }
    out
}

/// Reverse [`interleave`].
pub fn deinterleave(bytes: &[u8], stride: usize) -> Vec<u8> {
    if stride <= 1 || bytes.is_empty() {
        return bytes.to_vec();
    }

    let mut out = vec![0u8; bytes.len()];
    let mut source = 0usize;
    for lane in 0..stride {
        let mut index = lane;
        while index < bytes.len() {
            out[index] = bytes[source];
            source += 1;
            index += stride;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use glyphnet_core::Frame;
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn parity_code_verifies_clean_blocks() {
        let code = ParityCode::new(8);
        let encoded = code.encode(b"glyphnet");
        assert!(code.verify(&encoded, b"glyphnet".len()));
    }

    #[test]
    fn parity_code_detects_corruption() {
        let code = ParityCode::new(8);
        let mut encoded = code.encode(b"glyphnet");
        encoded[2] ^= 0x44;
        assert!(!code.verify(&encoded, b"glyphnet".len()));
    }

    #[test]
    fn reed_solomon_code_verifies_clean_blocks() {
        let code = ReedSolomonCode::from_level(EccLevel::High, b"glyphnet".len());
        let encoded = code.encode(b"glyphnet");
        assert!(code.verify(&encoded, b"glyphnet".len()));
    }

    #[test]
    fn reed_solomon_code_detects_corruption() {
        let code = ReedSolomonCode::from_level(EccLevel::High, b"glyphnet".len());
        let mut encoded = code.encode(b"glyphnet");
        encoded[1] ^= 0x55;
        assert!(!code.verify(&encoded, b"glyphnet".len()));
    }

    #[test]
    fn encode_verify_for_mode_roundtrip() {
        let wire = b"glyphnet wire bytes";
        for mode in [
            TransmissionMode::Print,
            TransmissionMode::Screen,
            TransmissionMode::Burst,
        ] {
            let encoded = encode_for_mode(mode, EccLevel::High, wire);
            assert!(verify_for_mode(mode, EccLevel::High, &encoded, wire.len()));
        }
    }

    #[test]
    fn interleave_policy_is_mode_specific() {
        assert_eq!(
            interleave_stride_for_mode(TransmissionMode::Print, EccLevel::High),
            1
        );
        assert_eq!(
            interleave_stride_for_mode(TransmissionMode::Screen, EccLevel::High),
            4
        );
        assert_eq!(
            interleave_stride_for_mode(TransmissionMode::Burst, EccLevel::High),
            8
        );
    }

    #[test]
    fn screen_scheme_matches_feature_gate() {
        #[cfg(feature = "ldpc")]
        assert_eq!(
            scheme_for_mode(TransmissionMode::Screen, EccLevel::High),
            EccScheme::Ldpc
        );
        #[cfg(not(feature = "ldpc"))]
        assert_eq!(
            scheme_for_mode(TransmissionMode::Screen, EccLevel::High),
            EccScheme::Parity
        );
    }

    #[test]
    fn verify_for_mode_accepts_legacy_non_interleaved_screen_payloads() {
        let wire = b"legacy-screen-wire";
        let legacy = ParityCode::from_level(EccLevel::High, wire.len()).encode(wire);
        assert!(verify_for_mode(
            TransmissionMode::Screen,
            EccLevel::High,
            &legacy,
            wire.len(),
        ));
    }

    #[test]
    fn print_mode_large_frames_fallback_to_parity_and_verify() {
        let wire = vec![0x5a; 1100];
        let encoded = encode_for_mode(TransmissionMode::Print, EccLevel::High, &wire);
        assert!(encoded.len() > wire.len());
        assert!(verify_for_mode(
            TransmissionMode::Print,
            EccLevel::High,
            &encoded,
            wire.len()
        ));
    }

    #[test]
    fn parity_recovery_fixes_single_data_byte_corruption() {
        let frame = Frame::new(
            TransmissionMode::Screen,
            EccLevel::High,
            0,
            1,
            42,
            b"glyphnet wire bytes".to_vec(),
        )
        .unwrap();
        let wire = frame.encode();
        let mut encoded = encode_for_mode(TransmissionMode::Screen, EccLevel::High, &wire);
        encoded[6] ^= 0x31;
        assert!(!verify_for_mode(
            TransmissionMode::Screen,
            EccLevel::High,
            &encoded,
            wire.len(),
        ));
        let recovered = try_recover_for_mode(
            TransmissionMode::Screen,
            EccLevel::High,
            &encoded,
            wire.len(),
        )
        .unwrap();
        assert!(verify_for_mode(
            TransmissionMode::Screen,
            EccLevel::High,
            &recovered,
            wire.len(),
        ));
        assert_eq!(&recovered[..wire.len()], wire.as_slice());
    }

    #[test]
    fn prioritized_recovery_respects_attempt_budget() {
        let frame = Frame::new(
            TransmissionMode::Screen,
            EccLevel::High,
            0,
            1,
            77,
            b"budgeted-recovery".to_vec(),
        )
        .unwrap();
        let wire = frame.encode();
        let mut encoded = encode_for_mode(TransmissionMode::Screen, EccLevel::High, &wire);
        encoded[10] ^= 0x44;
        let no_hit = try_recover_for_mode_with_suspects(
            TransmissionMode::Screen,
            EccLevel::High,
            &encoded,
            wire.len(),
            &[0],
            32,
        );
        assert!(no_hit.is_none());
        let hit = try_recover_for_mode_with_suspects(
            TransmissionMode::Screen,
            EccLevel::High,
            &encoded,
            wire.len(),
            &[10],
            512,
        );
        assert!(hit.is_some());
    }

    #[test]
    fn print_mode_rs_recovery_fixes_single_data_byte_corruption() {
        let frame = Frame::new(
            TransmissionMode::Print,
            EccLevel::High,
            0,
            1,
            91,
            b"print-rs".to_vec(),
        )
        .unwrap();
        let wire = frame.encode();
        let mut encoded = encode_for_mode(TransmissionMode::Print, EccLevel::High, &wire);
        encoded[8] ^= 0x66;
        assert!(!verify_for_mode(
            TransmissionMode::Print,
            EccLevel::High,
            &encoded,
            wire.len(),
        ));
        let recovered = try_recover_for_mode_with_suspects(
            TransmissionMode::Print,
            EccLevel::High,
            &encoded,
            wire.len(),
            &[8],
            16,
        )
        .unwrap();
        assert!(verify_for_mode(
            TransmissionMode::Print,
            EccLevel::High,
            &recovered,
            wire.len(),
        ));
        assert_eq!(&recovered[..wire.len()], wire.as_slice());
    }

    #[test]
    fn print_mode_rs_recovery_fixes_two_data_byte_corruptions() {
        let frame = Frame::new(
            TransmissionMode::Print,
            EccLevel::High,
            0,
            1,
            99,
            b"print-rs-two".to_vec(),
        )
        .unwrap();
        let wire = frame.encode();
        let mut encoded = encode_for_mode(TransmissionMode::Print, EccLevel::High, &wire);
        encoded[8] ^= 0x66;
        encoded[12] ^= 0x3a;
        assert!(!verify_for_mode(
            TransmissionMode::Print,
            EccLevel::High,
            &encoded,
            wire.len(),
        ));
        let recovered = try_recover_for_mode_with_suspects(
            TransmissionMode::Print,
            EccLevel::High,
            &encoded,
            wire.len(),
            &[8, 12],
            128,
        )
        .unwrap();
        assert!(verify_for_mode(
            TransmissionMode::Print,
            EccLevel::High,
            &recovered,
            wire.len(),
        ));
        assert_eq!(&recovered[..wire.len()], wire.as_slice());
    }

    #[test]
    fn xor_code_recovers_one_missing_data_shard() {
        let code = XorShardCode::new(4);
        let set = code.encode_shards(b"abcdefghijklmnopqrstuvwxyz").unwrap();
        let recovered = code.recover_one(&set, 1).unwrap();
        assert_eq!(recovered, set.shards[1]);
    }

    proptest! {
        #[test]
        fn interleave_roundtrip(bytes in proptest::collection::vec(any::<u8>(), 0..512), stride in 1usize..32) {
            prop_assert_eq!(deinterleave(&interleave(&bytes, stride), stride), bytes);
        }
    }
}
