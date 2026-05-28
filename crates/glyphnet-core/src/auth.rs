use crate::{GlyphError, Result};

const AUTH_MAGIC: [u8; 4] = *b"GAUT";
const AUTH_VERSION: u8 = 1;
const AUTH_TAG_LEN: usize = 16;
const AUTH_HEADER_LEN: usize = 16;
const DETACHED_AUTH_MAGIC: [u8; 4] = *b"GDSG";
const DETACHED_AUTH_VERSION: u8 = 1;

/// Embedded authenticity envelope metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthEnvelopeHeader {
    /// Key identifier used by sender to select verification key.
    pub key_id: u32,
    /// Raw payload length before envelope/tag.
    pub payload_len: u32,
}

/// Detached authenticity signature metadata and tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetachedAuthSignature {
    /// Key identifier used by sender to select verification key.
    pub key_id: u32,
    /// Raw payload length covered by this signature.
    pub payload_len: u32,
    /// Truncated keyed BLAKE3 tag.
    pub tag: [u8; AUTH_TAG_LEN],
}

/// Build an embedded authenticity envelope using BLAKE3 keyed MAC.
pub fn seal_payload(payload: &[u8], key: &[u8; 32], key_id: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(AUTH_HEADER_LEN + payload.len() + AUTH_TAG_LEN);
    out.extend_from_slice(&AUTH_MAGIC);
    out.push(AUTH_VERSION);
    out.push(0); // reserved flags
    out.extend_from_slice(&key_id.to_be_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&[0u8; 2]); // reserved
    out.extend_from_slice(payload);
    let tag = auth_tag(&out[..AUTH_HEADER_LEN + payload.len()], key);
    out.extend_from_slice(&tag);
    out
}

/// Parse and verify an embedded authenticity envelope.
pub fn open_payload<F>(bytes: &[u8], mut key_lookup: F) -> Result<(AuthEnvelopeHeader, Vec<u8>)>
where
    F: FnMut(u32) -> Option<[u8; 32]>,
{
    if bytes.len() < AUTH_HEADER_LEN + AUTH_TAG_LEN {
        return Err(GlyphError::InvalidAuthenticityEnvelope);
    }
    if bytes[0..4] != AUTH_MAGIC || bytes[4] != AUTH_VERSION {
        return Err(GlyphError::InvalidAuthenticityEnvelope);
    }
    let key_id = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);
    let payload_len = u32::from_be_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    let required = AUTH_HEADER_LEN + payload_len + AUTH_TAG_LEN;
    if bytes.len() < required {
        return Err(GlyphError::InvalidAuthenticityEnvelope);
    }

    let Some(key) = key_lookup(key_id) else {
        return Err(GlyphError::UnknownAuthenticityKey(key_id));
    };
    let signed_end = AUTH_HEADER_LEN + payload_len;
    let expected = auth_tag(&bytes[..signed_end], &key);
    let provided = &bytes[signed_end..signed_end + AUTH_TAG_LEN];
    if provided != expected {
        return Err(GlyphError::AuthenticityMismatch);
    }
    let payload = bytes[AUTH_HEADER_LEN..signed_end].to_vec();
    Ok((
        AuthEnvelopeHeader {
            key_id,
            payload_len: payload_len as u32,
        },
        payload,
    ))
}

/// Create a detached authenticity signature over the raw payload.
pub fn sign_detached_payload(payload: &[u8], key: &[u8; 32], key_id: u32) -> DetachedAuthSignature {
    let mut to_sign = Vec::with_capacity(10 + payload.len());
    to_sign.extend_from_slice(&DETACHED_AUTH_MAGIC);
    to_sign.push(DETACHED_AUTH_VERSION);
    to_sign.push(0);
    to_sign.extend_from_slice(&key_id.to_be_bytes());
    to_sign.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    to_sign.extend_from_slice(payload);
    DetachedAuthSignature {
        key_id,
        payload_len: payload.len() as u32,
        tag: auth_tag(&to_sign, key),
    }
}

/// Verify a detached authenticity signature over the raw payload.
pub fn verify_detached_payload<F>(
    payload: &[u8],
    signature: &DetachedAuthSignature,
    mut key_lookup: F,
) -> Result<()>
where
    F: FnMut(u32) -> Option<[u8; 32]>,
{
    if signature.payload_len != payload.len() as u32 {
        return Err(GlyphError::AuthenticityMismatch);
    }
    let Some(key) = key_lookup(signature.key_id) else {
        return Err(GlyphError::UnknownAuthenticityKey(signature.key_id));
    };
    let expected = sign_detached_payload(payload, &key, signature.key_id);
    if expected.tag != signature.tag {
        return Err(GlyphError::AuthenticityMismatch);
    }
    Ok(())
}

fn auth_tag(bytes: &[u8], key: &[u8; 32]) -> [u8; AUTH_TAG_LEN] {
    let hash = blake3::keyed_hash(key, bytes);
    let mut out = [0u8; AUTH_TAG_LEN];
    out.copy_from_slice(&hash.as_bytes()[..AUTH_TAG_LEN]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: [u8; 32] = [0x11; 32];
    const KEY_B: [u8; 32] = [0x22; 32];

    #[test]
    fn auth_envelope_roundtrip() {
        let sealed = seal_payload(b"glyphnet-auth", &KEY_A, 7);
        let (header, payload) =
            open_payload(&sealed, |id| if id == 7 { Some(KEY_A) } else { None }).unwrap();
        assert_eq!(header.key_id, 7);
        assert_eq!(payload, b"glyphnet-auth");
    }

    #[test]
    fn auth_envelope_rejects_wrong_key() {
        let sealed = seal_payload(b"glyphnet-auth", &KEY_A, 7);
        let err = open_payload(&sealed, |id| if id == 7 { Some(KEY_B) } else { None }).unwrap_err();
        assert!(matches!(err, GlyphError::AuthenticityMismatch));
    }

    #[test]
    fn auth_envelope_rejects_unknown_key() {
        let sealed = seal_payload(b"glyphnet-auth", &KEY_A, 9);
        let err = open_payload(&sealed, |_id| None).unwrap_err();
        assert!(matches!(err, GlyphError::UnknownAuthenticityKey(9)));
    }

    #[test]
    fn detached_signature_roundtrip() {
        let sig = sign_detached_payload(b"glyphnet-detached", &KEY_A, 5);
        verify_detached_payload(b"glyphnet-detached", &sig, |id| {
            if id == 5 { Some(KEY_A) } else { None }
        })
        .unwrap();
    }

    #[test]
    fn detached_signature_rejects_wrong_key() {
        let sig = sign_detached_payload(b"glyphnet-detached", &KEY_A, 5);
        let err = verify_detached_payload(b"glyphnet-detached", &sig, |id| {
            if id == 5 { Some(KEY_B) } else { None }
        })
        .unwrap_err();
        assert!(matches!(err, GlyphError::AuthenticityMismatch));
    }
}
