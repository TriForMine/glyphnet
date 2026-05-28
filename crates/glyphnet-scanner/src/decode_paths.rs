use glyphnet_core::{Cell, FrameHeader, HEADER_LEN, LayoutFamily, SymbolMatrix, bitstream, layout};
use glyphnet_decode::{
    AutoDecodedSymbol, DecodeError, DecodeOptions, RasterDecoder, decode_matrix,
};
use image::{DynamicImage, GrayImage};

use crate::ScanRegion;

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
    if matches!(candidate.stage, "signature-window" | "coarse-grid") {
        if let Ok(decoded) = decode_exact_ribbon_candidate(image, region) {
            return Ok(decoded);
        }
        if let Ok(decoded) = decode_fractional_ribbon_candidate(image) {
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
    }
    decoder.decode_auto_with_info(image)
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
            for threshold in [160, 192, 224] {
                let exact = RasterDecoder::new(DecodeOptions {
                    module_px,
                    quiet_zone_modules: 4,
                    threshold,
                    layout: LayoutFamily::RibbonWeave,
                });
                if let Ok(decoded) = exact.decode(image) {
                    return Ok(AutoDecodedSymbol {
                        decoded,
                        info: glyphnet_decode::AutoDecodeInfo {
                            module_px,
                            quiet_zone_modules: 4,
                            threshold,
                            layout: LayoutFamily::RibbonWeave,
                        },
                    });
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

fn module_shifts(radius: i32) -> impl Iterator<Item = f32> {
    (-radius * 2..=radius * 2).map(|value| value as f32 * 0.5)
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
        }
    }

    let decoded = decode_matrix(&matrix)?;
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
