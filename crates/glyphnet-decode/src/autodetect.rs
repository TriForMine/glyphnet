use glyphnet_core::{FrameHeader, HEADER_LEN, LayoutFamily, SymbolMatrix, bitstream, layout};
use image::GrayImage;

use crate::DecodeOptions;

const AUTO_QUIET_ZONE_MAX: u32 = 32;
const AUTO_MIN_SYMBOL_MODULES: u32 = 20;
const AUTO_MAX_SYMBOL_WIDTH_MODULES: u32 = 512;
const AUTO_MAX_SYMBOL_HEIGHT_MODULES: u32 = 256;
const AUTO_MAX_SYMBOL_AREA_MODULES: u32 = 65_536;
pub(crate) const MAX_SUSPECT_BYTES: usize = 16;

pub(crate) fn gcd_u32(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let tmp = a % b;
        a = b;
        b = tmp;
    }
    a
}

fn divisors_desc(value: u32) -> Vec<u32> {
    if value == 0 {
        return Vec::new();
    }
    let mut small = Vec::new();
    let mut large = Vec::new();
    let mut i = 1u32;
    while i * i <= value {
        if value % i == 0 {
            small.push(i);
            let other = value / i;
            if other != i {
                large.push(other);
            }
        }
        i += 1;
    }
    large.extend(small.into_iter().rev());
    large
}

pub(crate) fn module_candidates(gcd: u32, preferred: u32) -> Vec<u32> {
    let mut candidates = Vec::new();
    if preferred != 0 && gcd % preferred == 0 {
        candidates.push(preferred);
    }
    for candidate in divisors_desc(gcd) {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

pub(crate) fn quiet_zone_candidates(width_modules: u32, height_modules: u32) -> Vec<(u32, u32)> {
    let mut candidates = Vec::new();
    for quiet in 0..=AUTO_QUIET_ZONE_MAX {
        if width_modules > quiet * 2 && height_modules > quiet * 2 {
            let symbol_width = width_modules - quiet * 2;
            let symbol_height = height_modules - quiet * 2;
            if reference_sized_geometry(symbol_width, symbol_height) {
                candidates.push((quiet, quiet));
            }
        }
    }
    for quiet_x in 0..=AUTO_QUIET_ZONE_MAX {
        for quiet_y in 0..=AUTO_QUIET_ZONE_MAX {
            if quiet_x == quiet_y {
                continue;
            }
            if width_modules > quiet_x * 2 && height_modules > quiet_y * 2 {
                let symbol_width = width_modules - quiet_x * 2;
                let symbol_height = height_modules - quiet_y * 2;
                if reference_sized_geometry(symbol_width, symbol_height) {
                    candidates.push((quiet_x, quiet_y));
                }
            }
        }
    }
    candidates
}

fn reference_sized_geometry(width: u32, height: u32) -> bool {
    if width == height && width >= 29 {
        return true;
    }
    if width >= 96 && height >= 24 && width % 4 == 0 && height % 4 == 0 {
        let aspect = width as f32 / height.max(1) as f32;
        return (2.0..=6.5).contains(&aspect);
    }
    if width >= 48 && height >= 28 && width % 2 == 0 && height % 2 == 0 {
        let aspect = width as f32 / height.max(1) as f32;
        return (1.0..=3.2).contains(&aspect);
    }
    false
}

pub(crate) fn plausible_symbol_geometry(width: u32, height: u32) -> bool {
    if width < AUTO_MIN_SYMBOL_MODULES || height < AUTO_MIN_SYMBOL_MODULES {
        return false;
    }
    if width > AUTO_MAX_SYMBOL_WIDTH_MODULES || height > AUTO_MAX_SYMBOL_HEIGHT_MODULES {
        return false;
    }
    if width.saturating_mul(height) > AUTO_MAX_SYMBOL_AREA_MODULES {
        return false;
    }
    let aspect = width as f32 / height.max(1) as f32;
    (0.5..=8.0).contains(&aspect)
}

pub(crate) fn layout_candidates(primary: LayoutFamily) -> Vec<LayoutFamily> {
    let all = [
        LayoutFamily::RibbonWeave,
        LayoutFamily::SpectralMesh,
        LayoutFamily::PulseStream,
        LayoutFamily::Constellation,
        LayoutFamily::FrameGrid,
        LayoutFamily::Matrix,
        LayoutFamily::Hexagonal,
        LayoutFamily::Radial,
    ];
    let mut layouts = Vec::with_capacity(all.len());
    layouts.push(primary);
    for layout in all {
        if layout != primary {
            layouts.push(layout);
        }
    }
    layouts
}

pub(crate) fn threshold_candidates(configured: u8, luma: &GrayImage) -> Vec<u8> {
    let estimated = estimate_threshold_otsu(luma);
    let mut thresholds = vec![estimated];
    if estimated <= 1 || estimated >= 254 {
        thresholds.push(128);
    }
    if configured != estimated && !thresholds.contains(&configured) {
        thresholds.push(configured);
    }
    thresholds
}

fn estimate_threshold_otsu(luma: &GrayImage) -> u8 {
    let mut histogram = [0u32; 256];
    for pixel in luma.pixels() {
        histogram[pixel[0] as usize] += 1;
    }
    let total = u64::from(luma.width()).saturating_mul(u64::from(luma.height()));
    if total == 0 {
        return 0;
    }
    let mut sum_total = 0u64;
    for (i, count) in histogram.iter().enumerate() {
        sum_total += (i as u64) * u64::from(*count);
    }
    let mut sum_background = 0u64;
    let mut weight_background = 0u64;
    let mut max_variance = -1.0f64;
    let mut threshold = 0u8;
    for (i, count) in histogram.iter().enumerate() {
        weight_background += u64::from(*count);
        if weight_background == 0 {
            continue;
        }
        let weight_foreground = total - weight_background;
        if weight_foreground == 0 {
            break;
        }
        sum_background += (i as u64) * u64::from(*count);
        let mean_background = sum_background as f64 / weight_background as f64;
        let mean_foreground = (sum_total - sum_background) as f64 / weight_foreground as f64;
        let diff = mean_background - mean_foreground;
        let variance = (weight_background as f64) * (weight_foreground as f64) * diff * diff;
        if variance > max_variance {
            max_variance = variance;
            threshold = i as u8;
        }
    }
    threshold
}

pub(crate) fn average_module_luma(
    image: &GrayImage,
    module_x: u32,
    module_y: u32,
    module_px: u32,
    quiet_zone_x_modules: u32,
    quiet_zone_y_modules: u32,
) -> u8 {
    let start_x = (module_x + quiet_zone_x_modules) * module_px;
    let start_y = (module_y + quiet_zone_y_modules) * module_px;
    let mut sum = 0u32;
    for y in start_y..start_y + module_px {
        for x in start_x..start_x + module_px {
            sum += u32::from(image.get_pixel(x, y).0[0]);
        }
    }
    (sum / (module_px * module_px)) as u8
}

pub(crate) fn header_precheck(
    luma: &GrayImage,
    options: &DecodeOptions,
    quiet_zone_x_modules: u32,
    quiet_zone_y_modules: u32,
    symbol_width: u16,
    symbol_height: u16,
) -> bool {
    let mut bits = Vec::with_capacity(HEADER_LEN * 8);
    'rows: for y in 0..symbol_height {
        for x in 0..symbol_width {
            if !layout::is_data_module_for(options.layout, symbol_width, symbol_height, x, y) {
                continue;
            }
            let avg = average_module_luma(
                luma,
                u32::from(x),
                u32::from(y),
                options.module_px,
                quiet_zone_x_modules,
                quiet_zone_y_modules,
            );
            bits.push(avg < options.threshold);
            if bits.len() == HEADER_LEN * 8 {
                break 'rows;
            }
        }
    }
    if bits.len() < HEADER_LEN * 8 {
        return false;
    }
    let bytes = bitstream::bits_to_bytes(&bits);
    FrameHeader::decode(&bytes).is_ok()
}

pub(crate) fn suspect_bytes_from_bit_confidence(
    matrix: &SymbolMatrix,
    bit_confidence: &[u8],
    limit: usize,
) -> Vec<usize> {
    if bit_confidence.is_empty() || limit == 0 {
        return Vec::new();
    }
    let total_data_bits = matrix.read_data_bits().len();
    let total_bytes = total_data_bits.div_ceil(8);
    let mut scores = Vec::with_capacity(total_bytes);
    for byte_index in 0..total_bytes {
        let start = byte_index * 8;
        let end = (start + 8).min(bit_confidence.len());
        if start >= end {
            break;
        }
        let score = *bit_confidence[start..end].iter().min().unwrap_or(&u8::MAX);
        scores.push((byte_index, score));
    }
    scores.sort_by_key(|(_, score)| *score);
    scores.into_iter().take(limit).map(|(idx, _)| idx).collect()
}
