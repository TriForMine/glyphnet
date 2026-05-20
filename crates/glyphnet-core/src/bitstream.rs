/// Convert bytes into most-significant-bit-first bits.
pub fn bytes_to_bits(bytes: &[u8]) -> Vec<bool> {
    let mut bits = Vec::with_capacity(bytes.len() * 8);
    for byte in bytes {
        for shift in (0..8).rev() {
            bits.push((byte >> shift) & 1 == 1);
        }
    }
    bits
}

/// Convert most-significant-bit-first bits into bytes, padding the final byte with zeros.
pub fn bits_to_bytes(bits: &[bool]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(bits.len().div_ceil(8));
    for chunk in bits.chunks(8) {
        let mut byte = 0u8;
        for (index, bit) in chunk.iter().enumerate() {
            if *bit {
                byte |= 1 << (7 - index);
            }
        }
        bytes.push(byte);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_roundtrip_preserves_bytes() {
        let bytes = [0x00, 0x01, 0x80, 0xff, 0xa5];
        assert_eq!(bits_to_bytes(&bytes_to_bits(&bytes)), bytes);
    }
}
