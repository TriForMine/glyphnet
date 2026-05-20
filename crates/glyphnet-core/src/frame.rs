use crc32fast::Hasher;

use crate::{EccLevel, GlyphError, Result, TransmissionMode};

/// Binary frame magic.
pub const MAGIC: [u8; 4] = *b"GLYN";
/// Supported binary wire version.
pub const WIRE_VERSION: u8 = 1;
/// Binary frame header length in bytes.
pub const HEADER_LEN: usize = 32;

/// Binary frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Transmission mode.
    pub mode: TransmissionMode,
    /// Error-correction profile.
    pub ecc_level: EccLevel,
    /// Zero-based frame index.
    pub frame_index: u16,
    /// Total frame count.
    pub frame_count: u16,
    /// Stream identifier.
    pub stream_id: u64,
    /// Payload length in bytes before ECC padding.
    pub payload_len: u32,
    /// CRC-32 of the payload bytes.
    pub payload_crc: u32,
    /// CRC-32 of the header fields before this checksum.
    pub header_crc: u32,
}

impl FrameHeader {
    /// Create a validated frame header.
    pub fn new(
        mode: TransmissionMode,
        ecc_level: EccLevel,
        frame_index: u16,
        frame_count: u16,
        stream_id: u64,
        payload_len: u32,
        payload_crc: u32,
    ) -> Result<Self> {
        if frame_count == 0 || frame_index >= frame_count {
            return Err(GlyphError::InvalidFrameIndex {
                index: frame_index,
                count: frame_count,
            });
        }

        let mut header = Self {
            mode,
            ecc_level,
            frame_index,
            frame_count,
            stream_id,
            payload_len,
            payload_crc,
            header_crc: 0,
        };
        header.header_crc = header.compute_header_crc();
        Ok(header)
    }

    /// Encode this header to its fixed-width binary representation.
    pub fn encode(self) -> [u8; HEADER_LEN] {
        let mut bytes = [0u8; HEADER_LEN];
        let mut offset = 0usize;
        bytes[offset..offset + 4].copy_from_slice(&MAGIC);
        offset += 4;
        bytes[offset] = WIRE_VERSION;
        offset += 1;
        bytes[offset] = self.mode.wire_id();
        offset += 1;
        bytes[offset] = self.ecc_level.wire_id();
        offset += 1;
        bytes[offset] = 0;
        offset += 1;
        bytes[offset..offset + 2].copy_from_slice(&self.frame_index.to_be_bytes());
        offset += 2;
        bytes[offset..offset + 2].copy_from_slice(&self.frame_count.to_be_bytes());
        offset += 2;
        bytes[offset..offset + 8].copy_from_slice(&self.stream_id.to_be_bytes());
        offset += 8;
        bytes[offset..offset + 4].copy_from_slice(&self.payload_len.to_be_bytes());
        offset += 4;
        bytes[offset..offset + 4].copy_from_slice(&self.payload_crc.to_be_bytes());
        offset += 4;
        bytes[offset..offset + 4].copy_from_slice(&self.compute_header_crc().to_be_bytes());
        bytes
    }

    /// Decode and validate a fixed-width binary header.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
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
        let header_crc = u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);

        let header = Self {
            mode,
            ecc_level,
            frame_index,
            frame_count,
            stream_id,
            payload_len,
            payload_crc,
            header_crc,
        };
        if frame_count == 0 || frame_index >= frame_count {
            return Err(GlyphError::InvalidFrameIndex {
                index: frame_index,
                count: frame_count,
            });
        }
        if header.compute_header_crc() != header_crc {
            return Err(GlyphError::HeaderChecksumMismatch);
        }
        Ok(header)
    }

    fn compute_header_crc(self) -> u32 {
        let mut bytes = self;
        bytes.header_crc = 0;
        let encoded = bytes.encode_without_header_crc();
        crc32(&encoded)
    }

    fn encode_without_header_crc(self) -> [u8; HEADER_LEN - 4] {
        let mut bytes = [0u8; HEADER_LEN - 4];
        let mut offset = 0usize;
        bytes[offset..offset + 4].copy_from_slice(&MAGIC);
        offset += 4;
        bytes[offset] = WIRE_VERSION;
        offset += 1;
        bytes[offset] = self.mode.wire_id();
        offset += 1;
        bytes[offset] = self.ecc_level.wire_id();
        offset += 1;
        bytes[offset] = 0;
        offset += 1;
        bytes[offset..offset + 2].copy_from_slice(&self.frame_index.to_be_bytes());
        offset += 2;
        bytes[offset..offset + 2].copy_from_slice(&self.frame_count.to_be_bytes());
        offset += 2;
        bytes[offset..offset + 8].copy_from_slice(&self.stream_id.to_be_bytes());
        offset += 8;
        bytes[offset..offset + 4].copy_from_slice(&self.payload_len.to_be_bytes());
        offset += 4;
        bytes[offset..offset + 4].copy_from_slice(&self.payload_crc.to_be_bytes());
        bytes
    }
}

/// Complete binary frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Parsed frame header.
    pub header: FrameHeader,
    /// Payload bytes before ECC padding.
    pub payload: Vec<u8>,
}

impl Frame {
    /// Create a new checksummed frame.
    pub fn new(
        mode: TransmissionMode,
        ecc_level: EccLevel,
        frame_index: u16,
        frame_count: u16,
        stream_id: u64,
        payload: Vec<u8>,
    ) -> Result<Self> {
        let payload_len = u32::try_from(payload.len())
            .map_err(|_| GlyphError::InvalidArgument("payload is too large for one frame"))?;
        let payload_crc = crc32(&payload);
        let header = FrameHeader::new(
            mode,
            ecc_level,
            frame_index,
            frame_count,
            stream_id,
            payload_len,
            payload_crc,
        )?;
        Ok(Self { header, payload })
    }

    /// Encode this frame to binary wire bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + self.payload.len());
        bytes.extend_from_slice(&self.header.encode());
        bytes.extend_from_slice(&self.payload);
        bytes
    }

    /// Decode a binary frame, ignoring trailing ECC or padding bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let header = FrameHeader::decode(bytes)?;
        let payload_len = header.payload_len as usize;
        let required = HEADER_LEN + payload_len;
        if bytes.len() < required {
            return Err(GlyphError::Truncated {
                needed: required,
                actual: bytes.len(),
            });
        }

        let payload = bytes[HEADER_LEN..required].to_vec();
        if crc32(&payload) != header.payload_crc {
            return Err(GlyphError::PayloadChecksumMismatch);
        }
        Ok(Self { header, payload })
    }
}

/// Compute CRC-32 using the canonical GlyphNet polynomial implementation.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(bytes);
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip_is_stable() {
        let frame = Frame::new(
            TransmissionMode::Print,
            EccLevel::High,
            0,
            1,
            42,
            b"hello glyphnet".to_vec(),
        )
        .unwrap();
        assert_eq!(Frame::decode(&frame.encode()).unwrap(), frame);
    }

    #[test]
    fn detects_payload_corruption() {
        let frame = Frame::new(
            TransmissionMode::Screen,
            EccLevel::Medium,
            0,
            1,
            1,
            b"payload".to_vec(),
        )
        .unwrap();
        let mut encoded = frame.encode();
        let last = encoded.last_mut().unwrap();
        *last ^= 0b0000_0001;
        assert!(matches!(
            Frame::decode(&encoded),
            Err(GlyphError::PayloadChecksumMismatch)
        ));
    }

    #[test]
    fn ignores_trailing_padding() {
        let frame = Frame::new(
            TransmissionMode::Burst,
            EccLevel::Low,
            0,
            2,
            99,
            vec![1, 2, 3],
        )
        .unwrap();
        let mut encoded = frame.encode();
        encoded.extend_from_slice(&[0xff; 16]);
        assert_eq!(Frame::decode(&encoded).unwrap(), frame);
    }
}
