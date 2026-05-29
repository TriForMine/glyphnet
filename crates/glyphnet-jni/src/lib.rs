use base64::Engine as _;
use glyphnet_core::TransmissionMode;
use glyphnet_encode::{Encoder, EncoderConfig};
use glyphnet_render::SvgRenderer;
use glyphnet_scanner::scan_still_with_diagnostics;
use image::{DynamicImage, RgbaImage};
use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::jstring;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct ScanStillRequest {
    mode: Option<String>,
    #[serde(rename = "verifyKeyHex")]
    verify_key_hex: Option<String>,
    #[serde(rename = "verifyKeyId")]
    verify_key_id: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
    #[serde(rename = "rgbaBase64")]
    rgba_base64: Option<String>,
    #[serde(rename = "imageBase64")]
    image_base64: Option<String>,
    #[serde(rename = "roiX")]
    roi_x: Option<f32>,
    #[serde(rename = "roiY")]
    roi_y: Option<f32>,
    #[serde(rename = "roiW")]
    roi_w: Option<f32>,
    #[serde(rename = "roiH")]
    roi_h: Option<f32>,
}

fn make_java_string(env: JNIEnv<'_>, value: &str) -> jstring {
    match env.new_string(value) {
        Ok(s) => s.into_raw(),
        Err(_) => core::ptr::null_mut(),
    }
}

fn mode_from_json(mode: Option<&str>) -> TransmissionMode {
    match mode {
        Some("screen") | Some("Screen") => TransmissionMode::Screen,
        Some("burst") | Some("Burst") => TransmissionMode::Burst,
        _ => TransmissionMode::Print,
    }
}

fn error_json(
    mode: TransmissionMode,
    verify_key_provided: bool,
    verify_key_id: Option<u32>,
    error: &str,
) -> String {
    json!({
        "ok": false,
        "error": error,
        "mode": format!("{mode:?}"),
        "verify_key_provided": verify_key_provided,
        "verify_key_id": verify_key_id
    })
    .to_string()
}

fn apply_normalized_roi(
    image: DynamicImage,
    roi_x: Option<f32>,
    roi_y: Option<f32>,
    roi_w: Option<f32>,
    roi_h: Option<f32>,
) -> DynamicImage {
    let (x, y, w, h) = match (roi_x, roi_y, roi_w, roi_h) {
        (Some(x), Some(y), Some(w), Some(h)) => (x, y, w, h),
        _ => return image,
    };
    if !(0.0..1.0).contains(&w) || !(0.0..1.0).contains(&h) || w <= 0.0 || h <= 0.0 {
        return image;
    }

    let img_w = image.width();
    let img_h = image.height();
    let left = (x.clamp(0.0, 1.0) * img_w as f32).round() as u32;
    let top = (y.clamp(0.0, 1.0) * img_h as f32).round() as u32;
    let mut crop_w = (w.clamp(0.0, 1.0) * img_w as f32).round() as u32;
    let mut crop_h = (h.clamp(0.0, 1.0) * img_h as f32).round() as u32;
    if left >= img_w || top >= img_h {
        return image;
    }
    crop_w = crop_w.min(img_w.saturating_sub(left)).max(1);
    crop_h = crop_h.min(img_h.saturating_sub(top)).max(1);
    image.crop_imm(left, top, crop_w, crop_h)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_expo_modules_glyphnetscanner_GlyphNetNativeBridge_encodeSvgNative(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    payload: JString<'_>,
) -> jstring {
    let payload: String = match env.get_string(&payload) {
        Ok(v) => v.into(),
        Err(error) => {
            let err = json!({
                "ok": false,
                "error": format!("invalid_payload_string: {error}")
            })
            .to_string();
            return make_java_string(env, &err);
        }
    };

    let encoded = match Encoder::new(EncoderConfig::default()).encode_static(payload.as_bytes()) {
        Ok(v) => v,
        Err(error) => {
            let err = json!({
                "ok": false,
                "error": format!("encode_failed: {error}")
            })
            .to_string();
            return make_java_string(env, &err);
        }
    };

    match SvgRenderer::default().render(&encoded.matrix) {
        Ok(svg) => {
            let svg_with_white_bg = if let Some(tag_end) = svg.find('>') {
                let mut out = String::with_capacity(svg.len() + 64);
                out.push_str(&svg[..=tag_end]);
                out.push_str(r##"<rect width="100%" height="100%" fill="#FFFFFF"/>"##);
                out.push_str(&svg[tag_end + 1..]);
                out
            } else {
                svg
            };
            make_java_string(env, &svg_with_white_bg)
        }
        Err(error) => {
            let err = json!({
                "ok": false,
                "error": format!("render_failed: {error}")
            })
            .to_string();
            make_java_string(env, &err)
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_expo_modules_glyphnetscanner_GlyphNetNativeBridge_scanStillNative(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    request_json: JString<'_>,
) -> jstring {
    let request_json: String = match env.get_string(&request_json) {
        Ok(v) => v.into(),
        Err(error) => {
            let err = json!({
                "ok": false,
                "error": format!("invalid_request_json: {error}")
            })
            .to_string();
            return make_java_string(env, &err);
        }
    };

    let parsed = match serde_json::from_str::<ScanStillRequest>(&request_json) {
        Ok(v) => v,
        Err(error) => {
            let err = json!({
                "ok": false,
                "error": format!("invalid_request_json: {error}")
            })
            .to_string();
            return make_java_string(env, &err);
        }
    };

    let mode = mode_from_json(parsed.mode.as_deref());
    let verify_key_provided = parsed.verify_key_hex.is_some();
    let verify_key_id = parsed.verify_key_id;

    let image = if let Some(image_base64) = parsed.image_base64 {
        let bytes = match base64::engine::general_purpose::STANDARD.decode(image_base64.as_bytes())
        {
            Ok(v) => v,
            Err(error) => {
                let err = error_json(
                    mode,
                    verify_key_provided,
                    verify_key_id,
                    &format!("invalid_image_base64: {error}"),
                );
                return make_java_string(env, &err);
            }
        };

        match image::load_from_memory(&bytes) {
            Ok(v) => v,
            Err(error) => {
                let err = error_json(
                    mode,
                    verify_key_provided,
                    verify_key_id,
                    &format!("invalid_image_data: {error}"),
                );
                return make_java_string(env, &err);
            }
        }
    } else {
        let width = match parsed.width {
            Some(v) => v,
            None => {
                let err = error_json(mode, verify_key_provided, verify_key_id, "missing_width");
                return make_java_string(env, &err);
            }
        };
        let height = match parsed.height {
            Some(v) => v,
            None => {
                let err = error_json(mode, verify_key_provided, verify_key_id, "missing_height");
                return make_java_string(env, &err);
            }
        };
        let rgba_base64 = match parsed.rgba_base64 {
            Some(v) => v,
            None => {
                let err = error_json(
                    mode,
                    verify_key_provided,
                    verify_key_id,
                    "missing_rgba_base64",
                );
                return make_java_string(env, &err);
            }
        };

        let rgba = match base64::engine::general_purpose::STANDARD.decode(rgba_base64.as_bytes()) {
            Ok(v) => v,
            Err(error) => {
                let err = error_json(
                    mode,
                    verify_key_provided,
                    verify_key_id,
                    &format!("invalid_rgba_base64: {error}"),
                );
                return make_java_string(env, &err);
            }
        };

        let expected = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .unwrap_or(0) as usize;
        if rgba.len() != expected {
            let err = error_json(
                mode,
                verify_key_provided,
                verify_key_id,
                &format!(
                    "invalid_rgba_length: expected {expected}, got {}",
                    rgba.len()
                ),
            );
            return make_java_string(env, &err);
        }

        match RgbaImage::from_raw(width, height, rgba) {
            Some(v) => DynamicImage::ImageRgba8(v),
            None => {
                let err = error_json(
                    mode,
                    verify_key_provided,
                    verify_key_id,
                    "failed_to_construct_rgba_image",
                );
                return make_java_string(env, &err);
            }
        }
    };

    let cropped = apply_normalized_roi(
        image,
        parsed.roi_x,
        parsed.roi_y,
        parsed.roi_w,
        parsed.roi_h,
    );
    let response = match scan_still_with_diagnostics(&cropped, mode) {
        Ok(scanned) => json!({
            "ok": true,
            "payload_utf8_lossy": String::from_utf8_lossy(&scanned.decoded.decoded.frame.payload),
            "payload_len": scanned.decoded.decoded.frame.payload.len(),
            "mode": scanned.decoded.decoded.frame.header.mode.to_string(),
            "auto": {
                "module_px": scanned.decoded.info.module_px,
                "quiet_zone_modules": scanned.decoded.info.quiet_zone_modules,
                "threshold": scanned.decoded.info.threshold,
                "layout": format!("{:?}", scanned.decoded.info.layout),
            }
        })
        .to_string(),
        Err(failed) => json!({
            "ok": false,
            "error": failed.error.to_string(),
            "mode": format!("{mode:?}"),
        })
        .to_string(),
    };

    make_java_string(env, &response)
}
