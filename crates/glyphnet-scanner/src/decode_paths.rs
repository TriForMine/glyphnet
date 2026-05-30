use glyphnet_core::{Cell, FrameHeader, HEADER_LEN, LayoutFamily, SymbolMatrix, bitstream, layout};
use glyphnet_cv::{Point, Quad, warp_perspective_gray};
use glyphnet_decode::{
    AutoDecodedSymbol, DecodeError, DecodeOptions, RasterDecoder, decode_matrix_with_suspect_bytes,
};
use image::{DynamicImage, GrayImage, Luma, Rgba, RgbaImage};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::ScanRegion;
static NORMALIZED_DEBUG_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn decode_candidate(
    decoder: &RasterDecoder,
    image: &DynamicImage,
    candidate: crate::detectors::ScanCandidate,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    let region = candidate.region;
    if matches!(candidate.layout_hint, Some(LayoutFamily::Matrix))
        && let Ok(decoded) = decode_exact_matrix_candidate(image, region)
    {
        return Ok(decoded);
    }
    if matches!(
        candidate.stage,
        "signature-window" | "coarse-grid" | "top-left-scale" | "roi-group" | "border-trim"
    ) {
        let region_area = region.width.saturating_mul(region.height);
        if let Ok(decoded) = decode_exact_ribbon_candidate(image, region) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_forced_normalized_ribbon_candidate(image, region) {
            return Ok(decoded);
        }
        if region_area <= 700_000
            && let Ok(decoded) = decode_fractional_ribbon_candidate(image)
        {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_with_padding_variants(decoder, image) {
            return Ok(decoded);
        }
        let target_module_px = 4;
        let resized = image::imageops::resize(
            image,
            104 * target_module_px,
            44 * target_module_px,
            image::imageops::FilterType::Triangle,
        );
        let resized = DynamicImage::ImageRgba8(resized);
        let normalized_region = ScanRegion {
            x: 0,
            y: 0,
            width: 104 * target_module_px,
            height: 44 * target_module_px,
        };
        if let Ok(decoded) = decode_exact_ribbon_candidate(&resized, normalized_region) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_with_padding_variants(decoder, &resized) {
            return Ok(decoded);
        }
        return Err(DecodeError::AutoDetectFailed);
    }

    if matches!(
        candidate.stage,
        "reference-sweep" | "component-reference" | "dark-bounds" | "dark-ribbon"
    ) {
        if let Ok(decoded) = decode_exact_ribbon_candidate(image, region) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_fractional_ribbon_candidate(image) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_with_padding_variants(decoder, image) {
            return Ok(decoded);
        }
    }
    decoder
        .decode_auto_with_info(image)
        .or_else(|_| decode_with_padding_variants(decoder, image))
}

fn decode_with_padding_variants(
    decoder: &RasterDecoder,
    image: &DynamicImage,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    for pad_pct in [4u32, 8, 12, 16] {
        let padded = white_pad_image(image, pad_pct);
        if let Ok(decoded) = decoder.decode_auto_with_info(&padded) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_fractional_ribbon_candidate(&padded) {
            return Ok(decoded);
        }
        let region = crate::ScanRegion {
            x: 0,
            y: 0,
            width: padded.width(),
            height: padded.height(),
        };
        if let Ok(decoded) = decode_exact_ribbon_candidate(&padded, region) {
            return Ok(decoded);
        }
    }
    Err(DecodeError::AutoDetectFailed)
}

fn white_pad_image(image: &DynamicImage, pad_pct: u32) -> DynamicImage {
    let rgba = image.to_rgba8();
    let pad_x = (rgba.width().saturating_mul(pad_pct) / 100).max(2);
    let pad_y = (rgba.height().saturating_mul(pad_pct) / 100).max(2);
    let out_w = rgba.width().saturating_add(pad_x.saturating_mul(2));
    let out_h = rgba.height().saturating_add(pad_y.saturating_mul(2));
    let mut canvas = RgbaImage::from_pixel(out_w, out_h, Rgba([255, 255, 255, 255]));
    image::imageops::overlay(&mut canvas, &rgba, i64::from(pad_x), i64::from(pad_y));
    DynamicImage::ImageRgba8(canvas)
}

fn decode_exact_matrix_candidate(
    image: &DynamicImage,
    region: ScanRegion,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    let module_candidates = matrix_module_px_candidates(region);
    if module_candidates.is_empty() {
        return Err(DecodeError::AutoDetectFailed);
    }

    for module_px in module_candidates {
        for threshold in [160, 192, 224] {
            let exact = RasterDecoder::new(DecodeOptions {
                module_px,
                quiet_zone_modules: 4,
                threshold,
                layout: LayoutFamily::Matrix,
            });
            if let Ok(decoded) = exact.decode(image) {
                return Ok(AutoDecodedSymbol {
                    decoded,
                    info: glyphnet_decode::AutoDecodeInfo {
                        module_px,
                        quiet_zone_modules: 4,
                        threshold,
                        layout: LayoutFamily::Matrix,
                    },
                });
            }
        }
    }

    Err(DecodeError::AutoDetectFailed)
}

fn matrix_module_px_candidates(region: ScanRegion) -> Vec<u32> {
    let max_dim = region.width.min(region.height);
    if max_dim < 64 {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    for module_px in 2..=24 {
        if region.width % module_px != 0 || region.height % module_px != 0 {
            continue;
        }
        let width_modules = region.width / module_px;
        let height_modules = region.height / module_px;
        if width_modules != height_modules || width_modules < 29 {
            continue;
        }
        candidates.push(module_px);
    }
    candidates.sort_unstable_by(|a, b| b.cmp(a));
    candidates
}

fn decode_exact_ribbon_candidate(
    image: &DynamicImage,
    region: ScanRegion,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    if region.width >= 104 && region.height >= 44 {
        let module_px = (region.width / 104).max(1);
        if region.width == 104 * module_px && region.height == 44 * module_px {
            for quiet_zone_modules in [0u32, 1, 2, 3, 4, 5, 6, 7, 8] {
                for threshold in [144, 160, 176, 192, 208, 224] {
                    let exact = RasterDecoder::new(DecodeOptions {
                        module_px,
                        quiet_zone_modules,
                        threshold,
                        layout: LayoutFamily::RibbonWeave,
                    });
                    if let Ok(decoded) = exact.decode(image) {
                        return Ok(AutoDecodedSymbol {
                            decoded,
                            info: glyphnet_decode::AutoDecodeInfo {
                                module_px,
                                quiet_zone_modules,
                                threshold,
                                layout: LayoutFamily::RibbonWeave,
                            },
                        });
                    }
                }
            }
        }
    }
    Err(DecodeError::AutoDetectFailed)
}

fn decode_fractional_ribbon_candidate(
    image: &DynamicImage,
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
    let mut thresholds = vec![otsu, 160, 192, 224];
    thresholds.sort_unstable();
    thresholds.dedup();

    if let Ok(decoded) = decode_normalized_dark_bounds_ribbon(&luma, otsu) {
        return Ok(decoded);
    }

    for scale_adjust in [1.0_f32, 0.985, 1.015, 0.97, 1.03] {
        let scale_x = base_scale_x * scale_adjust;
        let scale_y = base_scale_y * scale_adjust;
        if scale_x < 1.0 || scale_y < 1.0 {
            continue;
        }
        for y_shift in module_shifts(3) {
            for x_shift in module_shifts(2) {
                let origin_x = QUIET_MODULES + x_shift;
                let origin_y = QUIET_MODULES + y_shift;
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
                for &threshold in &thresholds {
                    if !fractional_header_precheck(
                        &integral, origin_x, origin_y, scale_x, scale_y, threshold,
                    ) {
                        continue;
                    }
                    if let Ok(decoded) = decode_fractional_with_params(
                        &integral, origin_x, origin_y, scale_x, scale_y, threshold,
                    ) {
                        return Ok(decoded);
                    }
                }
            }
        }
    }

    Err(DecodeError::AutoDetectFailed)
}

fn decode_forced_normalized_ribbon_candidate(
    image: &DynamicImage,
    region: ScanRegion,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    const SYMBOL_WIDTH: u32 = 96;
    const SYMBOL_HEIGHT: u32 = 36;
    const QUIET: u32 = 4;

    let x = region.x.min(image.width().saturating_sub(1));
    let y = region.y.min(image.height().saturating_sub(1));
    let max_w = image.width().saturating_sub(x);
    let max_h = image.height().saturating_sub(y);
    let w = region.width.min(max_w);
    let h = region.height.min(max_h);
    if w < 80 || h < 30 {
        return Err(DecodeError::AutoDetectFailed);
    }

    let full = image.to_luma8();
    let trims = [0.00f32, 0.06];
    let shifts = [0.00f32, -0.04, 0.04];

    let est_w = (w / 104).max(2);
    let est_h = (h / 44).max(2);
    let est = ((est_w + est_h) / 2).clamp(2, 24);
    let mut module_candidates = Vec::new();
    for d in -3i32..=3 {
        let m = (est as i32 + d).clamp(2, 24) as u32;
        if !module_candidates.contains(&m) {
            module_candidates.push(m);
        }
    }

    for trim in trims {
        let trim_x = ((w as f32) * trim).round() as u32;
        let trim_y = ((h as f32) * trim).round() as u32;
        let base_w = w.saturating_sub(trim_x.saturating_mul(2));
        let base_h = h.saturating_sub(trim_y.saturating_mul(2));
        if base_w < 60 || base_h < 24 {
            continue;
        }
        let base_x = x.saturating_add(trim_x);
        let base_y = y.saturating_add(trim_y);

        for sx in shifts {
            for sy in shifts {
                let dx = ((base_w as f32) * sx).round() as i32;
                let dy = ((base_h as f32) * sy).round() as i32;
                let rx = (base_x as i32 + dx).max(0) as u32;
                let ry = (base_y as i32 + dy).max(0) as u32;
                let rw = base_w.min(full.width().saturating_sub(rx));
                let rh = base_h.min(full.height().saturating_sub(ry));
                if rw < 60 || rh < 24 {
                    continue;
                }

                let sub = image::imageops::crop_imm(&full, rx, ry, rw, rh).to_image();
                for module_px in &module_candidates {
                    let total_w = (SYMBOL_WIDTH + QUIET * 2) * *module_px;
                    let total_h = (SYMBOL_HEIGHT + QUIET * 2) * *module_px;
                    let content_w = SYMBOL_WIDTH * *module_px;
                    let content_h = SYMBOL_HEIGHT * *module_px;
                    let resized = image::imageops::resize(
                        &sub,
                        content_w,
                        content_h,
                        image::imageops::FilterType::CatmullRom,
                    );

                    let mut thresholds = vec![fractional_threshold(&resized), 120, 136, 152];
                    thresholds.sort_unstable();
                    thresholds.dedup();

                    for threshold in thresholds {
                        let mut bin = resized.clone();
                        for px in bin.pixels_mut() {
                            px.0[0] = if px.0[0] < threshold { 0 } else { 255 };
                        }
                        dump_normalized_debug(region, threshold, &resized, &bin);
                        let mut canvas = GrayImage::from_pixel(total_w, total_h, Luma([255]));
                        image::imageops::overlay(
                            &mut canvas,
                            &bin,
                            i64::from(QUIET * *module_px),
                            i64::from(QUIET * *module_px),
                        );
                        let candidate = DynamicImage::ImageLuma8(canvas);
                        let full_region = ScanRegion {
                            x: 0,
                            y: 0,
                            width: total_w,
                            height: total_h,
                        };
                        if let Ok(decoded) = decode_exact_ribbon_candidate(&candidate, full_region) {
                            return Ok(decoded);
                        }
                        let auto = RasterDecoder::default();
                        if let Ok(decoded) = auto.decode_auto_with_info(&candidate) {
                            return Ok(decoded);
                        }
                    }
                }
            }
        }
    }

    Err(DecodeError::AutoDetectFailed)
}

fn dump_normalized_debug(region: ScanRegion, threshold: u8, resized: &GrayImage, bin: &GrayImage) {
    if std::env::var_os("GLYPHNET_SCAN_DEBUG").is_none() {
        return;
    }
    let base_dir = std::env::var_os("GLYPHNET_SCAN_DEBUG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/scan-debug"));
    let out_dir = base_dir.join("normalized");
    if fs::create_dir_all(&out_dir).is_err() {
        return;
    }
    let idx = NORMALIZED_DEBUG_COUNTER.fetch_add(1, Ordering::Relaxed);
    let prefix = format!(
        "n{:04}-{}x{}+{}+{}-t{}",
        idx, region.width, region.height, region.x, region.y, threshold
    );
    let _ = resized.save(out_dir.join(format!("{prefix}-normalized-resized.png")));
    let _ = bin.save(out_dir.join(format!("{prefix}-normalized-binary.png")));
    let mut txt = String::new();
    for y in 0..bin.height() {
        for x in 0..bin.width() {
            let value = if bin.get_pixel(x, y).0[0] == 0 { '1' } else { '0' };
            txt.push(value);
        }
        txt.push('\n');
    }
    let _ = fs::write(
        out_dir.join(format!("{prefix}-normalized-binary.txt")),
        txt,
    );
}

fn module_shifts(radius: i32) -> impl Iterator<Item = f32> {
    (-radius * 2..=radius * 2).map(|value| value as f32 * 0.5)
}

fn decode_normalized_dark_bounds_ribbon(
    luma: &GrayImage,
    otsu: u8,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    const MODULE_PX: u32 = 4;
    const SYMBOL_WIDTH: u32 = 96;
    const SYMBOL_HEIGHT: u32 = 36;
    const QUIET_MODULES: u32 = 4;
    const TOTAL_WIDTH: u32 = (SYMBOL_WIDTH + QUIET_MODULES * 2) * MODULE_PX;
    const TOTAL_HEIGHT: u32 = (SYMBOL_HEIGHT + QUIET_MODULES * 2) * MODULE_PX;

    for threshold in [otsu.saturating_sub(80).clamp(40, 72), 48, 56] {
        let Some(bounds) = dark_bounds_luma(luma, threshold) else {
            continue;
        };
        let (min_x, min_y, max_x, max_y) = bounds;
        let width = max_x.saturating_sub(min_x).saturating_add(1);
        let height = max_y.saturating_sub(min_y).saturating_add(1);
        if width < SYMBOL_WIDTH || height < SYMBOL_HEIGHT {
            continue;
        }
        let Some(quad) = dark_content_quad(luma, threshold, min_x, max_x) else {
            continue;
        };
        let mut normalized = warp_perspective_gray(
            luma,
            quad,
            SYMBOL_WIDTH * MODULE_PX,
            SYMBOL_HEIGHT * MODULE_PX,
        )
        .map_err(|_| DecodeError::AutoDetectFailed)?;
        let normalized_threshold = fractional_threshold(&normalized).clamp(96, 160);
        for pixel in normalized.pixels_mut() {
            pixel.0[0] = if pixel.0[0] < normalized_threshold {
                0
            } else {
                255
            };
        }
        let mut canvas = GrayImage::from_pixel(TOTAL_WIDTH, TOTAL_HEIGHT, Luma([255]));
        image::imageops::overlay(
            &mut canvas,
            &normalized,
            i64::from(QUIET_MODULES * MODULE_PX),
            i64::from(QUIET_MODULES * MODULE_PX),
        );
        let image = DynamicImage::ImageLuma8(canvas);
        let region = ScanRegion {
            x: 0,
            y: 0,
            width: TOTAL_WIDTH,
            height: TOTAL_HEIGHT,
        };
        if let Ok(decoded) = decode_exact_ribbon_candidate(&image, region) {
            return Ok(decoded);
        }
    }
    Err(DecodeError::AutoDetectFailed)
}

fn dark_content_quad(luma: &GrayImage, threshold: u8, min_x: u32, max_x: u32) -> Option<Quad> {
    let span = max_x.saturating_sub(min_x).saturating_add(1);
    let band = (span / 5).max(16);
    let left = vertical_dark_extent(luma, threshold, min_x, min_x.saturating_add(band))?;
    let right = vertical_dark_extent(luma, threshold, max_x.saturating_sub(band), max_x)?;
    let expand_edge = |top: u32, bottom: u32| {
        let module = bottom.saturating_sub(top).max(1) as f32 / 29.0;
        (
            (top as f32 - module * 3.0).max(0.0),
            (bottom as f32 + module * 4.0).min(luma.height().saturating_sub(1) as f32),
        )
    };
    let left = expand_edge(left.0, left.1);
    let right = expand_edge(right.0, right.1);
    Some(Quad {
        top_left: Point {
            x: min_x as f32,
            y: left.0,
        },
        top_right: Point {
            x: max_x as f32,
            y: right.0,
        },
        bottom_right: Point {
            x: max_x as f32,
            y: right.1,
        },
        bottom_left: Point {
            x: min_x as f32,
            y: left.1,
        },
    })
}

fn vertical_dark_extent(
    luma: &GrayImage,
    threshold: u8,
    start_x: u32,
    end_x: u32,
) -> Option<(u32, u32)> {
    let mut min_y = u32::MAX;
    let mut max_y = 0u32;
    let mut found = false;
    for y in 0..luma.height() {
        for x in start_x..=end_x.min(luma.width().saturating_sub(1)) {
            if luma.get_pixel(x, y)[0] < threshold {
                min_y = min_y.min(y);
                max_y = max_y.max(y);
                found = true;
            }
        }
    }
    found.then_some((min_y, max_y))
}

fn dark_bounds_luma(luma: &GrayImage, threshold: u8) -> Option<(u32, u32, u32, u32)> {
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;
    for (x, y, pixel) in luma.enumerate_pixels() {
        if pixel[0] >= threshold {
            continue;
        }
        found = true;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    found.then_some((min_x, min_y, max_x, max_y))
}

fn fractional_header_precheck(
    integral: &IntegralGray,
    origin_x_modules: f32,
    origin_y_modules: f32,
    scale_x: f32,
    scale_y: f32,
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
            let avg = fractional_module_luma(
                integral,
                origin_x_modules + f32::from(x),
                origin_y_modules + f32::from(y),
                scale_x,
                scale_y,
            );
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
    origin_x_modules: f32,
    origin_y_modules: f32,
    scale_x: f32,
    scale_y: f32,
    threshold: u8,
) -> std::result::Result<AutoDecodedSymbol, DecodeError> {
    const SYMBOL_WIDTH: u16 = 96;
    const SYMBOL_HEIGHT: u16 = 36;

    let mut matrix =
        SymbolMatrix::with_layout(SYMBOL_WIDTH, SYMBOL_HEIGHT, LayoutFamily::RibbonWeave);
    let mut bit_confidence = Vec::new();
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
            let avg = fractional_module_luma(
                integral,
                origin_x_modules + f32::from(x),
                origin_y_modules + f32::from(y),
                scale_x,
                scale_y,
            );
            matrix.set(x, y, Cell::Data(avg < threshold))?;
            bit_confidence.push(avg.abs_diff(threshold));
        }
    }

    let suspect_bytes =
        glyphnet_decode::suspect_bytes_from_confidence(&matrix, &bit_confidence, 16);
    let decoded = decode_matrix_with_suspect_bytes(&matrix, &suspect_bytes)?;
    Ok(AutoDecodedSymbol {
        decoded,
        info: glyphnet_decode::AutoDecodeInfo {
            module_px: scale_x.round().max(1.0) as u32,
            quiet_zone_modules: 4,
            threshold,
            layout: LayoutFamily::RibbonWeave,
        },
    })
}

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
    let left = (module_x * scale_x).floor().max(0.0) as u32;
    let top = (module_y * scale_y).floor().max(0.0) as u32;
    let right = ((module_x + 1.0) * scale_x).ceil().max(1.0) as u32;
    let bottom = ((module_y + 1.0) * scale_y).ceil().max(1.0) as u32;
    let right = right.saturating_sub(1);
    let bottom = bottom.saturating_sub(1);
    let sum = integral.sum_inclusive(left, top, right, bottom);
    let area = right
        .saturating_sub(left)
        .saturating_add(1)
        .saturating_mul(bottom.saturating_sub(top).saturating_add(1));
    (sum / area.max(1)) as u8
}

pub(crate) struct IntegralGray {
    width: u32,
    height: u32,
    stride: usize,
    sums: Vec<u32>,
}

impl IntegralGray {
    pub(crate) fn new(image: &GrayImage) -> Self {
        let width = image.width();
        let height = image.height();
        let stride = width as usize + 1;
        let mut sums = vec![0u32; (height as usize + 1) * stride];
        for y in 0..height {
            let mut row_sum = 0u32;
            for x in 0..width {
                row_sum = row_sum.saturating_add(u32::from(image.get_pixel(x, y).0[0]));
                let idx = (y as usize + 1) * stride + (x as usize + 1);
                sums[idx] = sums[y as usize * stride + (x as usize + 1)].saturating_add(row_sum);
            }
        }
        Self {
            width,
            height,
            stride,
            sums,
        }
    }

    pub(crate) fn sum_inclusive(&self, x0: u32, y0: u32, x1: u32, y1: u32) -> u32 {
        let x0 = x0.min(self.width) as usize;
        let y0 = y0.min(self.height) as usize;
        let x1 = x1.saturating_add(1).min(self.width) as usize;
        let y1 = y1.saturating_add(1).min(self.height) as usize;
        let a = self.sums[y0 * self.stride + x0];
        let b = self.sums[y0 * self.stride + x1];
        let c = self.sums[y1 * self.stride + x0];
        let d = self.sums[y1 * self.stride + x1];
        d + a - b - c
    }
}
