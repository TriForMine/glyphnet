use std::{collections::HashMap, fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use glyphnet_core::{
    DetachedAuthSignature, DetachedEd25519Signature, open_authenticated_payload,
    sign_detached_payload, sign_detached_payload_ed25519, verify_detached_payload,
    verify_detached_payload_ed25519,
};
use glyphnet_decode::decode_authenticated_payload;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub(crate) enum DetachedVerificationInput {
    Mac(DetachedAuthSignature),
    Ed25519(DetachedEd25519Signature),
}

#[derive(Default)]
struct VerificationKeyring {
    mac_keys: HashMap<u32, [u8; 32]>,
    ed25519_public_keys: HashMap<u32, [u8; 32]>,
}

pub(crate) fn parse_auth_key_hex(input: &str) -> Result<[u8; 32]> {
    let normalized = input.trim();
    if normalized.len() != 64 {
        bail!("auth key must be exactly 64 hex characters");
    }
    let mut key = [0u8; 32];
    for (idx, slot) in key.iter_mut().enumerate() {
        let start = idx * 2;
        let end = start + 2;
        *slot = u8::from_str_radix(&normalized[start..end], 16)
            .with_context(|| format!("invalid hex at bytes {}..{}", start, end))?;
    }
    Ok(key)
}

fn parse_public_key_hex(input: &str) -> Result<[u8; 32]> {
    parse_auth_key_hex(input)
}

pub(crate) fn verify_auth_payload(
    payload: &[u8],
    verify_key_hex: Option<&str>,
    verify_key_id: u32,
    verify_key_file: Option<&PathBuf>,
    detached_signature: Option<&DetachedVerificationInput>,
) -> Result<Option<(bool, u32, Option<String>)>> {
    let keyring = verification_keyring(verify_key_hex, verify_key_id, verify_key_file)?;
    if let Some(signature) = detached_signature {
        match signature {
            DetachedVerificationInput::Mac(signature) => {
                match verify_detached_payload(payload, signature, |id| {
                    keyring.mac_keys.get(&id).copied()
                }) {
                    Ok(_) => return Ok(Some((true, signature.key_id, None))),
                    Err(error) => {
                        return Ok(Some((false, signature.key_id, Some(error.to_string()))));
                    }
                }
            }
            DetachedVerificationInput::Ed25519(signature) => {
                match verify_detached_payload_ed25519(payload, signature, |id| {
                    keyring.ed25519_public_keys.get(&id).copied()
                }) {
                    Ok(_) => return Ok(Some((true, signature.key_id, None))),
                    Err(error) => {
                        return Ok(Some((false, signature.key_id, Some(error.to_string()))));
                    }
                }
            }
        }
    }
    if open_authenticated_payload(payload, |_id| None).is_err() {
        return Ok(None);
    }
    if keyring.mac_keys.is_empty() {
        return Ok(Some((
            false,
            verify_key_id,
            Some("missing verification key".to_string()),
        )));
    }
    match decode_authenticated_payload(payload, |id| keyring.mac_keys.get(&id).copied()) {
        Ok(_) => Ok(Some((true, verify_key_id, None))),
        Err(error) => Ok(Some((false, verify_key_id, Some(error.to_string())))),
    }
}

fn verification_keyring(
    verify_key_hex: Option<&str>,
    verify_key_id: u32,
    verify_key_file: Option<&PathBuf>,
) -> Result<VerificationKeyring> {
    let mut keys = VerificationKeyring::default();
    if let Some(path) = verify_key_file {
        let loaded = load_verify_keys(path)?;
        keys.mac_keys.extend(loaded.mac_keys);
        keys.ed25519_public_keys.extend(loaded.ed25519_public_keys);
    }
    if let Some(hex) = verify_key_hex {
        keys.mac_keys
            .insert(verify_key_id, parse_auth_key_hex(hex)?);
    }
    Ok(keys)
}

fn load_verify_keys(path: &PathBuf) -> Result<VerificationKeyring> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read verify key file {}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("invalid JSON in verify key file {}", path.display()))?;
    let mut out = VerificationKeyring::default();
    if let Some(arr) = json.as_array() {
        for item in arr {
            let key_id = item
                .get("key_id")
                .and_then(serde_json::Value::as_u64)
                .context("verify key item missing numeric key_id")? as u32;
            let key_hex = item
                .get("key_hex")
                .and_then(serde_json::Value::as_str)
                .context("verify key item missing string key_hex")?;
            out.mac_keys.insert(key_id, parse_auth_key_hex(key_hex)?);
        }
        return Ok(out);
    }
    let version = json
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .context("verify key file missing numeric version")?;
    if version != 1 {
        bail!("unsupported verify key file version {version}");
    }
    let keys = json
        .get("keys")
        .and_then(serde_json::Value::as_array)
        .context("verify key file missing keys array")?;
    for item in keys {
        let key_id = item
            .get("key_id")
            .and_then(serde_json::Value::as_u64)
            .context("verify key item missing numeric key_id")? as u32;
        let alg = item
            .get("alg")
            .and_then(serde_json::Value::as_str)
            .context("verify key item missing string alg")?;
        match alg {
            "mac-blake3" => {
                let key_hex = item
                    .get("key_hex")
                    .and_then(serde_json::Value::as_str)
                    .context("mac-blake3 key missing string key_hex")?;
                out.mac_keys.insert(key_id, parse_auth_key_hex(key_hex)?);
            }
            "ed25519" => {
                let key_hex = item
                    .get("public_key_hex")
                    .and_then(serde_json::Value::as_str)
                    .context("ed25519 key missing string public_key_hex")?;
                out.ed25519_public_keys
                    .insert(key_id, parse_public_key_hex(key_hex)?);
            }
            _ => bail!("unsupported key algorithm {alg}"),
        }
    }
    Ok(out)
}

fn load_keyset_json(path: &PathBuf) -> Result<serde_json::Value> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read keyset file {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("invalid JSON in keyset file {}", path.display()))
}

fn validate_keyset_json(json: &serde_json::Value) -> Result<()> {
    let version = json
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .context("keyset missing numeric version")?;
    if version != 1 {
        bail!("unsupported keyset version {version}");
    }
    json.get("issuer")
        .and_then(serde_json::Value::as_str)
        .context("keyset missing string issuer")?;
    let created_at = json
        .get("created_at")
        .and_then(serde_json::Value::as_str)
        .context("keyset missing string created_at")?;
    let expires_at = json
        .get("expires_at")
        .and_then(serde_json::Value::as_str)
        .context("keyset missing string expires_at")?;
    let created = OffsetDateTime::parse(created_at, &Rfc3339)
        .context("keyset created_at must be RFC3339 UTC")?;
    let expires = OffsetDateTime::parse(expires_at, &Rfc3339)
        .context("keyset expires_at must be RFC3339 UTC")?;
    if expires <= created {
        bail!("keyset expires_at must be after created_at");
    }
    let keys = json
        .get("keys")
        .and_then(serde_json::Value::as_array)
        .context("keyset missing keys array")?;
    if keys.is_empty() {
        bail!("keyset keys array must not be empty");
    }
    for item in keys {
        item.get("key_id")
            .and_then(serde_json::Value::as_u64)
            .context("keyset key missing numeric key_id")?;
        let alg = item
            .get("alg")
            .and_then(serde_json::Value::as_str)
            .context("keyset key missing string alg")?;
        match alg {
            "mac-blake3" => {
                let key_hex = item
                    .get("key_hex")
                    .and_then(serde_json::Value::as_str)
                    .context("mac-blake3 key missing string key_hex")?;
                let _ = parse_auth_key_hex(key_hex)?;
            }
            "ed25519" => {
                let key_hex = item
                    .get("public_key_hex")
                    .and_then(serde_json::Value::as_str)
                    .context("ed25519 key missing string public_key_hex")?;
                let _ = parse_public_key_hex(key_hex)?;
            }
            _ => bail!("unsupported key algorithm {alg}"),
        }
    }
    Ok(())
}

fn keyset_payload_for_signature(json: &serde_json::Value) -> Result<Vec<u8>> {
    let version = json
        .get("version")
        .cloned()
        .context("keyset missing version")?;
    let issuer = json
        .get("issuer")
        .cloned()
        .context("keyset missing issuer")?;
    let created_at = json
        .get("created_at")
        .cloned()
        .context("keyset missing created_at")?;
    let expires_at = json
        .get("expires_at")
        .cloned()
        .context("keyset missing expires_at")?;
    let keys = json.get("keys").cloned().context("keyset missing keys")?;
    serde_json::to_vec(&serde_json::json!({
        "version": version,
        "issuer": issuer,
        "created_at": created_at,
        "expires_at": expires_at,
        "keys": keys
    }))
    .context("failed to serialize keyset payload for signature")
}

fn parse_signature_hex(signature_hex: &str) -> Result<[u8; 64]> {
    if signature_hex.len() != 128 {
        bail!("keyset signature_hex must be 128 hex chars");
    }
    let mut signature = [0u8; 64];
    for (idx, slot) in signature.iter_mut().enumerate() {
        let start = idx * 2;
        let end = start + 2;
        *slot = u8::from_str_radix(&signature_hex[start..end], 16)
            .with_context(|| format!("invalid signature hex at bytes {}..{}", start, end))?;
    }
    Ok(signature)
}

fn verify_keyset_signature(json: &serde_json::Value, root_pubkey_hex: &str) -> Result<()> {
    let sig = json
        .get("signature")
        .and_then(serde_json::Value::as_object)
        .context("keyset missing signature object")?;
    let signature_alg = sig
        .get("signature_alg")
        .and_then(serde_json::Value::as_str)
        .context("keyset signature missing signature_alg")?;
    if signature_alg != "ed25519" {
        bail!("unsupported keyset signature_alg {signature_alg}");
    }
    let signature_hex = sig
        .get("signature_hex")
        .and_then(serde_json::Value::as_str)
        .context("keyset signature missing signature_hex")?;
    let signature = parse_signature_hex(signature_hex)?;
    let public_key = parse_public_key_hex(root_pubkey_hex)?;
    let payload = keyset_payload_for_signature(json)?;
    let detached = DetachedEd25519Signature {
        key_id: 0,
        payload_len: payload.len() as u32,
        signature,
    };
    verify_detached_payload_ed25519(&payload, &detached, |_id| Some(public_key))
        .map_err(anyhow::Error::from)?;
    Ok(())
}

pub(crate) fn load_detached_verification_input(
    path: Option<&PathBuf>,
) -> Result<Option<DetachedVerificationInput>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read detached signature file {}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("invalid JSON in detached signature file {}", path.display()))?;
    if json.get("tag_hex").is_some() {
        let key_id = json
            .get("key_id")
            .and_then(serde_json::Value::as_u64)
            .context("detached signature missing numeric key_id")? as u32;
        let payload_len =
            json.get("payload_len")
                .and_then(serde_json::Value::as_u64)
                .context("detached signature missing numeric payload_len")? as u32;
        let tag_hex = json
            .get("tag_hex")
            .and_then(serde_json::Value::as_str)
            .context("detached signature missing string tag_hex")?;
        if tag_hex.len() != 32 {
            bail!("detached signature tag_hex must be 32 hex chars");
        }
        let mut tag = [0u8; 16];
        for (idx, slot) in tag.iter_mut().enumerate() {
            let start = idx * 2;
            let end = start + 2;
            *slot = u8::from_str_radix(&tag_hex[start..end], 16)
                .with_context(|| format!("invalid tag hex at bytes {}..{}", start, end))?;
        }
        return Ok(Some(DetachedVerificationInput::Mac(
            DetachedAuthSignature {
                key_id,
                payload_len,
                tag,
            },
        )));
    }
    if json.get("signature_hex").is_some() {
        let signature = load_detached_ed25519_signature(path)?;
        return Ok(Some(DetachedVerificationInput::Ed25519(signature)));
    }
    bail!("detached signature must contain tag_hex or signature_hex");
}

fn load_detached_ed25519_signature(path: &PathBuf) -> Result<DetachedEd25519Signature> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read detached signature file {}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("invalid JSON in detached signature file {}", path.display()))?;
    let key_id = json
        .get("key_id")
        .and_then(serde_json::Value::as_u64)
        .context("detached signature missing numeric key_id")? as u32;
    let payload_len = json
        .get("payload_len")
        .and_then(serde_json::Value::as_u64)
        .context("detached signature missing numeric payload_len")? as u32;
    let signature_hex = json
        .get("signature_hex")
        .and_then(serde_json::Value::as_str)
        .context("detached signature missing string signature_hex")?;
    if signature_hex.len() != 128 {
        bail!("detached signature signature_hex must be 128 hex chars");
    }
    let mut signature = [0u8; 64];
    for (idx, slot) in signature.iter_mut().enumerate() {
        let start = idx * 2;
        let end = start + 2;
        *slot = u8::from_str_radix(&signature_hex[start..end], 16)
            .with_context(|| format!("invalid signature hex at bytes {}..{}", start, end))?;
    }
    Ok(DetachedEd25519Signature {
        key_id,
        payload_len,
        signature,
    })
}

pub(crate) fn auth_sign(
    payload: &[u8],
    output: PathBuf,
    auth_key_hex: &str,
    auth_key_id: u32,
) -> Result<()> {
    let key = parse_auth_key_hex(auth_key_hex)?;
    let sig = sign_detached_payload(payload, &key, auth_key_id);
    let tag_hex = sig
        .tag
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    fs::write(
        &output,
        serde_json::to_string_pretty(&serde_json::json!({
            "key_id": sig.key_id,
            "payload_len": sig.payload_len,
            "tag_hex": tag_hex
        }))?,
    )
    .with_context(|| format!("failed to write detached signature to {}", output.display()))?;
    Ok(())
}

pub(crate) fn auth_sign_ed25519(
    payload: &[u8],
    output: PathBuf,
    signing_key_hex: &str,
    key_id: u32,
) -> Result<()> {
    let key = parse_auth_key_hex(signing_key_hex)?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&key);
    let sig = sign_detached_payload_ed25519(payload, &signing_key, key_id);
    let signature_hex = sig
        .signature
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    fs::write(
        &output,
        serde_json::to_string_pretty(&serde_json::json!({
            "key_id": sig.key_id,
            "payload_len": sig.payload_len,
            "signature_hex": signature_hex
        }))?,
    )
    .with_context(|| format!("failed to write detached signature to {}", output.display()))?;
    Ok(())
}

pub(crate) fn auth_verify_ed25519(
    payload: &[u8],
    signature_path: PathBuf,
    public_key_hex: &str,
    key_id: u32,
) -> Result<()> {
    let signature = load_detached_ed25519_signature(&signature_path)?;
    let pubkey = parse_public_key_hex(public_key_hex)?;
    let mut result = serde_json::json!({
        "verified": false,
        "key_id": signature.key_id,
        "error": serde_json::Value::Null
    });
    match verify_detached_payload_ed25519(payload, &signature, |id| {
        if id == key_id { Some(pubkey) } else { None }
    }) {
        Ok(()) => {
            result["verified"] = serde_json::json!(true);
            result["error"] = serde_json::Value::Null;
        }
        Err(error) => {
            result["verified"] = serde_json::json!(false);
            result["error"] = serde_json::json!(error.to_string());
        }
    }
    println!("{result}");
    Ok(())
}

pub(crate) fn keyset_inspect(path: PathBuf) -> Result<()> {
    let json = load_keyset_json(&path)?;
    validate_keyset_json(&json)?;
    let issuer = json
        .get("issuer")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let created_at = json
        .get("created_at")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let expires_at = json
        .get("expires_at")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let keys = json
        .get("keys")
        .and_then(serde_json::Value::as_array)
        .map(std::vec::Vec::len)
        .unwrap_or(0);
    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "version": 1,
            "issuer": issuer,
            "created_at": created_at,
            "expires_at": expires_at,
            "key_count": keys
        })
    );
    Ok(())
}

pub(crate) fn keyset_validate(path: PathBuf, root_pubkey_hex: Option<&str>) -> Result<()> {
    let json = load_keyset_json(&path)?;
    validate_keyset_json(&json)?;
    let now = OffsetDateTime::now_utc();
    let created = OffsetDateTime::parse(
        json.get("created_at")
            .and_then(serde_json::Value::as_str)
            .context("keyset missing string created_at")?,
        &Rfc3339,
    )
    .context("keyset created_at must be RFC3339 UTC")?;
    let expires = OffsetDateTime::parse(
        json.get("expires_at")
            .and_then(serde_json::Value::as_str)
            .context("keyset missing string expires_at")?,
        &Rfc3339,
    )
    .context("keyset expires_at must be RFC3339 UTC")?;
    if now < created {
        bail!("keyset not yet valid (created_at is in the future)");
    }
    if now > expires {
        bail!("keyset expired");
    }
    if let Some(root_pubkey_hex) = root_pubkey_hex {
        verify_keyset_signature(&json, root_pubkey_hex)?;
    }
    println!("{}", serde_json::json!({ "ok": true, "valid": true }));
    Ok(())
}

pub(crate) fn keyset_sign_ed25519(
    input: PathBuf,
    output: PathBuf,
    signing_key_hex: &str,
    signed_by: &str,
) -> Result<()> {
    let mut json = load_keyset_json(&input)?;
    validate_keyset_json(&json)?;
    let payload = keyset_payload_for_signature(&json)?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&parse_auth_key_hex(signing_key_hex)?);
    let signature = sign_detached_payload_ed25519(&payload, &signing_key, 0);
    let signature_hex = signature
        .signature
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    json["signature"] = serde_json::json!({
        "signature_alg": "ed25519",
        "signed_by": signed_by,
        "signature_hex": signature_hex
    });
    fs::write(&output, serde_json::to_string_pretty(&json)?)
        .with_context(|| format!("failed to write signed keyset {}", output.display()))?;
    Ok(())
}

pub(crate) fn keyset_verify(path: PathBuf, root_pubkey_hex: &str) -> Result<()> {
    let json = load_keyset_json(&path)?;
    validate_keyset_json(&json)?;
    verify_keyset_signature(&json, root_pubkey_hex)?;
    println!("{}", serde_json::json!({ "ok": true, "verified": true }));
    Ok(())
}
