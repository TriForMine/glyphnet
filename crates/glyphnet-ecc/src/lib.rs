//! Error-correction and erasure-recovery primitives.
//!
//! The reference implementation starts with deterministic parity and XOR
//! erasure recovery so the protocol stack has stable behavior and tests today.
//! The crate boundary is designed to host LDPC, fountain, and RaptorQ-like
//! implementations without changing encoder/decoder APIs.

use glyphnet_core::{EccLevel, GlyphError, TransmissionMode};
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
        TransmissionMode::Screen | TransmissionMode::Burst => EccScheme::Parity,
    }
}

/// Encode wire bytes with the selected ECC scheme.
pub fn encode_for_mode(mode: TransmissionMode, level: EccLevel, wire: &[u8]) -> Vec<u8> {
    match scheme_for_mode(mode, level) {
        EccScheme::Parity => ParityCode::from_level(level, wire.len()).encode(wire),
        EccScheme::ReedSolomon => ReedSolomonCode::from_level(level, wire.len()).encode(wire),
    }
}

/// Verify wire bytes with the selected ECC scheme.
pub fn verify_for_mode(
    mode: TransmissionMode,
    level: EccLevel,
    encoded: &[u8],
    data_len: usize,
) -> bool {
    match scheme_for_mode(mode, level) {
        EccScheme::Parity => ParityCode::from_level(level, data_len).verify(encoded, data_len),
        EccScheme::ReedSolomon => {
            ReedSolomonCode::from_level(level, data_len).verify(encoded, data_len)
        }
    }
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
