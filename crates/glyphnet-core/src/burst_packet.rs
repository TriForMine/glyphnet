use crate::{GlyphError, Result, frame::crc32};

/// Binary burst packet magic.
pub const BURST_PACKET_MAGIC: [u8; 4] = *b"GBPK";
/// Supported burst packet wire version.
pub const BURST_PACKET_VERSION: u8 = 1;
/// Fixed packet header length in bytes.
pub const BURST_PACKET_HEADER_LEN: usize = 32;

/// Metadata header for burst transport packets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BurstPacketHeader {
    /// Zero-based packet sequence number.
    pub sequence: u16,
    /// Total number of packets in this transfer.
    pub packet_count: u16,
    /// Stream identifier that groups all packets in a transfer.
    pub stream_id: u64,
    /// Flags reserved for transport signaling.
    pub flags: u8,
    /// Original burst payload length.
    pub original_len: u32,
    /// Number of data shards in this burst shard set.
    pub data_shards: u16,
    /// Payload length in bytes.
    pub payload_len: u16,
    /// CRC-32 of payload bytes.
    pub payload_crc: u32,
}

impl BurstPacketHeader {
    /// Build and validate a burst packet header.
    pub fn new(
        sequence: u16,
        packet_count: u16,
        stream_id: u64,
        flags: u8,
        original_len: u32,
        data_shards: u16,
        payload_len: u16,
        payload_crc: u32,
    ) -> Result<Self> {
        if packet_count == 0 || sequence >= packet_count {
            return Err(GlyphError::InvalidFrameIndex {
                index: sequence,
                count: packet_count,
            });
        }

        Ok(Self {
            sequence,
            packet_count,
            stream_id,
            flags,
            original_len,
            data_shards,
            payload_len,
            payload_crc,
        })
    }

    /// Encode this header to its fixed-width wire representation.
    pub fn encode(self) -> [u8; BURST_PACKET_HEADER_LEN] {
        let mut out = [0u8; BURST_PACKET_HEADER_LEN];
        out[0..4].copy_from_slice(&BURST_PACKET_MAGIC);
        out[4] = BURST_PACKET_VERSION;
        out[5] = self.flags;
        out[6..8].copy_from_slice(&self.sequence.to_be_bytes());
        out[8..10].copy_from_slice(&self.packet_count.to_be_bytes());
        out[10..18].copy_from_slice(&self.stream_id.to_be_bytes());
        out[18..22].copy_from_slice(&self.original_len.to_be_bytes());
        out[22..24].copy_from_slice(&self.data_shards.to_be_bytes());
        out[24..26].copy_from_slice(&self.payload_len.to_be_bytes());
        out[26..30].copy_from_slice(&self.payload_crc.to_be_bytes());
        out
    }

    /// Decode and validate a wire header.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < BURST_PACKET_HEADER_LEN {
            return Err(GlyphError::Truncated {
                needed: BURST_PACKET_HEADER_LEN,
                actual: bytes.len(),
            });
        }
        if bytes[0..4] != BURST_PACKET_MAGIC {
            return Err(GlyphError::InvalidMagic);
        }
        if bytes[4] != BURST_PACKET_VERSION {
            return Err(GlyphError::UnsupportedVersion(bytes[4]));
        }

        let sequence = u16::from_be_bytes([bytes[6], bytes[7]]);
        let packet_count = u16::from_be_bytes([bytes[8], bytes[9]]);
        let stream_id = u64::from_be_bytes([
            bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17],
        ]);
        let original_len = u32::from_be_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
        let data_shards = u16::from_be_bytes([bytes[22], bytes[23]]);
        let payload_len = u16::from_be_bytes([bytes[24], bytes[25]]);
        let payload_crc = u32::from_be_bytes([bytes[26], bytes[27], bytes[28], bytes[29]]);

        Self::new(
            sequence,
            packet_count,
            stream_id,
            bytes[5],
            original_len,
            data_shards,
            payload_len,
            payload_crc,
        )
    }
}

/// Complete burst transport packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BurstPacket {
    /// Parsed packet header.
    pub header: BurstPacketHeader,
    /// Raw payload carried by this packet.
    pub payload: Vec<u8>,
}

impl BurstPacket {
    /// Create a burst packet from metadata and payload.
    pub fn new(
        sequence: u16,
        packet_count: u16,
        stream_id: u64,
        flags: u8,
        original_len: u32,
        data_shards: u16,
        payload: Vec<u8>,
    ) -> Result<Self> {
        let payload_len = u16::try_from(payload.len())
            .map_err(|_| GlyphError::InvalidArgument("burst packet payload exceeds u16::MAX"))?;
        let payload_crc = crc32(&payload);
        let header = BurstPacketHeader::new(
            sequence,
            packet_count,
            stream_id,
            flags,
            original_len,
            data_shards,
            payload_len,
            payload_crc,
        )?;
        Ok(Self { header, payload })
    }

    /// Encode packet bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(BURST_PACKET_HEADER_LEN + self.payload.len());
        bytes.extend_from_slice(&self.header.encode());
        bytes.extend_from_slice(&self.payload);
        bytes
    }

    /// Decode and validate packet bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let header = BurstPacketHeader::decode(bytes)?;
        let required = BURST_PACKET_HEADER_LEN + usize::from(header.payload_len);
        if bytes.len() < required {
            return Err(GlyphError::Truncated {
                needed: required,
                actual: bytes.len(),
            });
        }
        let payload = bytes[BURST_PACKET_HEADER_LEN..required].to_vec();
        if crc32(&payload) != header.payload_crc {
            return Err(GlyphError::PayloadChecksumMismatch);
        }
        Ok(Self { header, payload })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_packet_roundtrip_is_stable() {
        let packet = BurstPacket::new(1, 3, 9, 0b0000_0011, 5, 2, b"hello".to_vec()).unwrap();
        assert_eq!(BurstPacket::decode(&packet.encode()).unwrap(), packet);
    }

    #[test]
    fn burst_packet_decode_rejects_truncated_header() {
        let err = BurstPacket::decode(&[0u8; 7]).unwrap_err();
        assert!(matches!(err, GlyphError::Truncated { .. }));
    }

    #[test]
    fn burst_packet_decode_rejects_payload_crc_mismatch() {
        let mut encoded = BurstPacket::new(0, 1, 42, 0, 4, 1, vec![1, 2, 3, 4])
            .unwrap()
            .encode();
        let last = encoded.last_mut().unwrap();
        *last ^= 0b0000_0001;
        assert!(matches!(
            BurstPacket::decode(&encoded),
            Err(GlyphError::PayloadChecksumMismatch)
        ));
    }

    #[test]
    fn burst_packet_header_rejects_invalid_sequence() {
        let err = BurstPacketHeader::new(2, 2, 5, 0, 8, 2, 1, 7).unwrap_err();
        assert!(matches!(err, GlyphError::InvalidFrameIndex { .. }));
    }
}
