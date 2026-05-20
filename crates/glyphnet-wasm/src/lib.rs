//! WebAssembly-facing API for GlyphNet.

use glyphnet_encode::{Encoder, EncoderConfig};
use glyphnet_render::{RenderOptions, SvgRenderer};

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_svg_api_returns_document() {
        let svg = encode_svg_string(b"browser").unwrap();
        assert!(svg.starts_with("<svg"));
    }
}
