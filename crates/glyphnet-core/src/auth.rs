use crate::{GlyphError, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

const AUTH_MAGIC: [u8; 4] = *b"GAUT";
const AUTH_VERSION: u8 = 1;
const AUTH_TAG_LEN: usize = 16;
const AUTH_HEADER_LEN: usize = 16;
const DETACHED_AUTH_MAGIC: [u8; 4] = *b"GDSG";
const DETACHED_AUTH_VERSION: u8 = 1;
const DETACHED_ED25519_MAGIC: [u8; 4] = *b"GDE2";
const DETACHED_ED25519_VERSION: u8 = 1;

/// Shared reason codes for authentication verification outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthVerifyReason {
    UnknownKeyId,
    KeyNotYetValid,
    KeyExpired,
    MissingVerificationKey,
    AuthMismatch,
    InvalidEnvelope,
    VerifyFailed,
    UnsignedPayload,
}

impl AuthVerifyReason {
    /// Stable snake_case code for JSON outputs.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownKeyId => "unknown_key_id",
            Self::KeyNotYetValid => "key_not_yet_valid",
            Self::KeyExpired => "key_expired",
            Self::MissingVerificationKey => "missing_verification_key",
            Self::AuthMismatch => "auth_mismatch",
            Self::InvalidEnvelope => "invalid_envelope",
            Self::VerifyFailed => "verify_failed",
            Self::UnsignedPayload => "unsigned_payload",
        }
    }
}

/// Extract `key_id` from an embedded authenticity envelope header, if present.
pub fn extract_auth_envelope_key_id(payload: &[u8]) -> Option<u32> {
    if payload.len() < 10 {
        return None;
    }
    if payload[0..4] != AUTH_MAGIC || payload[4] != AUTH_VERSION {
        return None;
    }
    Some(u32::from_be_bytes([
        payload[6], payload[7], payload[8], payload[9],
    ]))
}

/// Map core auth verification errors to stable reason codes.
pub fn reason_from_glyph_error(error: GlyphError) -> AuthVerifyReason {
    match error {
        GlyphError::UnknownAuthenticityKey(_) => AuthVerifyReason::UnknownKeyId,
        GlyphError::AuthenticityMismatch => AuthVerifyReason::AuthMismatch,
        GlyphError::InvalidAuthenticityEnvelope => AuthVerifyReason::InvalidEnvelope,
        _ => AuthVerifyReason::VerifyFailed,
    }
}

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

/// Detached Ed25519 authenticity signature metadata and tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetachedEd25519Signature {
    /// Key identifier used by sender to select verification key.
    pub key_id: u32,
    /// Raw payload length covered by this signature.
    pub payload_len: u32,
    /// Ed25519 signature bytes.
    pub signature: [u8; 64],
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

/// Create a detached Ed25519 signature over the raw payload.
pub fn sign_detached_payload_ed25519(
    payload: &[u8],
    signing_key: &SigningKey,
    key_id: u32,
) -> DetachedEd25519Signature {
    let message = detached_ed25519_message(payload, key_id);
    let signature = signing_key.sign(&message);
    DetachedEd25519Signature {
        key_id,
        payload_len: payload.len() as u32,
        signature: signature.to_bytes(),
    }
}

/// Verify a detached Ed25519 signature over the raw payload.
pub fn verify_detached_payload_ed25519<F>(
    payload: &[u8],
    signature: &DetachedEd25519Signature,
    mut key_lookup: F,
) -> Result<()>
where
    F: FnMut(u32) -> Option<[u8; 32]>,
{
    if signature.payload_len != payload.len() as u32 {
        return Err(GlyphError::AuthenticityMismatch);
    }
    let Some(pubkey_bytes) = key_lookup(signature.key_id) else {
        return Err(GlyphError::UnknownAuthenticityKey(signature.key_id));
    };
    let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|_| GlyphError::InvalidAuthenticityKeyMaterial)?;
    let message = detached_ed25519_message(payload, signature.key_id);
    let signature = Signature::from_bytes(&signature.signature);
    verifying_key
        .verify(&message, &signature)
        .map_err(|_| GlyphError::AuthenticityMismatch)
}

fn auth_tag(bytes: &[u8], key: &[u8; 32]) -> [u8; AUTH_TAG_LEN] {
    let hash = blake3::keyed_hash(key, bytes);
    let mut out = [0u8; AUTH_TAG_LEN];
    out.copy_from_slice(&hash.as_bytes()[..AUTH_TAG_LEN]);
    out
}

fn detached_ed25519_message(payload: &[u8], key_id: u32) -> Vec<u8> {
    let mut message = Vec::with_capacity(14 + payload.len());
    message.extend_from_slice(&DETACHED_ED25519_MAGIC);
    message.push(DETACHED_ED25519_VERSION);
    message.push(0);
    message.extend_from_slice(&key_id.to_be_bytes());
    message.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    message.extend_from_slice(payload);
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

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

    #[test]
    fn detached_ed25519_signature_roundtrip() {
        let signing = SigningKey::from_bytes(&[0x2Au8; 32]);
        let verifying = signing.verifying_key().to_bytes();
        let sig = sign_detached_payload_ed25519(b"glyphnet-ed25519", &signing, 12);
        verify_detached_payload_ed25519(b"glyphnet-ed25519", &sig, |id| {
            if id == 12 { Some(verifying) } else { None }
        })
        .unwrap();
    }

    #[test]
    fn detached_ed25519_signature_rejects_wrong_key() {
        let signing = SigningKey::from_bytes(&[0x2Au8; 32]);
        let wrong_signing = SigningKey::from_bytes(&[0x7Bu8; 32]);
        let wrong_verifying = wrong_signing.verifying_key().to_bytes();
        let sig = sign_detached_payload_ed25519(b"glyphnet-ed25519", &signing, 12);
        let err = verify_detached_payload_ed25519(b"glyphnet-ed25519", &sig, |id| {
            if id == 12 {
                Some(wrong_verifying)
            } else {
                None
            }
        })
        .unwrap_err();
        assert!(matches!(err, GlyphError::AuthenticityMismatch));
    }

    #[test]
    fn reason_mapping_is_stable() {
        assert_eq!(
            reason_from_glyph_error(GlyphError::UnknownAuthenticityKey(9)).as_str(),
            "unknown_key_id"
        );
        assert_eq!(
            reason_from_glyph_error(GlyphError::AuthenticityMismatch).as_str(),
            "auth_mismatch"
        );
        assert_eq!(
            reason_from_glyph_error(GlyphError::InvalidAuthenticityEnvelope).as_str(),
            "invalid_envelope"
        );
    }

    #[test]
    fn extract_key_id_from_auth_envelope_header() {
        let sealed = seal_payload(b"glyphnet-auth", &KEY_A, 42);
        assert_eq!(extract_auth_envelope_key_id(&sealed), Some(42));
        assert_eq!(extract_auth_envelope_key_id(b"plain"), None);
    }
}
