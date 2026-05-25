//! WebAssembly-facing API for GlyphNet.

use glyphnet_core::TransmissionMode;
use glyphnet_encode::{Encoder, EncoderConfig};
use glyphnet_render::{RenderOptions, SvgRenderer};
use glyphnet_scanner::scan_still;
use image::{DynamicImage, RgbaImage};

/// Encode bytes and return the symbol descriptor as JSON.
pub fn descriptor_json(payload: &[u8]) -> Result<String, String> {
    let encoded = Encoder::default()
        .encode_static(payload)
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
    match scan_still(&image, mode) {
        Ok(scanned) => {
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
            let attempts: Vec<_> = scanned
                .attempts
                .iter()
                .map(|attempt| {
                    serde_json::json!({
                        "stage": attempt.stage,
                        "region": {
                            "x": attempt.region.x,
                            "y": attempt.region.y,
                            "width": attempt.region.width,
                            "height": attempt.region.height
                        },
                        "decoded": attempt.decoded,
                        "error": attempt.error
                    })
                })
                .collect();
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
                "crop": crop,
                "quad": quad,
                "warp": warp,
                "candidate_count": attempts.len(),
                "attempts": attempts
            }))
            .map_err(|error| error.to_string())
        }
        Err(error) => serde_json::to_string_pretty(&serde_json::json!({
            "ok": false,
            "error": error.to_string()
        }))
        .map_err(|error| error.to_string()),
    }
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
    }
}
