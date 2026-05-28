use std::collections::HashMap;

use glyphnet_core::{
    AuthVerifyReason, DetachedAuthSignature, DetachedEd25519Signature,
    extract_auth_envelope_key_id, reason_from_glyph_error, sign_detached_payload,
    sign_detached_payload_ed25519, verify_detached_payload, verify_detached_payload_ed25519,
};
use glyphnet_decode::decode_authenticated_payload;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Clone, Copy, Default)]
struct KeyValidityWindow {
    not_before: Option<OffsetDateTime>,
    not_after: Option<OffsetDateTime>,
}

#[derive(Default)]
struct VerificationKeyring {
    keys: HashMap<u32, [u8; 32]>,
    validity_by_key_id: HashMap<u32, KeyValidityWindow>,
}

pub(crate) fn parse_auth_key_hex(auth_key_hex: &str) -> Result<[u8; 32], String> {
    if auth_key_hex.len() != 64 {
        return Err("auth key must be 64 hex chars (32 bytes)".to_string());
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        let start = i * 2;
        let end = start + 2;
        *byte = u8::from_str_radix(&auth_key_hex[start..end], 16)
            .map_err(|_| "auth key contains non-hex characters".to_string())?;
    }
    Ok(out)
}

fn auth_tag_hex(tag: &[u8; 16]) -> String {
    tag.iter().map(|b| format!("{b:02x}")).collect::<String>()
}

fn parse_detached_signature_json(signature_json: &str) -> Result<DetachedAuthSignature, String> {
    let json: serde_json::Value =
        serde_json::from_str(signature_json).map_err(|error| error.to_string())?;
    let key_id =
        json.get("key_id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "detached signature missing numeric key_id".to_string())? as u32;
    let payload_len = json
        .get("payload_len")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "detached signature missing numeric payload_len".to_string())?
        as u32;
    let tag_hex = json
        .get("tag_hex")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "detached signature missing string tag_hex".to_string())?;
    if tag_hex.len() != 32 {
        return Err("detached signature tag_hex must be 32 hex chars".to_string());
    }
    let mut tag = [0u8; 16];
    for (idx, slot) in tag.iter_mut().enumerate() {
        let start = idx * 2;
        let end = start + 2;
        *slot = u8::from_str_radix(&tag_hex[start..end], 16)
            .map_err(|_| "detached signature contains non-hex characters".to_string())?;
    }
    Ok(DetachedAuthSignature {
        key_id,
        payload_len,
        tag,
    })
}

fn parse_optional_rfc3339(
    item: &serde_json::Value,
    field: &str,
) -> Result<Option<OffsetDateTime>, String> {
    let Some(value) = item.get(field) else {
        return Ok(None);
    };
    let ts = value
        .as_str()
        .ok_or_else(|| format!("keyring item {field} must be string"))?;
    let parsed = OffsetDateTime::parse(ts, &Rfc3339)
        .map_err(|_| format!("keyring item {field} must be RFC3339 UTC"))?;
    Ok(Some(parsed))
}

fn parse_validity(item: &serde_json::Value) -> Result<KeyValidityWindow, String> {
    Ok(KeyValidityWindow {
        not_before: parse_optional_rfc3339(item, "not_before")?,
        not_after: parse_optional_rfc3339(item, "not_after")?,
    })
}

fn parse_keyring_json(keyring_json: &str) -> Result<VerificationKeyring, String> {
    let json: serde_json::Value =
        serde_json::from_str(keyring_json).map_err(|error| error.to_string())?;
    let mut out = VerificationKeyring::default();
    if let Some(arr) = json.as_array() {
        for item in arr {
            let key_id = item
                .get("key_id")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| "keyring item missing numeric key_id".to_string())?
                as u32;
            let key_hex = item
                .get("key_hex")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "keyring item missing string key_hex".to_string())?;
            out.keys.insert(key_id, parse_auth_key_hex(key_hex)?);
            out.validity_by_key_id.insert(key_id, parse_validity(item)?);
        }
        return Ok(out);
    }

    let version = json
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "keyring missing numeric version".to_string())?;
    if version != 1 {
        return Err(format!("unsupported keyring version {version}"));
    }
    let keys = json
        .get("keys")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "keyring missing keys array".to_string())?;
    for item in keys {
        let key_id = item
            .get("key_id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "keyring item missing numeric key_id".to_string())?
            as u32;
        let alg = item
            .get("alg")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "keyring item missing string alg".to_string())?;
        let key_hex = match alg {
            "mac-blake3" => item
                .get("key_hex")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "mac-blake3 key missing string key_hex".to_string())?,
            "ed25519" => item
                .get("public_key_hex")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "ed25519 key missing string public_key_hex".to_string())?,
            _ => return Err(format!("unsupported key algorithm {alg}")),
        };
        out.keys.insert(key_id, parse_auth_key_hex(key_hex)?);
        out.validity_by_key_id.insert(key_id, parse_validity(item)?);
    }
    Ok(out)
}

fn parse_detached_ed25519_signature_json(
    signature_json: &str,
) -> Result<DetachedEd25519Signature, String> {
    let json: serde_json::Value =
        serde_json::from_str(signature_json).map_err(|error| error.to_string())?;
    let key_id =
        json.get("key_id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "detached signature missing numeric key_id".to_string())? as u32;
    let payload_len = json
        .get("payload_len")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "detached signature missing numeric payload_len".to_string())?
        as u32;
    let signature_hex = json
        .get("signature_hex")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "detached signature missing string signature_hex".to_string())?;
    if signature_hex.len() != 128 {
        return Err("detached signature signature_hex must be 128 hex chars".to_string());
    }
    let mut signature = [0u8; 64];
    for (idx, slot) in signature.iter_mut().enumerate() {
        let start = idx * 2;
        let end = start + 2;
        *slot = u8::from_str_radix(&signature_hex[start..end], 16)
            .map_err(|_| "detached signature contains non-hex characters".to_string())?;
    }
    Ok(DetachedEd25519Signature {
        key_id,
        payload_len,
        signature,
    })
}

fn validity_failure(
    keyring: &VerificationKeyring,
    key_id: u32,
    now: OffsetDateTime,
) -> Option<(&'static str, String)> {
    let validity = keyring.validity_by_key_id.get(&key_id)?;
    if let Some(not_before) = validity.not_before
        && now < not_before
    {
        return Some((
            AuthVerifyReason::KeyNotYetValid.as_str(),
            format!("key_id {key_id} is not valid yet"),
        ));
    }
    if let Some(not_after) = validity.not_after
        && now > not_after
    {
        return Some((
            AuthVerifyReason::KeyExpired.as_str(),
            format!("key_id {key_id} is expired"),
        ));
    }
    None
}

fn reason_from_error_text(error: &str) -> &'static str {
    if error.contains("unknown authenticity key id") {
        AuthVerifyReason::UnknownKeyId.as_str()
    } else if error.contains("authenticity tag mismatch") {
        AuthVerifyReason::AuthMismatch.as_str()
    } else if error.contains("invalid authenticity envelope") {
        AuthVerifyReason::InvalidEnvelope.as_str()
    } else {
        AuthVerifyReason::VerifyFailed.as_str()
    }
}

pub(crate) fn sign_detached_auth_json(
    payload: &[u8],
    auth_key: &[u8; 32],
    key_id: u32,
) -> Result<String, String> {
    let signature = sign_detached_payload(payload, auth_key, key_id);
    serde_json::to_string_pretty(&serde_json::json!({
        "key_id": signature.key_id,
        "payload_len": signature.payload_len,
        "tag_hex": auth_tag_hex(&signature.tag)
    }))
    .map_err(|error| error.to_string())
}

pub(crate) fn verify_detached_auth_json(
    payload: &[u8],
    signature_json: &str,
    keyring_json: &str,
) -> Result<String, String> {
    let signature = parse_detached_signature_json(signature_json)?;
    let keyring = parse_keyring_json(keyring_json)?;
    let now = OffsetDateTime::now_utc();
    if let Some((reason, error)) = validity_failure(&keyring, signature.key_id, now) {
        return serde_json::to_string_pretty(&serde_json::json!({
            "verified": false,
            "key_id": signature.key_id,
            "error": error,
            "reason": reason
        }))
        .map_err(|error| error.to_string());
    }
    let result =
        match verify_detached_payload(payload, &signature, |id| keyring.keys.get(&id).copied()) {
            Ok(()) => serde_json::json!({
                "verified": true,
                "key_id": signature.key_id,
                "error": serde_json::Value::Null,
                "reason": serde_json::Value::Null
            }),
            Err(error) => {
                let error_text = error.to_string();
                serde_json::json!({
                    "verified": false,
                    "key_id": signature.key_id,
                    "error": error_text,
                    "reason": reason_from_glyph_error(error).as_str()
                })
            }
        };
    serde_json::to_string_pretty(&result).map_err(|error| error.to_string())
}

pub(crate) fn sign_detached_ed25519_json(
    payload: &[u8],
    signing_key: &[u8; 32],
    key_id: u32,
) -> Result<String, String> {
    let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key);
    let signature = sign_detached_payload_ed25519(payload, &signing_key, key_id);
    let signature_hex = signature
        .signature
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    serde_json::to_string_pretty(&serde_json::json!({
        "key_id": signature.key_id,
        "payload_len": signature.payload_len,
        "signature_hex": signature_hex
    }))
    .map_err(|error| error.to_string())
}

pub(crate) fn verify_detached_ed25519_json(
    payload: &[u8],
    signature_json: &str,
    keyring_json: &str,
) -> Result<String, String> {
    let signature = parse_detached_ed25519_signature_json(signature_json)?;
    let keyring = parse_keyring_json(keyring_json)?;
    let now = OffsetDateTime::now_utc();
    if let Some((reason, error)) = validity_failure(&keyring, signature.key_id, now) {
        return serde_json::to_string_pretty(&serde_json::json!({
            "verified": false,
            "key_id": signature.key_id,
            "error": error,
            "reason": reason
        }))
        .map_err(|error| error.to_string());
    }
    let result = match verify_detached_payload_ed25519(payload, &signature, |id| {
        keyring.keys.get(&id).copied()
    }) {
        Ok(()) => serde_json::json!({
            "verified": true,
            "key_id": signature.key_id,
            "error": serde_json::Value::Null,
            "reason": serde_json::Value::Null
        }),
        Err(error) => {
            let error_text = error.to_string();
            serde_json::json!({
                "verified": false,
                "key_id": signature.key_id,
                "error": error_text,
                "reason": reason_from_glyph_error(error).as_str()
            })
        }
    };
    serde_json::to_string_pretty(&result).map_err(|error| error.to_string())
}

pub(crate) fn verify_payload_with_optional_key(
    payload: &[u8],
    verify_key: Option<[u8; 32]>,
    verify_key_id: Option<u32>,
) -> Result<Option<serde_json::Value>, String> {
    let envelope_key_id = extract_auth_envelope_key_id(payload);
    if envelope_key_id.is_none() {
        return Ok(None);
    }
    let (Some(key), Some(key_id)) = (verify_key, verify_key_id) else {
        return Ok(Some(serde_json::json!({
            "verified": false,
            "key_id": envelope_key_id,
            "error": "authenticated payload detected but no verification key was provided",
            "reason": AuthVerifyReason::MissingVerificationKey.as_str()
        })));
    };
    match decode_authenticated_payload(payload, |id| if id == key_id { Some(key) } else { None }) {
        Ok(_) => Ok(Some(serde_json::json!({
            "verified": true,
            "key_id": envelope_key_id.or(Some(key_id)),
            "error": serde_json::Value::Null,
            "reason": serde_json::Value::Null
        }))),
        Err(error) => {
            let error_text = error.to_string();
            Ok(Some(serde_json::json!({
                "verified": false,
                "key_id": envelope_key_id.or(Some(key_id)),
                "error": error_text,
                "reason": reason_from_error_text(&error_text)
            })))
        }
    }
}
