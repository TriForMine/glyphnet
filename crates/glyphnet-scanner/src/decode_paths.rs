use glyphnet_core::{
    Cell, Frame, FrameHeader, HEADER_LEN, LayoutFamily, SymbolMatrix, bitstream, layout,
};
use glyphnet_cv::{Point, Quad, VisionProfile, adaptive_threshold, warp_perspective_gray};
use glyphnet_decode::{
    AutoDecodeInfo, AutoDecodedSymbol, DecodeError, DecodeOptions, DecodedSymbol, RasterDecoder,
    decode_matrix,
};
use glyphnet_ecc::{RecoveryMethod, RecoveryTelemetry};
use image::{DynamicImage, GrayImage, Luma};
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ScanRegion;

pub(crate) fn decode_candidate(
    decoder: &RasterDecoder,
    image: &DynamicImage,
    candidate: crate::detectors::ScanCandidate,
    fast_mode: bool,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    let region = candidate.region;
    if matches!(
        candidate.stage,
        "signature-window" | "coarse-grid" | "photo-grid" | "centered-full-frame"
    ) {
        debug_dump_dynamic("decode_input", image);
        if let Ok(decoded) = decode_refined_ribbon_candidate(image, fast_mode) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_fractional_ribbon_no_quiet(image, fast_mode) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_thresholded_ribbon_candidate(image, fast_mode) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_exact_ribbon_candidate(image, region, fast_mode) {
            return Ok(decoded);
        }
        if !fast_mode {
            if let Ok(decoded) = decode_fractional_ribbon_candidate(image) {
                return Ok(decoded);
            }
        }
        let target_module_px = 4;
        let resized = image::imageops::resize(
            image,
            104 * target_module_px,
            44 * target_module_px,
            image::imageops::FilterType::Triangle,
        );
        let resized = DynamicImage::ImageRgba8(resized);
        debug_dump_dynamic("decode_resized_104x44", &resized);
        let normalized_region = ScanRegion {
            x: 0,
            y: 0,
            width: 104 * target_module_px,
            height: 44 * target_module_px,
        };
        if let Ok(decoded) = decode_exact_ribbon_candidate(&resized, normalized_region, fast_mode) {
            return Ok(decoded);
        }
        return Err(DecodeError::AutoDetectFailed);
    }

    if matches!(
        candidate.stage,
        "reference-sweep" | "component-reference" | "dark-bounds" | "dark-ribbon"
    ) {
        if let Ok(decoded) = decode_exact_ribbon_candidate(image, region, fast_mode) {
            return Ok(decoded);
        }
        if !fast_mode {
            if let Ok(decoded) = decode_fractional_ribbon_candidate(image) {
                return Ok(decoded);
            }
        }
    }
    decoder.decode_auto_with_info(image)
}

fn decode_refined_ribbon_candidate(
    image: &DynamicImage,
    fast_mode: bool,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    let luma = image.to_luma8();
    let profile = VisionProfile::for_mode(glyphnet_core::TransmissionMode::Print);
    let binary = adaptive_threshold(&luma, profile.threshold_radius, profile.threshold_bias)
        .map_err(|_| DecodeError::AutoDetectFailed)?;
    let Some((min_x, min_y, max_x, max_y)) = dark_bounds_ignoring_border(&binary) else {
        return Err(DecodeError::AutoDetectFailed);
    };
    let bw = max_x.saturating_sub(min_x).saturating_add(1);
    let bh = max_y.saturating_sub(min_y).saturating_add(1);
    if bw < 96 || bh < 36 {
        return Err(DecodeError::AutoDetectFailed);
    }
    let module_x = (bw / 96).max(1);
    let module_y = (bh / 36).max(1);
    let pads = [
        (0u32, 0u32),
        (module_x, module_y),
        (module_x * 2, module_y * 2),
        (module_x * 4, module_y * 4),
    ];
    for (pad_x, pad_y) in pads {
        let x0 = min_x.saturating_sub(pad_x);
        let y0 = min_y.saturating_sub(pad_y);
        let x1 = (max_x.saturating_add(pad_x)).min(luma.width().saturating_sub(1));
        let y1 = (max_y.saturating_add(pad_y)).min(luma.height().saturating_sub(1));
        let w = x1.saturating_sub(x0).saturating_add(1);
        let h = y1.saturating_sub(y0).saturating_add(1);
        if w < 96 || h < 36 {
            continue;
        }
        let cropped = image::imageops::crop_imm(image, x0, y0, w, h).to_image();
        let cropped = DynamicImage::ImageRgba8(cropped);
        debug_dump_dynamic("decode_refined_crop", &cropped);
        if let Ok(decoded) = decode_perspective_ribbon_candidate(&cropped, fast_mode) {
            return Ok(decoded);
        }
        if let Some(normalized) = normalize_ribbon_crop(&cropped, 104, 44) {
            debug_dump_dynamic("decode_refined_normalized_104x44", &normalized);
            if let Ok(decoded) = decode_exact_ribbon_candidate(
                &normalized,
                ScanRegion {
                    x: 0,
                    y: 0,
                    width: normalized.width(),
                    height: normalized.height(),
                },
                fast_mode,
            ) {
                return Ok(decoded);
            }
            if let Ok(decoded) =
                decode_fractional_ribbon_candidate_with_mode(&normalized, fast_mode)
            {
                return Ok(decoded);
            }
        }
        if let Some(normalized) = normalize_ribbon_crop(&cropped, 96, 36) {
            debug_dump_dynamic("decode_refined_normalized_96x36", &normalized);
            if let Ok(decoded) = decode_exact_ribbon_no_quiet_candidate(&normalized, fast_mode) {
                return Ok(decoded);
            }
            if let Ok(decoded) = decode_fractional_ribbon_no_quiet(&normalized, fast_mode) {
                return Ok(decoded);
            }
        }
        let region = ScanRegion {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        if let Ok(decoded) = decode_exact_ribbon_candidate(&cropped, region, fast_mode) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_fractional_ribbon_candidate_with_mode(&cropped, fast_mode) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_fractional_ribbon_no_quiet(&cropped, fast_mode) {
            return Ok(decoded);
        }
        if !fast_mode && let Ok(decoded) = decode_fractional_ribbon_candidate(&cropped) {
            return Ok(decoded);
        }
    }
    Err(DecodeError::AutoDetectFailed)
}

fn decode_perspective_ribbon_candidate(
    image: &DynamicImage,
    fast_mode: bool,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    let luma = image.to_luma8();
    if luma.width() < 96 || luma.height() < 36 {
        return Err(DecodeError::AutoDetectFailed);
    }
    let profile = VisionProfile::for_mode(glyphnet_core::TransmissionMode::Print);
    let binary = adaptive_threshold(&luma, profile.threshold_radius, profile.threshold_bias)
        .map_err(|_| DecodeError::AutoDetectFailed)?;
    let Some(quad) = estimate_ribbon_totem_quad(&binary) else {
        return Err(DecodeError::AutoDetectFailed);
    };
    for pad_modules in [0.0_f32] {
        let quad = expand_quad_by_modules(quad, 96.0, 36.0, pad_modules);
        let width_top = point_distance(quad.top_left, quad.top_right);
        let width_bottom = point_distance(quad.bottom_left, quad.bottom_right);
        let module_px = ((width_top.max(width_bottom) / 96.0).round() as u32).clamp(2, 16);
        let warp_width = 96 * module_px;
        let warp_height = 36 * module_px;
        let Ok(warped_luma) = warp_perspective_gray(&luma, quad, warp_width, warp_height) else {
            continue;
        };
        let warped = DynamicImage::ImageLuma8(warped_luma.clone());
        debug_dump_dynamic("decode_perspective_no_quiet", &warped);
        if let Ok(decoded) = decode_exact_ribbon_no_quiet_candidate(&warped, fast_mode) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_fractional_ribbon_no_quiet(&warped, fast_mode) {
            return Ok(decoded);
        }
        let Ok(binary_warped) = adaptive_threshold(
            &warped_luma,
            profile.threshold_radius,
            profile.threshold_bias,
        ) else {
            continue;
        };
        let binary_warped = DynamicImage::ImageLuma8(binary_warped);
        debug_dump_dynamic("decode_perspective_thresholded_no_quiet", &binary_warped);
        if let Ok(decoded) = decode_exact_ribbon_no_quiet_candidate(&binary_warped, fast_mode) {
            return Ok(decoded);
        }
        if let Some(luma) = binary_warped.as_luma8()
            && let Ok(decoded) = decode_shifted_no_quiet_luma(luma, module_px)
        {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_fractional_ribbon_no_quiet(&binary_warped, fast_mode) {
            return Ok(decoded);
        }
    }
    Err(DecodeError::AutoDetectFailed)
}

fn decode_shifted_no_quiet_luma(
    luma: &GrayImage,
    module_px: u32,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    const SYMBOL_WIDTH: u16 = 96;
    const SYMBOL_HEIGHT: u16 = 36;
    let integral = IntegralGray::new(luma);
    for scale_adjust in [1.0_f32, 0.95, 1.05, 0.9, 1.1] {
        let scale = module_px as f32 * scale_adjust;
        for y_step in -8..=12 {
            let origin_y = y_step as f32 * 0.5;
            for x_step in -16..=16 {
                let origin_x = x_step as f32 * 0.5;
                if !fractional_grid_fits(
                    luma,
                    origin_x,
                    origin_y,
                    scale,
                    scale,
                    SYMBOL_WIDTH,
                    SYMBOL_HEIGHT,
                ) {
                    continue;
                }
                if !fractional_header_precheck(
                    &integral,
                    FractionalSampleParams {
                        origin_x_modules: origin_x,
                        origin_y_modules: origin_y,
                        scale_x: scale,
                        scale_y: scale,
                        symbol_height: SYMBOL_HEIGHT,
                        vertical_warp: 0.0,
                    },
                    128,
                ) {
                    continue;
                }
                if let Ok(decoded) = decode_fractional_with_params(
                    &integral,
                    FractionalDecodeParams {
                        sample: FractionalSampleParams {
                            origin_x_modules: origin_x,
                            origin_y_modules: origin_y,
                            scale_x: scale,
                            scale_y: scale,
                            symbol_height: SYMBOL_HEIGHT,
                            vertical_warp: 0.0,
                        },
                        threshold: 128,
                        quiet_zone_modules: 0,
                        allow_recovery: false,
                    },
                ) {
                    return Ok(decoded);
                }
            }
        }
    }
    Err(DecodeError::AutoDetectFailed)
}

#[derive(Debug, Clone, Copy)]
struct VerticalSignature {
    min_x: u32,
    max_x: u32,
    min_y: u32,
    max_y: u32,
    dark: u32,
    transitions: u32,
}

fn estimate_ribbon_totem_quad(binary: &GrayImage) -> Option<Quad> {
    let width = binary.width();
    let height = binary.height();
    if width < 96 || height < 36 {
        return None;
    }

    let mut columns = Vec::new();
    let y_start = (height / 64).max(1);
    let y_end = height.saturating_sub(y_start + 1);
    for x in 0..width {
        let mut dark = 0u32;
        let mut min_y = height;
        let mut max_y = 0u32;
        let mut transitions = 0u32;
        let mut last_dark = false;
        let mut seen = false;
        for y in y_start..=y_end {
            let is_dark = binary.get_pixel(x, y).0[0] == 0;
            if is_dark {
                dark += 1;
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
            if seen && is_dark != last_dark {
                transitions += 1;
            }
            seen = true;
            last_dark = is_dark;
        }
        if dark == 0 {
            continue;
        }
        let span = max_y.saturating_sub(min_y).saturating_add(1);
        if span >= height / 3
            && min_y <= height / 3
            && max_y >= height.saturating_mul(3) / 5
            && transitions >= 8
            && dark >= span / 18
        {
            columns.push(VerticalSignature {
                min_x: x,
                max_x: x,
                min_y,
                max_y,
                dark,
                transitions,
            });
        }
    }

    let mut groups: Vec<VerticalSignature> = Vec::new();
    for column in columns {
        match groups.last_mut() {
            Some(group) if column.min_x <= group.max_x + 8 => {
                group.max_x = column.max_x;
                group.min_y = group.min_y.min(column.min_y);
                group.max_y = group.max_y.max(column.max_y);
                group.dark += column.dark;
                group.transitions += column.transitions;
            }
            _ => groups.push(column),
        }
    }
    if groups.len() < 2 {
        debug_log_decode(format!("perspective groups insufficient: {}", groups.len()));
        return None;
    }
    debug_log_decode(format!("perspective groups: {:?}", groups));

    let left_zone_max = width.saturating_mul(2) / 5;
    let right_zone_min = width.saturating_mul(3) / 5;
    let min_separation = width / 2;
    let edge_left = groups
        .iter()
        .copied()
        .filter(|group| {
            signature_center_x(*group) <= left_zone_max
                && group.dark >= 300
                && group.transitions >= 80
        })
        .min_by_key(|group| signature_center_x(*group));
    let edge_right = groups
        .iter()
        .copied()
        .filter(|group| {
            signature_center_x(*group) >= right_zone_min
                && group.dark >= 300
                && group.transitions >= 80
        })
        .max_by_key(|group| signature_center_x(*group));

    let edge_pair = edge_left.zip(edge_right).and_then(|(left, right)| {
        (right.max_x.saturating_sub(left.min_x) >= min_separation).then_some((left, right))
    });

    let mut best_pair: Option<(VerticalSignature, VerticalSignature, u64)> = None;
    for (left_index, left) in groups.iter().enumerate() {
        if signature_center_x(*left) > left_zone_max {
            continue;
        }
        for right in groups.iter().skip(left_index + 1) {
            if signature_center_x(*right) < right_zone_min {
                continue;
            }
            if right.max_x.saturating_sub(left.min_x) < min_separation {
                continue;
            }
            let left_span = left.max_y.saturating_sub(left.min_y).saturating_add(1);
            let right_span = right.max_y.saturating_sub(right.min_y).saturating_add(1);
            let separation = right.max_x.saturating_sub(left.min_x);
            let score = u64::from(separation)
                .saturating_mul(u64::from(separation))
                .saturating_mul(u64::from(left_span.min(right_span)).max(1))
                .saturating_mul(u64::from(left.dark.min(right.dark)).max(1))
                .saturating_mul(u64::from(left.transitions.min(right.transitions)).max(1));
            if best_pair.is_none_or(|(_, _, best_score)| score > best_score) {
                best_pair = Some((*left, *right, score));
            }
        }
    }
    let (left, right) = edge_pair.or_else(|| best_pair.map(|(left, right, _)| (left, right)))?;
    let quad = ribbon_symbol_quad_from_signature_lines(binary, left, right)
        .unwrap_or_else(|| ribbon_symbol_quad_from_totem_refs(binary, left, right));
    debug_log_decode(format!(
        "perspective picked left={left:?} right={right:?} quad=({:.1},{:.1}) ({:.1},{:.1}) ({:.1},{:.1}) ({:.1},{:.1})",
        quad.top_left.x,
        quad.top_left.y,
        quad.top_right.x,
        quad.top_right.y,
        quad.bottom_right.x,
        quad.bottom_right.y,
        quad.bottom_left.x,
        quad.bottom_left.y,
    ));
    Some(quad)
}

fn signature_center_x(signature: VerticalSignature) -> u32 {
    signature.min_x.saturating_add(signature.max_x) / 2
}

#[derive(Debug, Clone, Copy)]
struct HorizontalSignature {
    min_x: u32,
    max_x: u32,
    min_y: u32,
    max_y: u32,
    dark: u32,
    transitions: u32,
}

fn ribbon_symbol_quad_from_signature_lines(
    binary: &GrayImage,
    left: VerticalSignature,
    right: VerticalSignature,
) -> Option<Quad> {
    let horizontal = horizontal_signatures(binary);
    debug_log_decode(format!("perspective horizontal groups: {:?}", horizontal));
    let top = horizontal
        .iter()
        .copied()
        .filter(|group| horizontal_center_y(*group) <= binary.height() / 3)
        .min_by_key(|group| horizontal_center_y(*group))?;
    let bottom = horizontal
        .iter()
        .copied()
        .filter(|group| horizontal_center_y(*group) >= binary.height() / 2)
        .max_by_key(|group| horizontal_center_y(*group))?;

    let left_line = fit_signature_center_line(binary, left)?;
    let right_line = fit_signature_center_line(binary, right)?;
    let top_line =
        horizontal_line_at_y(fit_horizontal_signature_line(binary, top)?, top, top.min_y);
    let bottom_line = horizontal_line_at_y(
        fit_horizontal_signature_line(binary, bottom)?,
        bottom,
        bottom.max_y,
    );

    let ref_tl = intersect_vertical_horizontal(left_line, top_line)?;
    let ref_tr = intersect_vertical_horizontal(right_line, top_line)?;
    let ref_bl = intersect_vertical_horizontal(left_line, bottom_line)?;
    let ref_br = intersect_vertical_horizontal(right_line, bottom_line)?;

    // Reference intersections are approximately at x=6/x=89 and y=3/y=32.
    let top_module = point_scale(point_sub(ref_tr, ref_tl), 1.0 / 83.0);
    let bottom_module = point_scale(point_sub(ref_br, ref_bl), 1.0 / 83.0);
    let left_module_y = point_scale(point_sub(ref_bl, ref_tl), 1.0 / 29.0);
    let right_module_y = point_scale(point_sub(ref_br, ref_tr), 1.0 / 29.0);

    Some(Quad {
        top_left: point_add(
            point_add(ref_tl, point_scale(top_module, -6.0)),
            point_scale(left_module_y, -3.0),
        ),
        top_right: point_add(
            point_add(ref_tr, point_scale(top_module, 6.0)),
            point_scale(right_module_y, -3.0),
        ),
        bottom_right: point_add(
            point_add(ref_br, point_scale(bottom_module, 6.0)),
            point_scale(right_module_y, 3.0),
        ),
        bottom_left: point_add(
            point_add(ref_bl, point_scale(bottom_module, -6.0)),
            point_scale(left_module_y, 3.0),
        ),
    })
}

fn horizontal_signatures(binary: &GrayImage) -> Vec<HorizontalSignature> {
    let width = binary.width();
    let height = binary.height();
    let mut rows = Vec::new();
    for y in 0..height {
        let mut dark = 0u32;
        let mut min_x = width;
        let mut max_x = 0u32;
        let mut transitions = 0u32;
        let mut last_dark = false;
        let mut seen = false;
        for x in 0..width {
            let is_dark = binary.get_pixel(x, y).0[0] == 0;
            if is_dark {
                dark += 1;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
            }
            if seen && is_dark != last_dark {
                transitions += 1;
            }
            seen = true;
            last_dark = is_dark;
        }
        if dark == 0 {
            continue;
        }
        let span = max_x.saturating_sub(min_x).saturating_add(1);
        if span >= width / 2 && transitions >= 28 && dark >= span / 20 {
            rows.push(HorizontalSignature {
                min_x,
                max_x,
                min_y: y,
                max_y: y,
                dark,
                transitions,
            });
        }
    }

    let mut groups: Vec<HorizontalSignature> = Vec::new();
    for row in rows {
        match groups.last_mut() {
            Some(group) if row.min_y <= group.max_y + 6 => {
                group.max_y = row.max_y;
                group.min_x = group.min_x.min(row.min_x);
                group.max_x = group.max_x.max(row.max_x);
                group.dark += row.dark;
                group.transitions += row.transitions;
            }
            _ => groups.push(row),
        }
    }
    groups
}

fn horizontal_center_y(signature: HorizontalSignature) -> u32 {
    signature.min_y.saturating_add(signature.max_y) / 2
}

fn horizontal_line_at_y(
    line: HorizontalLine,
    signature: HorizontalSignature,
    y: u32,
) -> HorizontalLine {
    let center_x = (signature.min_x + signature.max_x) as f32 * 0.5;
    HorizontalLine {
        slope: line.slope,
        intercept: y as f32 - line.slope * center_x,
    }
}

fn ribbon_symbol_quad_from_totem_refs(
    binary: &GrayImage,
    left: VerticalSignature,
    right: VerticalSignature,
) -> Quad {
    let left_line = fit_signature_center_line(binary, left);
    let right_line = fit_signature_center_line(binary, right);
    let left_top = point_on_signature(left_line, left, left.min_y);
    let left_bottom = point_on_signature(left_line, left, left.max_y);
    let right_top = point_on_signature(right_line, right, right.min_y);
    let right_bottom = point_on_signature(right_line, right, right.max_y);

    // RibbonWeave side totems sit near module x=5 and x=90, spanning y=3..32.
    // Expand those reference lines back to the full 96x36 no-quiet symbol.
    let top_module = point_scale(point_sub(right_top, left_top), 1.0 / 85.0);
    let bottom_module = point_scale(point_sub(right_bottom, left_bottom), 1.0 / 85.0);
    let left_module_y = point_scale(point_sub(left_bottom, left_top), 1.0 / 29.0);
    let right_module_y = point_scale(point_sub(right_bottom, right_top), 1.0 / 29.0);

    Quad {
        top_left: point_add(
            point_add(left_top, point_scale(top_module, -5.0)),
            point_scale(left_module_y, -3.0),
        ),
        top_right: point_add(
            point_add(right_top, point_scale(top_module, 5.0)),
            point_scale(right_module_y, -3.0),
        ),
        bottom_right: point_add(
            point_add(right_bottom, point_scale(bottom_module, 5.0)),
            point_scale(right_module_y, 3.0),
        ),
        bottom_left: point_add(
            point_add(left_bottom, point_scale(bottom_module, -5.0)),
            point_scale(left_module_y, 3.0),
        ),
    }
}

#[derive(Debug, Clone, Copy)]
struct VerticalLine {
    slope: f32,
    intercept: f32,
}

#[derive(Debug, Clone, Copy)]
struct HorizontalLine {
    slope: f32,
    intercept: f32,
}

fn fit_signature_center_line(
    binary: &GrayImage,
    signature: VerticalSignature,
) -> Option<VerticalLine> {
    let mut count = 0.0_f32;
    let mut sum_y = 0.0_f32;
    let mut sum_x = 0.0_f32;
    let mut sum_yy = 0.0_f32;
    let mut sum_yx = 0.0_f32;
    for y in signature.min_y..=signature.max_y {
        let mut min_x = binary.width();
        let mut max_x = 0u32;
        let mut found = false;
        for x in signature.min_x..=signature.max_x {
            if binary.get_pixel(x, y).0[0] == 0 {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                found = true;
            }
        }
        if !found {
            continue;
        }
        let x = (min_x + max_x) as f32 * 0.5;
        let y = y as f32;
        count += 1.0;
        sum_y += y;
        sum_x += x;
        sum_yy += y * y;
        sum_yx += y * x;
    }
    if count < 8.0 {
        return None;
    }
    let denominator = count * sum_yy - sum_y * sum_y;
    if denominator.abs() < f32::EPSILON {
        return None;
    }
    let slope = (count * sum_yx - sum_y * sum_x) / denominator;
    let intercept = (sum_x - slope * sum_y) / count;
    Some(VerticalLine { slope, intercept })
}

fn point_on_signature(line: Option<VerticalLine>, signature: VerticalSignature, y: u32) -> Point {
    let fallback_x = (signature.min_x + signature.max_x) as f32 * 0.5;
    let y = y as f32;
    Point {
        x: line
            .map(|line| line.slope * y + line.intercept)
            .unwrap_or(fallback_x),
        y,
    }
}

fn fit_horizontal_signature_line(
    binary: &GrayImage,
    signature: HorizontalSignature,
) -> Option<HorizontalLine> {
    let y_start = signature.min_y.saturating_sub(3);
    let y_end = (signature.max_y + 3).min(binary.height().saturating_sub(1));
    let mut count = 0.0_f32;
    let mut sum_x = 0.0_f32;
    let mut sum_y = 0.0_f32;
    let mut sum_xx = 0.0_f32;
    let mut sum_xy = 0.0_f32;
    for x in signature.min_x..=signature.max_x {
        let mut min_y = binary.height();
        let mut max_y = 0u32;
        let mut found = false;
        for y in y_start..=y_end {
            if binary.get_pixel(x, y).0[0] == 0 {
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                found = true;
            }
        }
        if !found {
            continue;
        }
        let x = x as f32;
        let y = (min_y + max_y) as f32 * 0.5;
        count += 1.0;
        sum_x += x;
        sum_y += y;
        sum_xx += x * x;
        sum_xy += x * y;
    }
    if count < 16.0 {
        return None;
    }
    let denominator = count * sum_xx - sum_x * sum_x;
    if denominator.abs() < f32::EPSILON {
        return None;
    }
    let slope = (count * sum_xy - sum_x * sum_y) / denominator;
    let intercept = (sum_y - slope * sum_x) / count;
    Some(HorizontalLine { slope, intercept })
}

fn intersect_vertical_horizontal(
    vertical: VerticalLine,
    horizontal: HorizontalLine,
) -> Option<Point> {
    let denominator = 1.0 - horizontal.slope * vertical.slope;
    if denominator.abs() < 1.0e-4 {
        return None;
    }
    let y = (horizontal.slope * vertical.intercept + horizontal.intercept) / denominator;
    let x = vertical.slope * y + vertical.intercept;
    Some(Point { x, y })
}

fn expand_quad_by_modules(quad: Quad, symbol_width: f32, symbol_height: f32, modules: f32) -> Quad {
    if modules == 0.0 {
        return quad;
    }
    let x_fraction = modules / symbol_width;
    let y_fraction = modules / symbol_height;
    Quad {
        top_left: expand_quad_point(
            quad.top_left,
            quad.top_right,
            quad.bottom_left,
            x_fraction,
            y_fraction,
        ),
        top_right: expand_quad_point(
            quad.top_right,
            quad.top_left,
            quad.bottom_right,
            x_fraction,
            y_fraction,
        ),
        bottom_right: expand_quad_point(
            quad.bottom_right,
            quad.bottom_left,
            quad.top_right,
            x_fraction,
            y_fraction,
        ),
        bottom_left: expand_quad_point(
            quad.bottom_left,
            quad.bottom_right,
            quad.top_left,
            x_fraction,
            y_fraction,
        ),
    }
}

fn expand_quad_point(
    point: Point,
    horizontal: Point,
    vertical: Point,
    x_fraction: f32,
    y_fraction: f32,
) -> Point {
    Point {
        x: point.x + (point.x - horizontal.x) * x_fraction + (point.x - vertical.x) * y_fraction,
        y: point.y + (point.y - horizontal.y) * x_fraction + (point.y - vertical.y) * y_fraction,
    }
}

fn point_distance(a: Point, b: Point) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

fn point_add(a: Point, b: Point) -> Point {
    Point {
        x: a.x + b.x,
        y: a.y + b.y,
    }
}

fn point_sub(a: Point, b: Point) -> Point {
    Point {
        x: a.x - b.x,
        y: a.y - b.y,
    }
}

fn point_scale(point: Point, scale: f32) -> Point {
    Point {
        x: point.x * scale,
        y: point.y * scale,
    }
}

fn normalize_ribbon_crop(
    image: &DynamicImage,
    total_width_modules: u32,
    total_height_modules: u32,
) -> Option<DynamicImage> {
    if image.width() < total_width_modules || image.height() < total_height_modules {
        return None;
    }
    let target_aspect = total_width_modules as f32 / total_height_modules as f32;
    let current_aspect = image.width() as f32 / image.height().max(1) as f32;
    let (crop_x, crop_y, crop_w, crop_h) = if current_aspect > target_aspect {
        let crop_w = (image.height() as f32 * target_aspect).round() as u32;
        (
            (image.width().saturating_sub(crop_w)) / 2,
            0,
            crop_w,
            image.height(),
        )
    } else {
        let crop_h = (image.width() as f32 / target_aspect).round() as u32;
        (
            0,
            (image.height().saturating_sub(crop_h)) / 2,
            image.width(),
            crop_h,
        )
    };
    if crop_w < total_width_modules || crop_h < total_height_modules {
        return None;
    }
    let cropped = image::imageops::crop_imm(image, crop_x, crop_y, crop_w, crop_h).to_image();
    let module_px = ((crop_w as f32 / total_width_modules as f32)
        .min(crop_h as f32 / total_height_modules as f32))
    .round()
    .max(1.0) as u32;
    let target_w = total_width_modules.saturating_mul(module_px);
    let target_h = total_height_modules.saturating_mul(module_px);
    Some(DynamicImage::ImageRgba8(image::imageops::resize(
        &DynamicImage::ImageRgba8(cropped),
        target_w,
        target_h,
        image::imageops::FilterType::Triangle,
    )))
}

fn decode_exact_ribbon_no_quiet_candidate(
    image: &DynamicImage,
    fast_mode: bool,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    if image.width() >= 96 && image.height() >= 36 {
        let module_px = (image.width() / 96).max(1);
        if image.width() == 96 * module_px && image.height() == 36 * module_px {
            for threshold in [144, 160, 176, 192, 208, 224] {
                let exact = RasterDecoder::new(DecodeOptions {
                    module_px,
                    quiet_zone_modules: 0,
                    threshold,
                    layout: LayoutFamily::RibbonWeave,
                });
                let info = AutoDecodeInfo {
                    module_px,
                    quiet_zone_modules: 0,
                    threshold,
                    layout: LayoutFamily::RibbonWeave,
                };
                if fast_mode {
                    if let Ok(matrix) = exact.sample_matrix(image) {
                        debug_dump_module_grid(&format!("exact_no_quiet_t{threshold}"), &matrix);
                        if let Ok(decoded) = auto_decoded_from_crc_valid_matrix(matrix, info) {
                            return Ok(decoded);
                        }
                    }
                    continue;
                }
                if let Ok(decoded) = exact.decode(image) {
                    return Ok(AutoDecodedSymbol { decoded, info });
                }
            }
        }
    }
    Err(DecodeError::AutoDetectFailed)
}

fn dark_bounds_ignoring_border(binary: &GrayImage) -> Option<(u32, u32, u32, u32)> {
    if binary.width() < 16 || binary.height() < 16 {
        return None;
    }
    let ignore_x = (binary.width() / 32).max(2);
    let ignore_y = (binary.height() / 32).max(2);
    let mut min_x = binary.width();
    let mut min_y = binary.height();
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;
    for y in ignore_y..binary.height().saturating_sub(ignore_y) {
        for x in ignore_x..binary.width().saturating_sub(ignore_x) {
            if binary.get_pixel(x, y).0[0] == 0 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                found = true;
            }
        }
    }
    found.then_some((min_x, min_y, max_x, max_y))
}

fn decode_exact_ribbon_candidate(
    image: &DynamicImage,
    region: ScanRegion,
    fast_mode: bool,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    if region.width >= 104 && region.height >= 44 {
        let module_px = (region.width / 104).max(1);
        if region.width == 104 * module_px && region.height == 44 * module_px {
            for threshold in [160, 192, 224] {
                let exact = RasterDecoder::new(DecodeOptions {
                    module_px,
                    quiet_zone_modules: 4,
                    threshold,
                    layout: LayoutFamily::RibbonWeave,
                });
                let info = AutoDecodeInfo {
                    module_px,
                    quiet_zone_modules: 4,
                    threshold,
                    layout: LayoutFamily::RibbonWeave,
                };
                if fast_mode {
                    if let Ok(matrix) = exact.sample_matrix(image) {
                        debug_dump_module_grid(&format!("exact_quiet_t{threshold}"), &matrix);
                        if let Ok(decoded) = auto_decoded_from_crc_valid_matrix(matrix, info) {
                            return Ok(decoded);
                        }
                    }
                    continue;
                }
                if let Ok(decoded) = exact.decode(image) {
                    return Ok(AutoDecodedSymbol { decoded, info });
                }
            }
        }
    }
    Err(DecodeError::AutoDetectFailed)
}

fn decode_fractional_ribbon_candidate(
    image: &DynamicImage,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    decode_fractional_ribbon_candidate_with_mode(image, false)
}

fn decode_fractional_ribbon_candidate_with_mode(
    image: &DynamicImage,
    fast_mode: bool,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    const SYMBOL_WIDTH: u16 = 96;
    const SYMBOL_HEIGHT: u16 = 36;
    const TOTAL_WIDTH_MODULES: f32 = 104.0;
    const TOTAL_HEIGHT_MODULES: f32 = 44.0;
    const QUIET_MODULES: f32 = 4.0;

    let luma = image.to_luma8();
    if luma.width() < 104 || luma.height() < 44 {
        return Err(DecodeError::AutoDetectFailed);
    }
    let base_scale_x = luma.width() as f32 / TOTAL_WIDTH_MODULES;
    let base_scale_y = luma.height() as f32 / TOTAL_HEIGHT_MODULES;
    if base_scale_x < 1.0 || base_scale_y < 1.0 {
        return Err(DecodeError::AutoDetectFailed);
    }

    let otsu = fractional_threshold(&luma);
    let integral = IntegralGray::new(&luma);
    let mut thresholds = if fast_mode {
        vec![otsu, 176, 192, 208]
    } else {
        vec![otsu, 144, 160, 176, 192, 208, 224]
    };
    thresholds.sort_unstable();
    thresholds.dedup();

    let scale_adjusts_x: &[f32] = if fast_mode {
        &[1.0]
    } else {
        &[1.0, 0.99, 1.01, 0.98, 1.02]
    };
    let scale_adjusts_y: &[f32] = if fast_mode {
        &[1.0]
    } else {
        &[1.0, 0.99, 1.01, 0.98, 1.02]
    };
    let shift_radius = if fast_mode { 0 } else { 3 };
    let phase_offsets: &[f32] = if fast_mode {
        &[0.0, -0.25, 0.25]
    } else {
        &[0.0, -0.25, 0.25, -0.5, 0.5]
    };
    let vertical_warps: &[f32] = if fast_mode {
        &[0.0, -0.06, -0.12, 0.06, 0.12]
    } else {
        &[0.0]
    };

    let mut decode_trials = 0usize;
    for scale_adjust_x in scale_adjusts_x {
        for scale_adjust_y in scale_adjusts_y {
            let scale_x = base_scale_x * scale_adjust_x;
            let scale_y = base_scale_y * scale_adjust_y;
            if scale_x < 1.0 || scale_y < 1.0 {
                continue;
            }
            for y_shift in module_shifts(shift_radius) {
                for x_shift in module_shifts(shift_radius) {
                    for y_phase in phase_offsets {
                        for x_phase in phase_offsets {
                            let origin_x = QUIET_MODULES + x_shift + x_phase;
                            let origin_y = QUIET_MODULES + y_shift + y_phase;
                            if origin_x < -2.0 || origin_y < -8.0 {
                                continue;
                            }
                            if !fractional_grid_fits(
                                &luma,
                                origin_x,
                                origin_y,
                                scale_x,
                                scale_y,
                                SYMBOL_WIDTH,
                                SYMBOL_HEIGHT,
                            ) {
                                continue;
                            }
                            for &vertical_warp in vertical_warps {
                                for &threshold in &thresholds {
                                    if !fractional_header_precheck(
                                        &integral,
                                        FractionalSampleParams {
                                            origin_x_modules: origin_x,
                                            origin_y_modules: origin_y,
                                            scale_x,
                                            scale_y,
                                            symbol_height: SYMBOL_HEIGHT,
                                            vertical_warp,
                                        },
                                        threshold,
                                    ) {
                                        continue;
                                    }
                                    decode_trials += 1;
                                    if fast_mode && decode_trials > 4 {
                                        return Err(DecodeError::AutoDetectFailed);
                                    }
                                    if let Ok(decoded) = decode_fractional_with_params(
                                        &integral,
                                        FractionalDecodeParams {
                                            sample: FractionalSampleParams {
                                                origin_x_modules: origin_x,
                                                origin_y_modules: origin_y,
                                                scale_x,
                                                scale_y,
                                                symbol_height: SYMBOL_HEIGHT,
                                                vertical_warp,
                                            },
                                            threshold,
                                            quiet_zone_modules: 4,
                                            allow_recovery: !fast_mode,
                                        },
                                    ) {
                                        return Ok(decoded);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Err(DecodeError::AutoDetectFailed)
}

fn decode_fractional_ribbon_no_quiet(
    image: &DynamicImage,
    fast_mode: bool,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    debug_dump_dynamic("decode_no_quiet_input", image);
    const SYMBOL_WIDTH: u16 = 96;
    const SYMBOL_HEIGHT: u16 = 36;
    const TOTAL_WIDTH_MODULES: f32 = 96.0;
    const TOTAL_HEIGHT_MODULES: f32 = 36.0;

    let luma = image.to_luma8();
    if luma.width() < 96 || luma.height() < 36 {
        return Err(DecodeError::AutoDetectFailed);
    }
    let base_scale_x = luma.width() as f32 / TOTAL_WIDTH_MODULES;
    let base_scale_y = luma.height() as f32 / TOTAL_HEIGHT_MODULES;
    if base_scale_x < 1.0 || base_scale_y < 1.0 {
        return Err(DecodeError::AutoDetectFailed);
    }

    let otsu = fractional_threshold(&luma);
    let integral = IntegralGray::new(&luma);
    let mut thresholds = if fast_mode {
        vec![otsu, 176, 192, 208]
    } else {
        vec![otsu, 144, 160, 176, 192, 208, 224]
    };
    thresholds.sort_unstable();
    thresholds.dedup();

    let scale_adjusts_x: &[f32] = if fast_mode {
        &[1.0]
    } else {
        &[1.0, 0.99, 1.01, 0.98, 1.02]
    };
    let scale_adjusts_y: &[f32] = if fast_mode {
        &[1.0]
    } else {
        &[1.0, 0.99, 1.01, 0.98, 1.02]
    };
    let shift_radius = if fast_mode { 1 } else { 3 };
    let x_phase_offsets: &[f32] = if fast_mode {
        &[0.0]
    } else {
        &[0.0, -0.25, 0.25, -0.5, 0.5]
    };
    let y_phase_offsets: &[f32] = if fast_mode {
        &[0.0]
    } else {
        &[0.0, -0.25, 0.25, -0.5, 0.5]
    };
    let vertical_warps: &[f32] = if fast_mode {
        &[0.0, -0.06, -0.12, 0.06, 0.12]
    } else {
        &[0.0]
    };
    let mut decode_trials = 0usize;
    for scale_adjust_x in scale_adjusts_x {
        for scale_adjust_y in scale_adjusts_y {
            let scale_x = base_scale_x * scale_adjust_x;
            let scale_y = base_scale_y * scale_adjust_y;
            if scale_x < 1.0 || scale_y < 1.0 {
                continue;
            }
            for y_shift in module_shifts(shift_radius) {
                for x_shift in module_shifts(shift_radius) {
                    for y_phase in y_phase_offsets {
                        for x_phase in x_phase_offsets {
                            let origin_x = x_shift + x_phase;
                            let origin_y = y_shift + y_phase;
                            if !fractional_grid_fits(
                                &luma,
                                origin_x,
                                origin_y,
                                scale_x,
                                scale_y,
                                SYMBOL_WIDTH,
                                SYMBOL_HEIGHT,
                            ) {
                                continue;
                            }
                            for &vertical_warp in vertical_warps {
                                for &threshold in &thresholds {
                                    if !fractional_header_precheck(
                                        &integral,
                                        FractionalSampleParams {
                                            origin_x_modules: origin_x,
                                            origin_y_modules: origin_y,
                                            scale_x,
                                            scale_y,
                                            symbol_height: SYMBOL_HEIGHT,
                                            vertical_warp,
                                        },
                                        threshold,
                                    ) {
                                        continue;
                                    }
                                    decode_trials += 1;
                                    if fast_mode && decode_trials > 4 {
                                        return Err(DecodeError::AutoDetectFailed);
                                    }
                                    if let Ok(decoded) = decode_fractional_with_params(
                                        &integral,
                                        FractionalDecodeParams {
                                            sample: FractionalSampleParams {
                                                origin_x_modules: origin_x,
                                                origin_y_modules: origin_y,
                                                scale_x,
                                                scale_y,
                                                symbol_height: SYMBOL_HEIGHT,
                                                vertical_warp,
                                            },
                                            threshold,
                                            quiet_zone_modules: 0,
                                            allow_recovery: !fast_mode,
                                        },
                                    ) {
                                        return Ok(decoded);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Err(DecodeError::AutoDetectFailed)
}

fn decode_thresholded_ribbon_candidate(
    image: &DynamicImage,
    fast_mode: bool,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    let gray = image.to_luma8();
    let profile = VisionProfile::for_mode(glyphnet_core::TransmissionMode::Print);
    let binary = adaptive_threshold(&gray, profile.threshold_radius, profile.threshold_bias)
        .map_err(|_| DecodeError::AutoDetectFailed)?;
    let bin = DynamicImage::ImageLuma8(binary);
    debug_dump_dynamic("decode_thresholded_input", &bin);
    let region = ScanRegion {
        x: 0,
        y: 0,
        width: bin.width(),
        height: bin.height(),
    };
    if let Ok(decoded) = decode_exact_ribbon_candidate(&bin, region, fast_mode) {
        return Ok(decoded);
    }
    decode_fractional_ribbon_no_quiet(&bin, fast_mode)
}

#[cfg(not(target_arch = "wasm32"))]
fn debug_dump_dynamic(stem: &str, image: &DynamicImage) {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let Some(dir) = std::env::var_os("GLYPHNET_SCAN_DEBUG_DIR").map(PathBuf::from) else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let index = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("04_decode_{stem}_{index:03}.png"));
    let _ = image.save(path);
}

#[cfg(target_arch = "wasm32")]
fn debug_dump_dynamic(_stem: &str, _image: &DynamicImage) {}

fn module_shifts(radius: i32) -> impl Iterator<Item = f32> {
    (-radius * 2..=radius * 2).map(|value| value as f32 * 0.5)
}

#[derive(Clone, Copy)]
struct FractionalSampleParams {
    origin_x_modules: f32,
    origin_y_modules: f32,
    scale_x: f32,
    scale_y: f32,
    symbol_height: u16,
    vertical_warp: f32,
}

#[derive(Clone, Copy)]
struct FractionalDecodeParams {
    sample: FractionalSampleParams,
    threshold: u8,
    quiet_zone_modules: u32,
    allow_recovery: bool,
}

fn fractional_header_precheck(
    integral: &IntegralGray,
    sample: FractionalSampleParams,
    threshold: u8,
) -> bool {
    const SYMBOL_WIDTH: u16 = 96;
    const SYMBOL_HEIGHT: u16 = 36;

    let mut bits = Vec::with_capacity(HEADER_LEN * 8);
    'rows: for y in 0..SYMBOL_HEIGHT {
        for x in 0..SYMBOL_WIDTH {
            if !layout::is_data_module_for(
                LayoutFamily::RibbonWeave,
                SYMBOL_WIDTH,
                SYMBOL_HEIGHT,
                x,
                y,
            ) {
                continue;
            }
            let avg = fractional_module_luma_projected(integral, x, y, sample);
            bits.push(avg < threshold);
            if bits.len() == HEADER_LEN * 8 {
                break 'rows;
            }
        }
    }
    if bits.len() < HEADER_LEN * 8 {
        return false;
    }
    FrameHeader::decode(&bitstream::bits_to_bytes(&bits)).is_ok()
}

fn fractional_grid_fits(
    luma: &GrayImage,
    origin_x_modules: f32,
    origin_y_modules: f32,
    scale_x: f32,
    scale_y: f32,
    symbol_width: u16,
    symbol_height: u16,
) -> bool {
    let min_x = origin_x_modules * scale_x;
    let min_y = origin_y_modules * scale_y;
    let max_x = (origin_x_modules + f32::from(symbol_width)) * scale_x;
    let max_y = (origin_y_modules + f32::from(symbol_height)) * scale_y;
    min_x >= -scale_x
        && min_y >= -scale_y
        && max_x < luma.width() as f32 + scale_x
        && max_y < luma.height() as f32 + scale_y
}

fn decode_fractional_with_params(
    integral: &IntegralGray,
    params: FractionalDecodeParams,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    const SYMBOL_WIDTH: u16 = 96;
    const SYMBOL_HEIGHT: u16 = 36;

    let mut matrix =
        SymbolMatrix::with_layout(SYMBOL_WIDTH, SYMBOL_HEIGHT, LayoutFamily::RibbonWeave);
    for y in 0..SYMBOL_HEIGHT {
        for x in 0..SYMBOL_WIDTH {
            if let Some(cell) = layout::function_cell_for(
                LayoutFamily::RibbonWeave,
                SYMBOL_WIDTH,
                SYMBOL_HEIGHT,
                x,
                y,
            ) {
                matrix.set(x, y, cell)?;
                continue;
            }
            let avg = fractional_module_luma_projected(integral, x, y, params.sample);
            matrix.set(x, y, Cell::Data(avg < params.threshold))?;
        }
    }
    debug_dump_module_grid(
        &format!("fractional_modules_warp_{:.2}", params.sample.vertical_warp),
        &matrix,
    );

    let info = AutoDecodeInfo {
        module_px: params.sample.scale_x.round().max(1.0) as u32,
        quiet_zone_modules: params.quiet_zone_modules,
        threshold: params.threshold,
        layout: LayoutFamily::RibbonWeave,
    };
    if !params.allow_recovery {
        return auto_decoded_from_crc_valid_matrix(matrix, info);
    }
    let decoded = decode_matrix(&matrix)?;
    Ok(AutoDecodedSymbol { decoded, info })
}

fn auto_decoded_from_crc_valid_matrix(
    matrix: SymbolMatrix,
    info: AutoDecodeInfo,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    let sampled_bytes = bitstream::bits_to_bytes(&matrix.read_data_bits());
    let frame = Frame::decode(&sampled_bytes).map_err(|_| DecodeError::AutoDetectFailed)?;
    Ok(AutoDecodedSymbol {
        decoded: DecodedSymbol {
            matrix,
            frame,
            sampled_bytes,
            recovery: RecoveryTelemetry {
                attempted: false,
                recovered: false,
                attempts: 0,
                method: RecoveryMethod::None,
                suspect_count: 0,
                max_attempts_exceeded: false,
            },
        },
        info,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn debug_dump_module_grid(stem: &str, matrix: &SymbolMatrix) {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let Some(dir) = std::env::var_os("GLYPHNET_SCAN_DEBUG_DIR").map(PathBuf::from) else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let index = COUNTER.fetch_add(1, Ordering::Relaxed);

    let mut image = GrayImage::new(u32::from(matrix.width()), u32::from(matrix.height()));
    let mut text = String::new();
    for y in 0..matrix.height() {
        for x in 0..matrix.width() {
            let dark = matrix.get(x, y).map(|cell| cell.is_dark()).unwrap_or(false);
            image.put_pixel(
                u32::from(x),
                u32::from(y),
                Luma([if dark { 0 } else { 255 }]),
            );
            text.push(if dark { '1' } else { '0' });
        }
        text.push('\n');
    }

    let png_path = dir.join(format!("05_decode_{stem}_{index:03}.png"));
    let txt_path = dir.join(format!("05_decode_{stem}_{index:03}.txt"));
    let _ = image.save(png_path);
    let _ = std::fs::write(txt_path, text);
}

#[cfg(target_arch = "wasm32")]
fn debug_dump_module_grid(_stem: &str, _matrix: &SymbolMatrix) {}

#[cfg(not(target_arch = "wasm32"))]
fn debug_log_decode(line: impl AsRef<str>) {
    let Some(dir) = std::env::var_os("GLYPHNET_SCAN_DEBUG_DIR").map(PathBuf::from) else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("07_decode_perspective.log");
    let mut current = std::fs::read_to_string(&path).unwrap_or_default();
    current.push_str(line.as_ref());
    current.push('\n');
    let _ = std::fs::write(path, current);
}

#[cfg(target_arch = "wasm32")]
fn debug_log_decode(_line: impl AsRef<str>) {}

fn fractional_threshold(luma: &GrayImage) -> u8 {
    let mut histogram = [0u32; 256];
    for pixel in luma.pixels() {
        histogram[pixel[0] as usize] += 1;
    }
    let total = u64::from(luma.width()).saturating_mul(u64::from(luma.height()));
    if total == 0 {
        return 128;
    }
    let mut sum_total = 0u64;
    for (value, count) in histogram.iter().enumerate() {
        sum_total += value as u64 * u64::from(*count);
    }
    let mut sum_background = 0u64;
    let mut weight_background = 0u64;
    let mut best_variance = -1.0f64;
    let mut threshold = 128u8;
    for (value, count) in histogram.iter().enumerate() {
        weight_background += u64::from(*count);
        if weight_background == 0 {
            continue;
        }
        let weight_foreground = total.saturating_sub(weight_background);
        if weight_foreground == 0 {
            break;
        }
        sum_background += value as u64 * u64::from(*count);
        let mean_background = sum_background as f64 / weight_background as f64;
        let mean_foreground =
            (sum_total.saturating_sub(sum_background)) as f64 / weight_foreground as f64;
        let variance = weight_background as f64
            * weight_foreground as f64
            * (mean_background - mean_foreground).powi(2);
        if variance > best_variance {
            best_variance = variance;
            threshold = value as u8;
        }
    }
    threshold.clamp(96, 224)
}

fn fractional_module_luma(
    integral: &IntegralGray,
    module_x: f32,
    module_y: f32,
    scale_x: f32,
    scale_y: f32,
) -> u8 {
    // Ribbon modules are rendered as capsules/diamonds, so averaging the full
    // square makes dark modules look too light in photos. Sample the center.
    let left = ((module_x + 0.25) * scale_x).floor().max(0.0) as u32;
    let top = ((module_y + 0.25) * scale_y).floor().max(0.0) as u32;
    let right = ((module_x + 0.75) * scale_x).ceil().max(1.0) as u32;
    let bottom = ((module_y + 0.75) * scale_y).ceil().max(1.0) as u32;
    let right = right.saturating_sub(1);
    let bottom = bottom.saturating_sub(1);
    let mut sum = 0u32;
    let mut min_luma = u8::MAX;
    let mut count = 0u32;
    for y in top..=bottom {
        for x in left..=right {
            let value = integral.pixel(x, y);
            sum += u32::from(value);
            min_luma = min_luma.min(value);
            count += 1;
        }
    }
    if count == 0 {
        return 255;
    }
    let mean = (sum / count) as u8;
    ((u16::from(mean) + u16::from(min_luma)) / 2) as u8
}

fn fractional_module_luma_projected(
    integral: &IntegralGray,
    module_x: u16,
    module_y: u16,
    sample: FractionalSampleParams,
) -> u8 {
    let local_y = f32::from(module_y);
    let height = f32::from(sample.symbol_height).max(1.0);
    let t = local_y / height;
    let warped_y = local_y + sample.vertical_warp * t * (1.0 - t) * height;
    fractional_module_luma(
        integral,
        sample.origin_x_modules + f32::from(module_x),
        sample.origin_y_modules + warped_y,
        sample.scale_x,
        sample.scale_y,
    )
}

pub(crate) struct IntegralGray {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl IntegralGray {
    pub(crate) fn new(image: &GrayImage) -> Self {
        let width = image.width();
        let height = image.height();
        Self {
            width,
            height,
            pixels: image.as_raw().clone(),
        }
    }

    fn pixel(&self, x: u32, y: u32) -> u8 {
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        self.pixels
            .get(y as usize * self.width as usize + x as usize)
            .copied()
            .unwrap_or(255)
    }
}
