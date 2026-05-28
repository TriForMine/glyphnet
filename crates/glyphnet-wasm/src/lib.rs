//! WebAssembly-facing API for GlyphNet.

use glyphnet_core::{LayoutFamily, ProfileId, TransmissionMode, profile_spec};
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

/// Stateful burst scanner session for incremental frame ingest.
#[derive(Debug)]
pub struct BurstScanSession {
    scanner: Scanner,
    frame_counter: u64,
}

impl BurstScanSession {
    /// Create a new burst scan session.
    pub fn new(mode: TransmissionMode) -> Self {
        Self::new_with_config(mode, None)
    }

    /// Create a new burst scan session with optional max decode window.
    pub fn new_with_config(mode: TransmissionMode, max_frames: Option<usize>) -> Self {
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
        serde_json::to_string_pretty(&serde_json::json!({
            "ok": true,
            "stream_id": event.frame.header.stream_id,
            "frame_index": event.frame.header.frame_index,
            "frame_count": event.frame.header.frame_count,
            "complete": event.complete_payload.is_some(),
            "burst_progress": {
                "frame_count": event.burst_progress.frame_count,
                "received_frames": event.burst_progress.received_frames,
                "missing_frames": event.burst_progress.missing_frames
            },
            "payload_utf8_lossy": event.complete_payload.as_ref().map(|payload| String::from_utf8_lossy(payload).to_string()),
            "payload_len": event.complete_payload.as_ref().map(std::vec::Vec::len),
        }))
        .map_err(|error| error.to_string())
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
}
