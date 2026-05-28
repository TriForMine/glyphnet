//! WebAssembly-facing API for GlyphNet.

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod auth;

use glyphnet_core::{LayoutFamily, ProfileId, TransmissionMode, profile_spec};
use glyphnet_decode::decode_authenticated_payload;
use glyphnet_encode::{Encoder, EncoderConfig};
use glyphnet_render::{RasterRenderer, RenderOptions, SvgRenderer};
use glyphnet_scanner::{
    CameraFrame, FailedStillScan, ScanAttempt, ScanTimings, Scanner, ScannerConfig,
    scan_still_with_diagnostics,
};
use image::{DynamicImage, ImageEncoder, RgbaImage};

/// Encode bytes and return the symbol descriptor as JSON.
pub fn descriptor_json(payload: &[u8]) -> Result<String, String> {
    let encoded = Encoder::default()
        .encode_static(payload)
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&encoded.descriptor).map_err(|error| error.to_string())
}

/// Encode bytes with authenticity envelope and return the symbol descriptor as JSON.
pub fn descriptor_json_authenticated(
    payload: &[u8],
    auth_key: &[u8; 32],
    key_id: u32,
) -> Result<String, String> {
    let encoded = Encoder::default()
        .encode_static_authenticated(payload, auth_key, key_id)
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&encoded.descriptor).map_err(|error| error.to_string())
}

/// Encode bytes and return an SVG document.
pub fn encode_svg_string(payload: &[u8]) -> Result<String, String> {
    let encoded = Encoder::default()
        .encode_static(payload)
        .map_err(|error| error.to_string())?;
    SvgRenderer::default()
        .render(&encoded.matrix)
        .map_err(|error| error.to_string())
}

/// Encode bytes with authenticity envelope and return an SVG document.
pub fn encode_svg_string_authenticated(
    payload: &[u8],
    auth_key: &[u8; 32],
    key_id: u32,
) -> Result<String, String> {
    let encoded = Encoder::default()
        .encode_static_authenticated(payload, auth_key, key_id)
        .map_err(|error| error.to_string())?;
    SvgRenderer::default()
        .render(&encoded.matrix)
        .map_err(|error| error.to_string())
}

/// Encode bytes using explicit module size and quiet zone.
pub fn encode_svg_with_geometry(
    payload: &[u8],
    module_px: u32,
    quiet_zone_modules: u32,
) -> Result<String, String> {
    let encoded = Encoder::new(EncoderConfig::default())
        .encode_static(payload)
        .map_err(|error| error.to_string())?;
    let renderer = SvgRenderer::new(RenderOptions {
        module_px,
        quiet_zone_modules,
        ..RenderOptions::default()
    });
    renderer
        .render(&encoded.matrix)
        .map_err(|error| error.to_string())
}

/// Encode bytes using explicit module size and quiet zone, returning PNG bytes.
pub fn encode_png_with_geometry(
    payload: &[u8],
    module_px: u32,
    quiet_zone_modules: u32,
) -> Result<Vec<u8>, String> {
    encode_png_with_layout_geometry(
        payload,
        LayoutFamily::RibbonWeave,
        module_px,
        quiet_zone_modules,
    )
}

/// Encode bytes using an explicit layout, module size, and quiet zone, returning PNG bytes.
pub fn encode_png_with_layout_geometry(
    payload: &[u8],
    layout: LayoutFamily,
    module_px: u32,
    quiet_zone_modules: u32,
) -> Result<Vec<u8>, String> {
    let encoded = Encoder::new(EncoderConfig {
        layout,
        ..EncoderConfig::default()
    })
    .encode_static(payload)
    .map_err(|error| error.to_string())?;
    let renderer = RasterRenderer::new(RenderOptions {
        module_px,
        quiet_zone_modules,
        ..RenderOptions::default()
    });
    let image = renderer
        .render(&encoded.matrix)
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ColorType::Rgba8.into(),
        )
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// Encode bytes with authenticity envelope using explicit layout and geometry, returning PNG bytes.
pub fn encode_png_with_layout_geometry_authenticated(
    payload: &[u8],
    auth_key: &[u8; 32],
    key_id: u32,
    layout: LayoutFamily,
    module_px: u32,
    quiet_zone_modules: u32,
) -> Result<Vec<u8>, String> {
    let encoded = Encoder::new(EncoderConfig {
        layout,
        ..EncoderConfig::default()
    })
    .encode_static_authenticated(payload, auth_key, key_id)
    .map_err(|error| error.to_string())?;
    let renderer = RasterRenderer::new(RenderOptions {
        module_px,
        quiet_zone_modules,
        ..RenderOptions::default()
    });
    let image = renderer
        .render(&encoded.matrix)
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ColorType::Rgba8.into(),
        )
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// Scan RGBA pixels and return scanner diagnostics as JSON.
pub fn scan_rgba_json(
    rgba: &[u8],
    width: u32,
    height: u32,
    mode: TransmissionMode,
) -> Result<String, String> {
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "image dimensions overflow".to_string())? as usize;
    if rgba.len() != expected {
        return Err(format!(
            "invalid RGBA buffer length: expected {expected}, got {}",
            rgba.len()
        ));
    }
    let image = RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| "failed to construct RGBA image".to_string())?;
    let image = DynamicImage::ImageRgba8(image);
    match scan_still_with_diagnostics(&image, mode) {
        Ok(scanned) => {
            let telemetry = scanned.telemetry();
            let crop = scanned.crop.map(|region| {
                serde_json::json!({
                    "x": region.x,
                    "y": region.y,
                    "width": region.width,
                    "height": region.height
                })
            });
            let quad = scanned.quad.map(|quad| {
                serde_json::json!({
                    "top_left": { "x": quad.top_left.x, "y": quad.top_left.y },
                    "top_right": { "x": quad.top_right.x, "y": quad.top_right.y },
                    "bottom_right": { "x": quad.bottom_right.x, "y": quad.bottom_right.y },
                    "bottom_left": { "x": quad.bottom_left.x, "y": quad.bottom_left.y }
                })
            });
            let warp = scanned.warp_size.map(|(width, height)| {
                serde_json::json!({
                    "width": width,
                    "height": height
                })
            });
            let attempts = attempts_json(&scanned.attempts);
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": true,
                "payload_utf8_lossy": String::from_utf8_lossy(&scanned.decoded.decoded.frame.payload),
                "payload_len": scanned.decoded.decoded.frame.payload.len(),
                "stream_id": scanned.decoded.decoded.frame.header.stream_id,
                "frame_index": scanned.decoded.decoded.frame.header.frame_index,
                "frame_count": scanned.decoded.decoded.frame.header.frame_count,
                "mode": scanned.decoded.decoded.frame.header.mode.to_string(),
                "ecc": scanned.decoded.decoded.frame.header.ecc_level.to_string(),
                "auto": {
                    "module_px": scanned.decoded.info.module_px,
                    "quiet_zone_modules": scanned.decoded.info.quiet_zone_modules,
                    "threshold": scanned.decoded.info.threshold,
                    "layout": format!("{:?}", scanned.decoded.info.layout)
                },
                "recovery": {
                    "attempted": scanned.decoded.decoded.recovery.attempted,
                    "recovered": scanned.decoded.decoded.recovery.recovered,
                    "attempts": scanned.decoded.decoded.recovery.attempts,
                    "method": format!("{:?}", scanned.decoded.decoded.recovery.method),
                    "suspect_count": scanned.decoded.decoded.recovery.suspect_count,
                    "max_attempts_exceeded": scanned.decoded.decoded.recovery.max_attempts_exceeded
                },
                "crop": crop,
                "quad": quad,
                "warp": warp,
                "timings": timings_json(scanned.timings),
                "scan_telemetry": {
                    "candidate_count": telemetry.candidate_count,
                    "failed_candidates": telemetry.failed_candidates,
                    "burst_progress": {
                        "frame_count": scanned.decoded.decoded.frame.header.frame_count,
                        "received_frames": 1,
                        "missing_frames": usize::from(scanned.decoded.decoded.frame.header.frame_count.saturating_sub(1))
                    }
                },
                "candidate_count": attempts.len(),
                "attempts": attempts
            }))
            .map_err(|error| error.to_string())
        }
        Err(failed) => failed_scan_json(failed).map_err(|error| error.to_string()),
    }
}

/// Scan RGBA pixels and verify an embedded authenticity envelope with a supplied key.
pub fn scan_rgba_json_with_verification(
    rgba: &[u8],
    width: u32,
    height: u32,
    mode: TransmissionMode,
    verify_key: &[u8; 32],
    verify_key_id: u32,
) -> Result<String, String> {
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "image dimensions overflow".to_string())? as usize;
    if rgba.len() != expected {
        return Err(format!(
            "invalid RGBA buffer length: expected {expected}, got {}",
            rgba.len()
        ));
    }
    let image = RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| "failed to construct RGBA image".to_string())?;
    let image = DynamicImage::ImageRgba8(image);
    match scan_still_with_diagnostics(&image, mode) {
        Ok(scanned) => {
            let decoded =
                decode_authenticated_payload(&scanned.decoded.decoded.frame.payload, |id| {
                    if id == verify_key_id {
                        Some(*verify_key)
                    } else {
                        None
                    }
                });
            let auth = auth::verify_payload_with_optional_key(
                &scanned.decoded.decoded.frame.payload,
                Some(*verify_key),
                Some(verify_key_id),
            )?
            .unwrap_or_else(|| {
                serde_json::json!({
                    "verified": false,
                    "key_id": serde_json::Value::Null,
                    "error": "payload is not authenticated",
                    "reason": "unsigned_payload"
                })
            });
            match decoded {
                Ok(payload) => serde_json::to_string_pretty(&serde_json::json!({
                    "ok": true,
                    "payload_utf8_lossy": String::from_utf8_lossy(&payload),
                    "payload_len": payload.len(),
                    "auth": auth
                }))
                .map_err(|error| error.to_string()),
                Err(_) => serde_json::to_string_pretty(&serde_json::json!({
                    "ok": false,
                    "error": "auth verification failed",
                    "auth": auth
                }))
                .map_err(|error| error.to_string()),
            }
        }
        Err(failed) => failed_scan_json(failed).map_err(|error| error.to_string()),
    }
}

/// Stateful burst scanner session for incremental frame ingest.
#[derive(Debug)]
pub struct BurstScanSession {
    scanner: Scanner,
    frame_counter: u64,
    verify_key: Option<[u8; 32]>,
    verify_key_id: Option<u32>,
}

impl BurstScanSession {
    /// Create a new burst scan session.
    pub fn new(mode: TransmissionMode) -> Self {
        Self::new_with_config(mode, None)
    }

    /// Create a new burst scan session with optional max decode window.
    pub fn new_with_config(mode: TransmissionMode, max_frames: Option<usize>) -> Self {
        Self::new_with_config_and_verification(mode, max_frames, None, None)
    }

    /// Create a new burst scan session with optional max decode window and authenticity verifier.
    pub fn new_with_config_and_verification(
        mode: TransmissionMode,
        max_frames: Option<usize>,
        verify_key: Option<[u8; 32]>,
        verify_key_id: Option<u32>,
    ) -> Self {
        let default_window = profile_spec(ProfileId::PulseBurst)
            .burst_max_decode_window
            .map(usize::from)
            .unwrap_or(120);
        Self {
            scanner: Scanner::new(ScannerConfig {
                mode,
                max_frames: max_frames.unwrap_or(default_window),
                ..ScannerConfig::default()
            }),
            frame_counter: 0,
            verify_key,
            verify_key_id,
        }
    }

    /// Scan one RGBA frame and return progress/result JSON.
    pub fn scan_rgba_frame_json(
        &mut self,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<String, String> {
        let expected = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "image dimensions overflow".to_string())?
            as usize;
        if rgba.len() != expected {
            return Err(format!(
                "invalid RGBA buffer length: expected {expected}, got {}",
                rgba.len()
            ));
        }
        let image = RgbaImage::from_raw(width, height, rgba.to_vec())
            .ok_or_else(|| "failed to construct RGBA image".to_string())?;
        let image = DynamicImage::ImageRgba8(image);
        let event = self
            .scanner
            .scan_frame(CameraFrame {
                image,
                timestamp_micros: self.frame_counter,
            })
            .map_err(|error| error.to_string())?;
        self.frame_counter = self.frame_counter.saturating_add(1);
        let mut payload = serde_json::json!({
            "ok": true,
            "stream_id": event.frame.header.stream_id,
            "frame_index": event.frame.header.frame_index,
            "frame_count": event.frame.header.frame_count,
            "complete": event.complete_payload.is_some(),
            "burst_progress": {
                "frame_count": event.burst_progress.frame_count,
                "received_frames": event.burst_progress.received_frames,
                "missing_frames": event.burst_progress.missing_frames
            }
        });
        if let Some(raw_payload) = event.complete_payload {
            payload["payload_utf8_lossy"] =
                serde_json::json!(String::from_utf8_lossy(&raw_payload).to_string());
            payload["payload_len"] = serde_json::json!(raw_payload.len());
            if let Some(auth) = self.verify_auth_payload(&raw_payload)? {
                payload["auth"] = auth;
            }
        } else {
            payload["payload_utf8_lossy"] = serde_json::Value::Null;
            payload["payload_len"] = serde_json::Value::Null;
        }
        serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())
    }

    fn verify_auth_payload(&self, payload: &[u8]) -> Result<Option<serde_json::Value>, String> {
        auth::verify_payload_with_optional_key(payload, self.verify_key, self.verify_key_id)
    }
}

fn failed_scan_json(failed: FailedStillScan) -> serde_json::Result<String> {
    let attempts = attempts_json(&failed.attempts);
    serde_json::to_string_pretty(&serde_json::json!({
            "ok": false,
            "error": failed.error.to_string(),
            "timings": timings_json(failed.timings),
            "candidate_count": attempts.len(),
            "attempts": attempts
    }))
}

fn attempts_json(attempts: &[ScanAttempt]) -> Vec<serde_json::Value> {
    attempts
        .iter()
        .map(|attempt| {
            serde_json::json!({
                "detector": attempt.detector,
                "layout_hint": attempt.layout_hint.map(|layout| format!("{layout:?}")),
                "stage": attempt.stage,
                "region": {
                    "x": attempt.region.x,
                    "y": attempt.region.y,
                    "width": attempt.region.width,
                    "height": attempt.region.height
                },
                "decoded": attempt.decoded,
                "duration_micros": attempt.duration_micros,
                "duration_ms": attempt.duration_micros as f64 / 1000.0,
                "error": attempt.error
            })
        })
        .collect()
}

fn timings_json(timings: ScanTimings) -> serde_json::Value {
    serde_json::json!({
        "total_micros": timings.total_micros,
        "total_ms": timings.total_micros as f64 / 1000.0,
        "full_frame_ms": timings.full_frame_micros as f64 / 1000.0,
        "grayscale_ms": timings.grayscale_micros as f64 / 1000.0,
        "threshold_ms": timings.threshold_micros as f64 / 1000.0,
        "quad_ms": timings.quad_micros as f64 / 1000.0,
        "candidate_ms": timings.candidate_micros as f64 / 1000.0,
        "decode_attempts_ms": timings.decode_attempts_micros as f64 / 1000.0
    })
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn mode_from_str(mode: &str) -> Result<TransmissionMode, String> {
    match mode {
        "print" | "Print" => Ok(TransmissionMode::Print),
        "screen" | "Screen" => Ok(TransmissionMode::Screen),
        "burst" | "Burst" => Ok(TransmissionMode::Burst),
        _ => Err(format!("unknown scan mode: {mode}")),
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn layout_from_str(layout: &str) -> Result<LayoutFamily, String> {
    match layout {
        "ribbon" | "ribbon-weave" | "RibbonWeave" => Ok(LayoutFamily::RibbonWeave),
        "matrix" | "Matrix" => Ok(LayoutFamily::Matrix),
        "spectral" | "spectral-mesh" | "SpectralMesh" => Ok(LayoutFamily::SpectralMesh),
        "pulse" | "pulse-stream" | "PulseStream" => Ok(LayoutFamily::PulseStream),
        "constellation" | "Constellation" => Ok(LayoutFamily::Constellation),
        "frame-grid" | "FrameGrid" => Ok(LayoutFamily::FrameGrid),
        "hexagonal" | "Hexagonal" => Ok(LayoutFamily::Hexagonal),
        "radial" | "Radial" => Ok(LayoutFamily::Radial),
        _ => Err(format!("unknown layout: {layout}")),
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    #![allow(unreachable_pub)]

    use wasm_bindgen::prelude::*;

    /// Encode a UTF-8 string into a GlyphNet SVG document.
    #[wasm_bindgen(js_name = encodeSvg)]
    pub fn encode_svg(input: &str) -> Result<String, JsValue> {
        crate::encode_svg_string(input.as_bytes()).map_err(|error| JsValue::from_str(&error))
    }

    /// Encode a UTF-8 string and return the symbol descriptor JSON.
    #[wasm_bindgen(js_name = descriptorJson)]
    pub fn descriptor_json(input: &str) -> Result<String, JsValue> {
        crate::descriptor_json(input.as_bytes()).map_err(|error| JsValue::from_str(&error))
    }

    /// Build a detached authenticity signature JSON sidecar.
    #[wasm_bindgen(js_name = signDetachedAuth)]
    pub fn sign_detached_auth(
        input: &str,
        auth_key_hex: &str,
        key_id: u32,
    ) -> Result<String, JsValue> {
        let auth_key = crate::auth::parse_auth_key_hex(auth_key_hex)
            .map_err(|error| JsValue::from_str(&error))?;
        crate::auth::sign_detached_auth_json(input.as_bytes(), &auth_key, key_id)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Verify detached authenticity signature JSON against payload and keyring JSON.
    #[wasm_bindgen(js_name = verifyDetachedAuth)]
    pub fn verify_detached_auth(
        input: &str,
        signature_json: &str,
        keyring_json: &str,
    ) -> Result<String, JsValue> {
        crate::auth::verify_detached_auth_json(input.as_bytes(), signature_json, keyring_json)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Build a detached Ed25519 authenticity signature JSON sidecar.
    #[wasm_bindgen(js_name = signDetachedEd25519)]
    pub fn sign_detached_ed25519(
        input: &str,
        signing_key_hex: &str,
        key_id: u32,
    ) -> Result<String, JsValue> {
        let signing_key = crate::auth::parse_auth_key_hex(signing_key_hex)
            .map_err(|error| JsValue::from_str(&error))?;
        crate::auth::sign_detached_ed25519_json(input.as_bytes(), &signing_key, key_id)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Verify detached Ed25519 signature JSON against payload and keyring JSON.
    #[wasm_bindgen(js_name = verifyDetachedEd25519)]
    pub fn verify_detached_ed25519(
        input: &str,
        signature_json: &str,
        keyring_json: &str,
    ) -> Result<String, JsValue> {
        crate::auth::verify_detached_ed25519_json(input.as_bytes(), signature_json, keyring_json)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Encode a UTF-8 string with authenticity envelope and return symbol descriptor JSON.
    #[wasm_bindgen(js_name = descriptorJsonAuthenticated)]
    pub fn descriptor_json_authenticated(
        input: &str,
        auth_key_hex: &str,
        key_id: u32,
    ) -> Result<String, JsValue> {
        let auth_key = crate::auth::parse_auth_key_hex(auth_key_hex)
            .map_err(|error| JsValue::from_str(&error))?;
        crate::descriptor_json_authenticated(input.as_bytes(), &auth_key, key_id)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Encode a UTF-8 string into SVG using explicit geometry.
    #[wasm_bindgen(js_name = encodeSvgWithGeometry)]
    pub fn encode_svg_with_geometry(
        input: &str,
        module_px: u32,
        quiet_zone_modules: u32,
    ) -> Result<String, JsValue> {
        crate::encode_svg_with_geometry(input.as_bytes(), module_px, quiet_zone_modules)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Encode a UTF-8 string with authenticity envelope into a GlyphNet SVG document.
    #[wasm_bindgen(js_name = encodeSvgAuthenticated)]
    pub fn encode_svg_authenticated(
        input: &str,
        auth_key_hex: &str,
        key_id: u32,
    ) -> Result<String, JsValue> {
        let auth_key = crate::auth::parse_auth_key_hex(auth_key_hex)
            .map_err(|error| JsValue::from_str(&error))?;
        crate::encode_svg_string_authenticated(input.as_bytes(), &auth_key, key_id)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Encode a UTF-8 string into PNG bytes using explicit geometry.
    #[wasm_bindgen(js_name = encodePngWithGeometry)]
    pub fn encode_png_with_geometry(
        input: &str,
        module_px: u32,
        quiet_zone_modules: u32,
    ) -> Result<Vec<u8>, JsValue> {
        crate::encode_png_with_geometry(input.as_bytes(), module_px, quiet_zone_modules)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Encode a UTF-8 string into PNG bytes using explicit layout and geometry.
    #[wasm_bindgen(js_name = encodePngWithLayoutGeometry)]
    pub fn encode_png_with_layout_geometry(
        input: &str,
        layout: &str,
        module_px: u32,
        quiet_zone_modules: u32,
    ) -> Result<Vec<u8>, JsValue> {
        let layout = crate::layout_from_str(layout).map_err(|error| JsValue::from_str(&error))?;
        crate::encode_png_with_layout_geometry(
            input.as_bytes(),
            layout,
            module_px,
            quiet_zone_modules,
        )
        .map_err(|error| JsValue::from_str(&error))
    }

    /// Encode UTF-8 into PNG bytes using explicit layout/geometry and an authenticity envelope.
    #[wasm_bindgen(js_name = encodePngWithLayoutGeometryAuthenticated)]
    pub fn encode_png_with_layout_geometry_authenticated(
        input: &str,
        auth_key_hex: &str,
        key_id: u32,
        layout: &str,
        module_px: u32,
        quiet_zone_modules: u32,
    ) -> Result<Vec<u8>, JsValue> {
        let auth_key = crate::auth::parse_auth_key_hex(auth_key_hex)
            .map_err(|error| JsValue::from_str(&error))?;
        let layout = crate::layout_from_str(layout).map_err(|error| JsValue::from_str(&error))?;
        crate::encode_png_with_layout_geometry_authenticated(
            input.as_bytes(),
            &auth_key,
            key_id,
            layout,
            module_px,
            quiet_zone_modules,
        )
        .map_err(|error| JsValue::from_str(&error))
    }

    /// Scan browser ImageData RGBA bytes and return scanner diagnostics JSON.
    #[wasm_bindgen(js_name = scanRgbaJson)]
    pub fn scan_rgba_json(
        rgba: &[u8],
        width: u32,
        height: u32,
        mode: &str,
    ) -> Result<String, JsValue> {
        let mode = crate::mode_from_str(mode).map_err(|error| JsValue::from_str(&error))?;
        crate::scan_rgba_json(rgba, width, height, mode).map_err(|error| JsValue::from_str(&error))
    }

    /// Scan browser ImageData RGBA bytes and verify payload authenticity.
    #[wasm_bindgen(js_name = scanRgbaJsonWithVerification)]
    pub fn scan_rgba_json_with_verification(
        rgba: &[u8],
        width: u32,
        height: u32,
        mode: &str,
        verify_key_hex: &str,
        verify_key_id: u32,
    ) -> Result<String, JsValue> {
        let mode = crate::mode_from_str(mode).map_err(|error| JsValue::from_str(&error))?;
        let verify_key = crate::auth::parse_auth_key_hex(verify_key_hex)
            .map_err(|error| JsValue::from_str(&error))?;
        crate::scan_rgba_json_with_verification(
            rgba,
            width,
            height,
            mode,
            &verify_key,
            verify_key_id,
        )
        .map_err(|error| JsValue::from_str(&error))
    }

    /// Stateful burst scan session for incremental frame ingest.
    #[wasm_bindgen(js_name = BurstScanSession)]
    pub struct WasmBurstScanSession {
        inner: crate::BurstScanSession,
    }

    #[wasm_bindgen(js_class = BurstScanSession)]
    impl WasmBurstScanSession {
        /// Create a session with scan mode (print|screen|burst).
        #[wasm_bindgen(constructor)]
        pub fn new(mode: &str) -> Result<WasmBurstScanSession, JsValue> {
            let mode = crate::mode_from_str(mode).map_err(|error| JsValue::from_str(&error))?;
            Ok(Self {
                inner: crate::BurstScanSession::new(mode),
            })
        }

        /// Create a session with explicit max frame window.
        #[wasm_bindgen(js_name = withConfig)]
        pub fn with_config(mode: &str, max_frames: u32) -> Result<WasmBurstScanSession, JsValue> {
            let mode = crate::mode_from_str(mode).map_err(|error| JsValue::from_str(&error))?;
            Ok(Self {
                inner: crate::BurstScanSession::new_with_config(mode, Some(max_frames as usize)),
            })
        }

        /// Create a session with explicit max frame window and authenticity verification key.
        #[wasm_bindgen(js_name = withConfigAndVerification)]
        pub fn with_config_and_verification(
            mode: &str,
            max_frames: u32,
            verify_key_hex: &str,
            verify_key_id: u32,
        ) -> Result<WasmBurstScanSession, JsValue> {
            let mode = crate::mode_from_str(mode).map_err(|error| JsValue::from_str(&error))?;
            let verify_key = crate::auth::parse_auth_key_hex(verify_key_hex)
                .map_err(|error| JsValue::from_str(&error))?;
            Ok(Self {
                inner: crate::BurstScanSession::new_with_config_and_verification(
                    mode,
                    Some(max_frames as usize),
                    Some(verify_key),
                    Some(verify_key_id),
                ),
            })
        }

        /// Scan one frame and return progress/result JSON.
        #[wasm_bindgen(js_name = scanRgbaFrameJson)]
        pub fn scan_rgba_frame_json(
            &mut self,
            rgba: &[u8],
            width: u32,
            height: u32,
        ) -> Result<String, JsValue> {
            self.inner
                .scan_rgba_frame_json(rgba, width, height)
                .map_err(|error| JsValue::from_str(&error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_svg_api_returns_document() {
        let svg = encode_svg_string(b"browser").unwrap();
        assert!(svg.starts_with("<svg"));
    }

    #[test]
    fn native_scan_api_reports_decode_failure_as_json() {
        let rgba = vec![255; 16 * 16 * 4];
        let json = scan_rgba_json(&rgba, 16, 16, TransmissionMode::Print).unwrap();
        assert!(json.contains(r#""ok": false"#));
    }

    #[test]
    fn native_scan_api_decodes_rendered_symbol() {
        let encoded = Encoder::default().encode_static(b"wasm scan").unwrap();
        let image = glyphnet_render::RasterRenderer::default()
            .render(&encoded.matrix)
            .unwrap();
        let json = scan_rgba_json(
            image.as_raw(),
            image.width(),
            image.height(),
            TransmissionMode::Print,
        )
        .unwrap();
        assert!(json.contains(r#""ok": true"#));
        assert!(json.contains("wasm scan"));
        assert!(json.contains(r#""scan_telemetry""#));
        assert!(json.contains(r#""recovery""#));
    }

    #[test]
    fn native_png_api_returns_decodable_symbol() {
        let png = encode_png_with_geometry(b"debug sample", 4, 4).unwrap();
        let image = image::load_from_memory(&png).unwrap().into_rgba8();
        let json = scan_rgba_json(
            image.as_raw(),
            image.width(),
            image.height(),
            TransmissionMode::Print,
        )
        .unwrap();
        assert!(json.contains(r#""ok": true"#));
        assert!(json.contains("debug sample"));
    }

    #[test]
    fn native_png_api_returns_decodable_matrix_symbol() {
        let png =
            encode_png_with_layout_geometry(b"matrix sample", LayoutFamily::Matrix, 4, 4).unwrap();
        let image = image::load_from_memory(&png).unwrap().into_rgba8();
        let json = scan_rgba_json(
            image.as_raw(),
            image.width(),
            image.height(),
            TransmissionMode::Print,
        )
        .unwrap();
        assert!(json.contains(r#""ok": true"#));
        assert!(json.contains("matrix sample"));
        assert!(json.contains("Matrix"));
    }

    #[test]
    fn native_scan_api_reports_recovery_and_telemetry_contract() {
        let encoded = Encoder::default()
            .encode_static(b"screen telemetry")
            .unwrap();
        let image = glyphnet_render::RasterRenderer::default()
            .render(&encoded.matrix)
            .unwrap();
        let json = scan_rgba_json(
            image.as_raw(),
            image.width(),
            image.height(),
            TransmissionMode::Print,
        )
        .unwrap();
        assert!(json.contains(r#""ok": true"#));
        assert!(json.contains("screen telemetry"));
        assert!(json.contains(r#""scan_telemetry""#));
        assert!(json.contains(r#""candidate_count""#));
        assert!(json.contains(r#""failed_candidates""#));
        assert!(json.contains(r#""recovery""#));
        assert!(json.contains(r#""method""#));
    }

    #[test]
    fn native_burst_session_reports_progress_and_completion() {
        let encoder = Encoder::new(EncoderConfig {
            mode: TransmissionMode::Burst,
            max_frame_payload: 3,
            ..EncoderConfig::default()
        });
        let frames = encoder.encode_burst(b"burst-session").unwrap();
        let mut session = BurstScanSession::new(TransmissionMode::Burst);

        let first = glyphnet_render::RasterRenderer::default()
            .render(&frames[0].matrix)
            .unwrap();
        let first_json = session
            .scan_rgba_frame_json(first.as_raw(), first.width(), first.height())
            .unwrap();
        assert!(first_json.contains(r#""complete": false"#));
        assert!(first_json.contains(r#""received_frames": 1"#));

        let mut final_json = String::new();
        for frame in frames.into_iter().skip(1) {
            let image = glyphnet_render::RasterRenderer::default()
                .render(&frame.matrix)
                .unwrap();
            final_json = session
                .scan_rgba_frame_json(image.as_raw(), image.width(), image.height())
                .unwrap();
        }
        assert!(final_json.contains(r#""complete": true"#));
        assert!(final_json.contains("burst-session"));
    }

    #[test]
    fn native_burst_session_with_verification_still_completes() {
        let key = [0x55u8; 32];
        let encoder = Encoder::new(EncoderConfig {
            mode: TransmissionMode::Burst,
            max_frame_payload: 3,
            ..EncoderConfig::default()
        });
        let frames = encoder.encode_burst(b"burst-auth").unwrap();
        let mut session = BurstScanSession::new_with_config_and_verification(
            TransmissionMode::Burst,
            None,
            Some(key),
            Some(17),
        );

        let mut final_json = String::new();
        for frame in frames {
            let image = glyphnet_render::RasterRenderer::default()
                .render(&frame.matrix)
                .unwrap();
            final_json = session
                .scan_rgba_frame_json(image.as_raw(), image.width(), image.height())
                .unwrap();
        }
        assert!(final_json.contains(r#""complete": true"#));
    }

    #[test]
    fn native_authenticated_png_roundtrip_verifies() {
        let key = [0x44u8; 32];
        let png = encode_png_with_layout_geometry_authenticated(
            b"wasm-auth",
            &key,
            99,
            LayoutFamily::RibbonWeave,
            4,
            4,
        )
        .unwrap();
        let image = image::load_from_memory(&png).unwrap().into_rgba8();
        let json = scan_rgba_json_with_verification(
            image.as_raw(),
            image.width(),
            image.height(),
            TransmissionMode::Print,
            &key,
            99,
        )
        .unwrap();
        assert!(json.contains(r#""ok": true"#));
        assert!(json.contains(r#""verified": true"#));
        assert!(json.contains("wasm-auth"));
    }

    #[test]
    fn native_detached_signature_verifies_with_matching_key() {
        let key = [0x33u8; 32];
        let sig = auth::sign_detached_auth_json(b"detached-ok", &key, 11).unwrap();
        let keyring = r#"[{"key_id":11,"key_hex":"3333333333333333333333333333333333333333333333333333333333333333"}]"#;
        let result = auth::verify_detached_auth_json(b"detached-ok", &sig, keyring).unwrap();
        assert!(result.contains(r#""verified": true"#));
    }

    #[test]
    fn native_detached_signature_fails_with_wrong_key() {
        let key = [0x33u8; 32];
        let sig = auth::sign_detached_auth_json(b"detached-wrong", &key, 11).unwrap();
        let keyring = r#"[{"key_id":11,"key_hex":"4444444444444444444444444444444444444444444444444444444444444444"}]"#;
        let result = auth::verify_detached_auth_json(b"detached-wrong", &sig, keyring).unwrap();
        assert!(result.contains(r#""verified": false"#));
    }

    #[test]
    fn native_detached_signature_supports_rotated_key_ids() {
        let key = [0x77u8; 32];
        let sig = auth::sign_detached_auth_json(b"detached-rotate", &key, 42).unwrap();
        let keyring = r#"[{"key_id":1,"key_hex":"1111111111111111111111111111111111111111111111111111111111111111"},{"key_id":42,"key_hex":"7777777777777777777777777777777777777777777777777777777777777777"}]"#;
        let result = auth::verify_detached_auth_json(b"detached-rotate", &sig, keyring).unwrap();
        assert!(result.contains(r#""verified": true"#));
        assert!(result.contains(r#""key_id": 42"#));
    }

    #[test]
    fn native_unsigned_payload_reports_no_auth_block_in_regular_scan() {
        let encoded = Encoder::default().encode_static(b"unsigned").unwrap();
        let image = glyphnet_render::RasterRenderer::default()
            .render(&encoded.matrix)
            .unwrap();
        let json = scan_rgba_json(
            image.as_raw(),
            image.width(),
            image.height(),
            TransmissionMode::Print,
        )
        .unwrap();
        assert!(json.contains(r#""ok": true"#));
        assert!(!json.contains(r#""auth""#));
    }

    #[test]
    fn native_detached_ed25519_signature_roundtrip() {
        let signing_key = [0xA5u8; 32];
        let signature = auth::sign_detached_ed25519_json(b"ed25519-ok", &signing_key, 21).unwrap();
        let verifying_key = ed25519_dalek::SigningKey::from_bytes(&signing_key)
            .verifying_key()
            .to_bytes();
        let key_hex = verifying_key
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let keyring = serde_json::json!([{ "key_id": 21, "key_hex": key_hex }]).to_string();
        let result =
            auth::verify_detached_ed25519_json(b"ed25519-ok", &signature, &keyring).unwrap();
        assert!(result.contains(r#""verified": true"#));
    }
}
