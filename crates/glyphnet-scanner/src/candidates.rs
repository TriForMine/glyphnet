use std::collections::VecDeque;

use glyphnet_core::LayoutFamily;
use glyphnet_cv::VisionProfile;
use image::{DynamicImage, GrayImage};

use crate::ScanRegion;
use crate::detectors::{CandidateDetector, ScanCandidate, push_unique_candidate};

const MIN_COMPONENT_PIXELS: u32 = 16;
const MAX_CANDIDATE_REGIONS: usize = 96;
const MAX_CONTENT_CANDIDATES: usize = 24;
const MAX_MATRIX_CANDIDATES: usize = 16;
const MAX_RIBBON_CANDIDATES: usize = 24;
const MAX_GENERIC_CANDIDATES: usize = 16;
const MAX_DARK_BOUNDS_CANDIDATES: usize = 8;
pub(crate) const MAX_QUAD_ATTEMPTS: usize = 4;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const PARALLEL_DECODE_CANDIDATE_THRESHOLD: usize = 8;

pub(crate) fn should_try_dark_bounds_fallback(
    image_width: u32,
    image_height: u32,
    candidate_count: usize,
) -> bool {
    image_width.saturating_mul(image_height) <= 900_000 || candidate_count == 0
}

pub(crate) fn still_scan_candidates(
    image: &DynamicImage,
    binary: &GrayImage,
    profile: VisionProfile,
    padding: u32,
    robust: bool,
) -> Vec<ScanCandidate> {
    let image_width = image.width();
    let image_height = image.height();
    let area = image_width.saturating_mul(image_height);
    let large_image = area > 900_000;
    let max_total = if robust {
        if large_image {
            28
        } else {
            MAX_CANDIDATE_REGIONS
        }
    } else if large_image {
        12
    } else {
        28
    };
    let max_content = if robust && large_image {
        8
    } else if robust {
        MAX_CONTENT_CANDIDATES
    } else if large_image {
        0
    } else {
        4
    };
    let max_matrix = if robust && large_image {
        8
    } else if robust {
        MAX_MATRIX_CANDIDATES
    } else if large_image {
        3
    } else {
        4
    };
    let max_ribbon = if robust && large_image {
        10
    } else if robust {
        MAX_RIBBON_CANDIDATES
    } else if large_image {
        8
    } else {
        12
    };
    let max_generic = if !robust || large_image {
        0
    } else {
        MAX_GENERIC_CANDIDATES
    };
    let max_dark_bounds = if !robust {
        1
    } else if large_image {
        4
    } else {
        MAX_DARK_BOUNDS_CANDIDATES
    };
    let mut candidates = Vec::new();

    if !large_image && let Some(bounds) = content_bounds(image) {
        let mut content =
            content_symbol_regions(bounds, image_width, image_height, profile.min_anchor_px);
        content.truncate(max_content);
        candidates.extend(content);
    }

    let mut matrix = matrix_candidates(binary, image_width, image_height);
    matrix.truncate(max_matrix);
    candidates.extend(matrix);

    let mut ribbon = ribbon_weave_candidates(binary, image_width, image_height);
    ribbon.truncate(max_ribbon);
    candidates.extend(ribbon);

    if max_generic > 0 {
        let mut generic = generic_binary_candidates(binary, padding, image_width, image_height);
        generic.truncate(max_generic);
        candidates.extend(generic);
    }

    if candidates.len() < max_total
        && should_try_dark_bounds_fallback(image_width, image_height, candidates.len())
        && let Some(bounds) = dark_bounds(binary)
    {
        let mut dark_bounds =
            ribbon_dark_bounds_candidates(bounds, padding, image_width, image_height);
        let dark_bounds_budget = max_dark_bounds.min(max_total - candidates.len());
        dark_bounds.truncate(dark_bounds_budget);
        candidates.extend(dark_bounds);
    }

    if candidates.len() < max_total {
        let mut fallback = coarse_ribbon_grid_candidates(image_width, image_height);
        fallback.truncate(max_total - candidates.len());
        candidates.extend(fallback);
    }

    candidates.truncate(max_total);
    candidates
}

fn coarse_ribbon_grid_candidates(image_width: u32, image_height: u32) -> Vec<ScanCandidate> {
    if image_width < 240 || image_height < 120 {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    for module_px in [4u32, 5, 3, 6, 7, 8] {
        let width = 104 * module_px;
        let height = 44 * module_px;
        if width > image_width || height > image_height {
            continue;
        }
        let x_fracs = [0.06_f32, 0.115, 0.16, 0.22, 0.30, 0.38];
        let y_fracs = [0.08_f32, 0.14, 0.20, 0.233, 0.28, 0.32];
        for xf in x_fracs {
            for yf in y_fracs {
                let x = ((image_width as f32 * xf).round() as u32)
                    .min(image_width.saturating_sub(width + 1));
                let y = ((image_height as f32 * yf).round() as u32)
                    .min(image_height.saturating_sub(height + 1));
                let region = ScanRegion {
                    x,
                    y,
                    width,
                    height,
                };
                if region_fits(region, image_width, image_height) {
                    push_unique_candidate(
                        &mut candidates,
                        CandidateDetector::RibbonWeave,
                        Some(LayoutFamily::RibbonWeave),
                        "coarse-grid",
                        region,
                    );
                }
            }
        }
        if module_px == 4 {
            let center_x = image_width.saturating_sub(width) / 2;
            let center_y = image_height.saturating_sub(height) / 2;
            for (x, y) in [
                (center_x, center_y),
                (center_x / 2, center_y),
                (center_x / 3, center_y / 2),
            ] {
                let region = ScanRegion {
                    x,
                    y,
                    width,
                    height,
                };
                if region_fits(region, image_width, image_height) {
                    push_unique_candidate(
                        &mut candidates,
                        CandidateDetector::RibbonWeave,
                        Some(LayoutFamily::RibbonWeave),
                        "coarse-grid",
                        region,
                    );
                }
            }
        }
    }
    candidates
}

fn content_bounds(image: &DynamicImage) -> Option<ScanRegion> {
    let rgba = image.to_rgba8();
    let mut min_x = image.width();
    let mut min_y = image.height();
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;

    for (x, y, pixel) in rgba.enumerate_pixels() {
        let [red, green, blue, alpha] = pixel.0;
        if alpha > 0 && (red < 245 || green < 245 || blue < 245) {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
            found = true;
        }
    }

    let region = found.then_some(ScanRegion {
        x: min_x,
        y: min_y,
        width: max_x.saturating_sub(min_x).saturating_add(1),
        height: max_y.saturating_sub(min_y).saturating_add(1),
    })?;
    let image_area = (image.width() as u64)
        .saturating_mul(image.height() as u64)
        .max(1);
    let region_area = (region.width as u64).saturating_mul(region.height as u64);
    if region_area.saturating_mul(100) / image_area >= 85 {
        return None;
    }
    Some(region)
}

fn content_symbol_regions(
    bounds: ScanRegion,
    image_width: u32,
    image_height: u32,
    min_module_px: u32,
) -> Vec<ScanCandidate> {
    let mut regions = Vec::new();
    for module_px in 1..=16 {
        if module_px < (min_module_px / 8).max(1) {
            continue;
        }
        if bounds.width % module_px != 0 || bounds.height % module_px != 0 {
            continue;
        }
        let symbol_width = bounds.width / module_px;
        let symbol_height = bounds.height / module_px;
        if !reference_generated_geometry(symbol_width, symbol_height) {
            continue;
        }
        let quiet_px = module_px.saturating_mul(4);
        let region = ScanRegion {
            x: bounds.x.saturating_sub(quiet_px),
            y: bounds.y.saturating_sub(quiet_px),
            width: bounds.width.saturating_add(quiet_px.saturating_mul(2)),
            height: bounds.height.saturating_add(quiet_px.saturating_mul(2)),
        };
        regions.push(ScanCandidate::new(
            CandidateDetector::GeneratedContent,
            None,
            "content-bounds",
            clamp_region(region, image_width, image_height),
        ));
    }
    for module_px in 1..=16 {
        if module_px < (min_module_px / 8).max(1) {
            continue;
        }
        let expected_content_width = 96 * module_px;
        if bounds.width.abs_diff(expected_content_width) > module_px {
            continue;
        }
        let quiet_px = module_px.saturating_mul(4);
        for extra_top_px in [0, 1, module_px / 2, module_px] {
            let region = ScanRegion {
                x: bounds.x.saturating_sub(quiet_px),
                y: bounds
                    .y
                    .saturating_sub(quiet_px.saturating_add(extra_top_px)),
                width: 104 * module_px,
                height: 44 * module_px,
            };
            regions.push(ScanCandidate::new(
                CandidateDetector::GeneratedContent,
                Some(LayoutFamily::RibbonWeave),
                "content-reference",
                clamp_region(region, image_width, image_height),
            ));
        }
    }
    for module_px in 1..=16 {
        if module_px < (min_module_px / 8).max(1) {
            continue;
        }
        if bounds.width % module_px != 0 || bounds.height % module_px != 0 {
            continue;
        }
        let symbol_width = bounds.width / module_px;
        let symbol_height = bounds.height / module_px;
        if !reference_ribbon_geometry(symbol_width, symbol_height) {
            continue;
        }
        let quiet_px = module_px.saturating_mul(4);
        let region = ScanRegion {
            x: bounds.x.saturating_sub(quiet_px),
            y: bounds.y.saturating_sub(quiet_px),
            width: bounds.width.saturating_add(quiet_px.saturating_mul(2)),
            height: bounds.height.saturating_add(quiet_px.saturating_mul(2)),
        };
        regions.push(ScanCandidate::new(
            CandidateDetector::GeneratedContent,
            Some(LayoutFamily::RibbonWeave),
            "content-bounds",
            clamp_region(region, image_width, image_height),
        ));
    }
    regions
}

fn reference_generated_geometry(width: u32, height: u32) -> bool {
    if width == height && width >= 29 {
        return true;
    }
    let aspect = width as f32 / height.max(1) as f32;
    width >= 48
        && height >= 28
        && width % 2 == 0
        && height % 2 == 0
        && (1.0..=8.0).contains(&aspect)
}

fn ribbon_dark_bounds_candidates(
    bounds: ScanRegion,
    padding: u32,
    image_width: u32,
    image_height: u32,
) -> Vec<ScanCandidate> {
    let mut candidates = Vec::new();
    if let Some(region) = ribbon_region_from_dark_bounds(bounds, image_width, image_height) {
        candidates.push(ScanCandidate::new(
            CandidateDetector::RibbonWeave,
            Some(LayoutFamily::RibbonWeave),
            "dark-ribbon",
            region,
        ));
    }
    let expanded = expand_region(bounds, padding, image_width, image_height);
    candidates.push(ScanCandidate::new(
        CandidateDetector::RibbonWeave,
        Some(LayoutFamily::RibbonWeave),
        "dark-bounds",
        expanded,
    ));
    if let Some(region) = ribbon_aspect_region(expanded, image_width, image_height) {
        candidates.push(ScanCandidate::new(
            CandidateDetector::RibbonWeave,
            Some(LayoutFamily::RibbonWeave),
            "signature-window",
            region,
        ));
    }
    candidates
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MatrixFinder {
    x: u32,
    y: u32,
    module_px: u32,
}

fn matrix_candidates(
    binary: &GrayImage,
    image_width: u32,
    image_height: u32,
) -> Vec<ScanCandidate> {
    if image_width < 80 || image_height < 80 {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    let finders = matrix_finders(binary);
    for top_left in &finders {
        for top_right in &finders {
            if top_right.y.abs_diff(top_left.y) > top_left.module_px * 2
                || top_right.x <= top_left.x + top_left.module_px * 16
            {
                continue;
            }
            for bottom_left in &finders {
                if bottom_left.x.abs_diff(top_left.x) > top_left.module_px * 2
                    || bottom_left.y <= top_left.y + top_left.module_px * 16
                {
                    continue;
                }
                let module_px = median3(
                    top_left.module_px,
                    top_right.module_px,
                    bottom_left.module_px,
                );
                if module_px == 0 {
                    continue;
                }
                let content_width = top_right.x.saturating_sub(top_left.x) / module_px + 7;
                let content_height = bottom_left.y.saturating_sub(top_left.y) / module_px + 7;
                if !reference_generated_geometry(content_width, content_height) {
                    continue;
                }
                let quiet_px = module_px.saturating_mul(4);
                let region = ScanRegion {
                    x: top_left.x.saturating_sub(quiet_px),
                    y: top_left.y.saturating_sub(quiet_px),
                    width: content_width.saturating_add(8).saturating_mul(module_px),
                    height: content_height.saturating_add(8).saturating_mul(module_px),
                };
                if region_fits(region, image_width, image_height) {
                    push_unique_candidate(
                        &mut candidates,
                        CandidateDetector::Matrix,
                        Some(LayoutFamily::Matrix),
                        "matrix-finders",
                        region,
                    );
                }
            }
        }
    }
    candidates
}

fn matrix_finders(binary: &GrayImage) -> Vec<MatrixFinder> {
    let mut finders = Vec::new();
    let area = binary.width().saturating_mul(binary.height());
    if area > 300_000 {
        scan_matrix_finders_runs(binary, &mut finders, 4);
        if finders.len() < 6 {
            scan_matrix_finders_runs(binary, &mut finders, 2);
        }
    } else {
        scan_matrix_finders_runs(binary, &mut finders, 2);
    }

    if finders.len() >= 12 {
        finders.truncate(24);
        return finders;
    }

    matrix_grid_template_finders(binary, &mut finders);

    if finders.len() >= 12 {
        finders.truncate(24);
        return finders;
    }

    for component in dark_components(binary).into_iter().take(128) {
        let width = component.bounds.width;
        let height = component.bounds.height;
        if width < 12 || height < 12 || width > 192 || height > 192 {
            continue;
        }
        let aspect = width as f32 / height.max(1) as f32;
        if !(0.75..=1.33).contains(&aspect) {
            continue;
        }
        let module_px = ((width.max(height) as f32 / 7.0).round() as u32).clamp(2, 24);
        let nudge = module_px.max(2) as i32;
        for dy in -nudge..=nudge {
            for dx in -nudge..=nudge {
                let x = (component.bounds.x as i32 + dx).max(0) as u32;
                let y = (component.bounds.y as i32 + dy).max(0) as u32;
                if x.saturating_add(7 * module_px) > binary.width()
                    || y.saturating_add(7 * module_px) > binary.height()
                {
                    continue;
                }
                if matrix_finder_matches(binary, x, y, module_px) {
                    push_matrix_finder(&mut finders, MatrixFinder { x, y, module_px });
                }
            }
        }
    }
    finders.truncate(96);
    finders
}

fn scan_matrix_finders_runs(binary: &GrayImage, finders: &mut Vec<MatrixFinder>, row_step: usize) {
    for y in (0..binary.height()).step_by(row_step.max(1)) {
        let mut runs = [0u32; 5];
        let mut run_color_dark = pixel_is_dark(binary, 0, y);
        let mut run_count = 0usize;
        for x in 0..binary.width() {
            let dark = pixel_is_dark(binary, x, y);
            if dark == run_color_dark {
                run_count += 1;
                continue;
            }
            shift_finder_runs(&mut runs, run_count as u32);
            if run_color_dark && matrix_finder_run_ratio(runs) {
                let total_width = runs.iter().sum::<u32>();
                let module_px = ((total_width as f32 / 7.0).round() as u32).clamp(2, 24);
                let center_x = x.saturating_sub(runs[4] + runs[3] + runs[2] / 2);
                let center_y = y;
                let finder_x = center_x.saturating_sub((7 * module_px) / 2);
                let finder_y = center_y.saturating_sub((7 * module_px) / 2);
                verify_and_push_matrix_finder(binary, finders, finder_x, finder_y, module_px);
            }
            run_color_dark = dark;
            run_count = 1;
        }
    }
}

fn matrix_grid_template_finders(binary: &GrayImage, finders: &mut Vec<MatrixFinder>) {
    for module_px in 2..=12 {
        let extent = 7 * module_px;
        if extent > binary.width() || extent > binary.height() {
            continue;
        }
        for y in 0..=binary.height() - extent {
            for x in 0..=binary.width() - extent {
                if !matrix_finder_fast_prefilter(binary, x, y, module_px) {
                    continue;
                }
                if matrix_finder_center_template_matches(binary, x, y, module_px) {
                    verify_and_push_matrix_finder(binary, finders, x, y, module_px);
                    if finders.len() >= 96 {
                        return;
                    }
                }
            }
        }
    }
}

fn matrix_finder_fast_prefilter(binary: &GrayImage, x: u32, y: u32, module_px: u32) -> bool {
    matrix_module_center_dark(binary, x, y, module_px, 0, 0)
        && matrix_module_center_dark(binary, x, y, module_px, 6, 0)
        && matrix_module_center_dark(binary, x, y, module_px, 0, 6)
        && matrix_module_center_dark(binary, x, y, module_px, 6, 6)
        && matrix_module_center_dark(binary, x, y, module_px, 3, 3)
        && !matrix_module_center_dark(binary, x, y, module_px, 1, 1)
        && !matrix_module_center_dark(binary, x, y, module_px, 5, 1)
        && !matrix_module_center_dark(binary, x, y, module_px, 1, 5)
        && !matrix_module_center_dark(binary, x, y, module_px, 5, 5)
}

fn matrix_finder_center_template_matches(
    binary: &GrayImage,
    x: u32,
    y: u32,
    module_px: u32,
) -> bool {
    let mut mismatches = 0u32;
    for module_y in 0..7 {
        for module_x in 0..7 {
            let expected_dark = matrix_finder_module_dark(module_x, module_y);
            let dark = matrix_module_center_dark(binary, x, y, module_px, module_x, module_y);
            if dark != expected_dark {
                mismatches += 1;
            }
        }
    }
    mismatches <= 6
}

fn matrix_module_center_dark(
    binary: &GrayImage,
    x: u32,
    y: u32,
    module_px: u32,
    module_x: u32,
    module_y: u32,
) -> bool {
    let center_x = x + module_x * module_px + module_px / 2;
    let center_y = y + module_y * module_px + module_px / 2;
    pixel_is_dark(
        binary,
        center_x.min(binary.width() - 1),
        center_y.min(binary.height() - 1),
    )
}

fn verify_and_push_matrix_finder(
    binary: &GrayImage,
    finders: &mut Vec<MatrixFinder>,
    x: u32,
    y: u32,
    module_px: u32,
) {
    let nudge = (module_px / 2).max(1) as i32;
    for dy in -nudge..=nudge {
        for dx in -nudge..=nudge {
            let x = (x as i32 + dx).max(0) as u32;
            let y = (y as i32 + dy).max(0) as u32;
            if x.saturating_add(7 * module_px) > binary.width()
                || y.saturating_add(7 * module_px) > binary.height()
            {
                continue;
            }
            if matrix_finder_matches(binary, x, y, module_px) {
                push_matrix_finder(finders, MatrixFinder { x, y, module_px });
                return;
            }
        }
    }
}

fn shift_finder_runs(runs: &mut [u32; 5], next: u32) {
    runs[0] = runs[1];
    runs[1] = runs[2];
    runs[2] = runs[3];
    runs[3] = runs[4];
    runs[4] = next;
}

fn matrix_finder_run_ratio(runs: [u32; 5]) -> bool {
    if runs.contains(&0) {
        return false;
    }
    let total = runs.iter().sum::<u32>();
    if total < 14 {
        return false;
    }
    let module = total as f32 / 7.0;
    let tolerance = module.max(1.0) * 0.8;
    (runs[0] as f32 - module).abs() <= tolerance
        && (runs[1] as f32 - module).abs() <= tolerance
        && (runs[2] as f32 - module * 3.0).abs() <= tolerance * 1.5
        && (runs[3] as f32 - module).abs() <= tolerance
        && (runs[4] as f32 - module).abs() <= tolerance
}

fn pixel_is_dark(binary: &GrayImage, x: u32, y: u32) -> bool {
    binary.get_pixel(x, y).0[0] == 0
}

fn push_matrix_finder(finders: &mut Vec<MatrixFinder>, finder: MatrixFinder) {
    let duplicate_radius = finder.module_px.max(2);
    if finders.iter().any(|existing| {
        existing.x.abs_diff(finder.x) <= duplicate_radius
            && existing.y.abs_diff(finder.y) <= duplicate_radius
    }) {
        return;
    }
    finders.push(finder);
}

fn matrix_finder_matches(binary: &GrayImage, x: u32, y: u32, module_px: u32) -> bool {
    let mut total = 0u32;
    let mut mismatches = 0u32;
    for module_y in 0..7 {
        for module_x in 0..7 {
            let expected_dark = matrix_finder_module_dark(module_x, module_y);
            let dark = module_dark_fraction(
                binary,
                x + module_x * module_px,
                y + module_y * module_px,
                module_px,
            ) >= 0.62;
            total += 1;
            if dark != expected_dark {
                mismatches += 1;
            }
        }
    }
    mismatches <= total / 8
}

const fn matrix_finder_module_dark(module_x: u32, module_y: u32) -> bool {
    module_x == 0
        || module_y == 0
        || module_x == 6
        || module_y == 6
        || (module_x >= 2 && module_x <= 4 && module_y >= 2 && module_y <= 4)
}

fn module_dark_fraction(binary: &GrayImage, x: u32, y: u32, module_px: u32) -> f32 {
    let mut dark = 0u32;
    let mut total = 0u32;
    let inset = (module_px / 4).max(1).min(module_px.saturating_sub(1));
    let x_start = x.saturating_add(inset).min(binary.width());
    let y_start = y.saturating_add(inset).min(binary.height());
    let x_end = x
        .saturating_add(module_px.saturating_sub(inset))
        .min(binary.width());
    let y_end = y
        .saturating_add(module_px.saturating_sub(inset))
        .min(binary.height());
    for py in y_start..y_end {
        for px in x_start..x_end {
            total += 1;
            if binary.get_pixel(px, py).0[0] == 0 {
                dark += 1;
            }
        }
    }
    dark as f32 / total.max(1) as f32
}

fn median3(a: u32, b: u32, c: u32) -> u32 {
    let mut values = [a, b, c];
    values.sort_unstable();
    values[1]
}

fn reference_ribbon_geometry(width: u32, height: u32) -> bool {
    width >= 96
        && height >= 28
        && width % 4 == 0
        && height % 4 == 0
        && (2.0..=8.0).contains(&(width as f32 / height.max(1) as f32))
}

fn clamp_region(region: ScanRegion, image_width: u32, image_height: u32) -> ScanRegion {
    let x = region.x.min(image_width.saturating_sub(1));
    let y = region.y.min(image_height.saturating_sub(1));
    let max_x = region.x.saturating_add(region.width).min(image_width);
    let max_y = region.y.saturating_add(region.height).min(image_height);
    ScanRegion {
        x,
        y,
        width: max_x.saturating_sub(x).max(1),
        height: max_y.saturating_sub(y).max(1),
    }
}

fn ribbon_aspect_region(
    region: ScanRegion,
    image_width: u32,
    image_height: u32,
) -> Option<ScanRegion> {
    let target_height = ((region.width as f32 * 44.0 / 104.0).round() as u32).max(region.height);
    let target_width = ((region.height as f32 * 104.0 / 44.0).round() as u32).max(region.width);
    let width_first = centered_region(
        region,
        region.width,
        target_height,
        image_width,
        image_height,
    );
    let height_first = centered_region(
        region,
        target_width,
        region.height,
        image_width,
        image_height,
    );

    [width_first, height_first]
        .into_iter()
        .flatten()
        .filter(|candidate| plausible_region(*candidate))
        .min_by_key(|candidate| {
            candidate
                .width
                .saturating_mul(candidate.height)
                .saturating_sub(region.width.saturating_mul(region.height))
        })
}

fn ribbon_region_from_dark_bounds(
    bounds: ScanRegion,
    image_width: u32,
    image_height: u32,
) -> Option<ScanRegion> {
    if bounds.width < 48 || bounds.height < 24 {
        return None;
    }
    let total_width = ((bounds.width as f32 * 104.0 / 96.0).round() as u32).max(bounds.width);
    let total_height = ((total_width as f32 * 44.0 / 104.0).round() as u32).max(bounds.height);
    centered_region(bounds, total_width, total_height, image_width, image_height)
        .filter(|candidate| plausible_region(*candidate))
}

fn centered_region(
    region: ScanRegion,
    width: u32,
    height: u32,
    image_width: u32,
    image_height: u32,
) -> Option<ScanRegion> {
    if width > image_width || height > image_height {
        return None;
    }
    let center_x = region.x.saturating_add(region.width / 2);
    let center_y = region.y.saturating_add(region.height / 2);
    let x = center_x
        .saturating_sub(width / 2)
        .min(image_width.saturating_sub(width));
    let y = center_y
        .saturating_sub(height / 2)
        .min(image_height.saturating_sub(height));
    Some(ScanRegion {
        x,
        y,
        width,
        height,
    })
}

#[derive(Debug, Clone, Copy)]
struct DarkComponent {
    pixels: u32,
    bounds: ScanRegion,
}

pub(crate) fn dark_bounds(binary: &GrayImage) -> Option<ScanRegion> {
    let components = dark_components(binary);
    let mut best: Option<(u32, ScanRegion)> = None;
    let mut significant: Option<ScanRegion> = None;

    for component in components {
        if component.pixels >= MIN_COMPONENT_PIXELS {
            significant = Some(match significant {
                Some(existing) => union_region(existing, component.bounds),
                None => component.bounds,
            });
        }
        if best.is_none_or(|(best_count, _)| component.pixels > best_count) {
            best = Some((component.pixels, component.bounds));
        }
    }
    significant.or_else(|| best.map(|(_, region)| region))
}

fn dark_components(binary: &GrayImage) -> Vec<DarkComponent> {
    let width = binary.width();
    let height = binary.height();
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let mut visited = vec![false; width as usize * height as usize];
    let mut components = Vec::new();

    for y in 0..binary.height() {
        for x in 0..binary.width() {
            let index = pixel_index(width, x, y);
            if visited[index] || binary.get_pixel(x, y).0[0] != 0 {
                continue;
            }

            let mut queue = VecDeque::from([(x, y)]);
            visited[index] = true;
            let mut count = 0u32;
            let mut min_x = x;
            let mut min_y = y;
            let mut max_x = x;
            let mut max_y = y;

            while let Some((cx, cy)) = queue.pop_front() {
                count += 1;
                min_x = min_x.min(cx);
                min_y = min_y.min(cy);
                max_x = max_x.max(cx);
                max_y = max_y.max(cy);

                let y_start = cy.saturating_sub(1);
                let y_end = (cy + 1).min(height - 1);
                let x_start = cx.saturating_sub(1);
                let x_end = (cx + 1).min(width - 1);
                for ny in y_start..=y_end {
                    for nx in x_start..=x_end {
                        let neighbor_index = pixel_index(width, nx, ny);
                        if visited[neighbor_index] || binary.get_pixel(nx, ny).0[0] != 0 {
                            continue;
                        }
                        visited[neighbor_index] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }

            let region = ScanRegion {
                x: min_x,
                y: min_y,
                width: max_x.saturating_sub(min_x).saturating_add(1),
                height: max_y.saturating_sub(min_y).saturating_add(1),
            };
            components.push(DarkComponent {
                pixels: count,
                bounds: region,
            });
        }
    }
    components.sort_by_key(|component| std::cmp::Reverse(component.pixels));
    components
}

fn pixel_index(width: u32, x: u32, y: u32) -> usize {
    y as usize * width as usize + x as usize
}

fn ribbon_weave_candidates(
    binary: &GrayImage,
    image_width: u32,
    image_height: u32,
) -> Vec<ScanCandidate> {
    let mut regions = Vec::new();
    regions.extend(ribbon_totem_regions(binary, image_width, image_height));
    regions.extend(ribbon_rail_regions(binary, image_width, image_height));
    regions.truncate(MAX_CANDIDATE_REGIONS);
    regions
}

fn generic_binary_candidates(
    binary: &GrayImage,
    padding: u32,
    image_width: u32,
    image_height: u32,
) -> Vec<ScanCandidate> {
    let mut regions = Vec::new();
    regions.extend(horizontal_band_regions(
        binary,
        padding,
        image_width,
        image_height,
    ));

    for component in dark_components(binary).into_iter().take(64) {
        if component.pixels < MIN_COMPONENT_PIXELS {
            continue;
        }
        let expanded = expand_region(component.bounds, padding * 2, image_width, image_height);
        if plausible_region(expanded) {
            push_unique_candidate(
                &mut regions,
                CandidateDetector::GenericBinary,
                None,
                "component",
                expanded,
            );
        }
    }

    regions.truncate(MAX_CANDIDATE_REGIONS);
    regions
}

#[derive(Debug, Clone, Copy)]
struct RailRow {
    y: u32,
    min_x: u32,
    max_x: u32,
    transitions: u32,
    dark: u32,
}

#[derive(Debug, Clone, Copy)]
struct RailGroup {
    y: u32,
    min_x: u32,
    max_x: u32,
    transitions: u32,
    dark: u32,
    rows: u32,
}

#[derive(Debug, Clone, Copy)]
struct TotemGroup {
    x: u32,
    min_y: u32,
    max_y: u32,
    dark: u32,
    columns: u32,
}

fn ribbon_totem_regions(
    binary: &GrayImage,
    image_width: u32,
    image_height: u32,
) -> Vec<ScanCandidate> {
    if image_width < 160 || image_height < 80 {
        return Vec::new();
    }

    let groups = totem_groups(binary, image_width, image_height);
    let mut regions = Vec::new();

    for left in &groups {
        for right in &groups {
            if right.x <= left.x + 120 {
                continue;
            }
            let overlap_y = left
                .max_y
                .min(right.max_y)
                .saturating_sub(left.min_y.max(right.min_y));
            if overlap_y < 120 {
                continue;
            }
            let dx = right.x.saturating_sub(left.x) as f32;
            for module_span in [83.0_f32, 85.0] {
                let scale = dx / module_span;
                if !(2.0..=32.0).contains(&scale) {
                    continue;
                }
                let left_x = left.x as f32;
                let top_y = left.min_y.min(right.min_y) as f32;
                for left_module_x in [9.0_f32, 11.0] {
                    for totem_top_module_y in [7.0_f32, 9.0] {
                        push_fractional_region(
                            &mut regions,
                            left_x - left_module_x * scale,
                            top_y - totem_top_module_y * scale,
                            104.0 * scale,
                            44.0 * scale,
                            image_width,
                            image_height,
                        );
                    }
                }
            }
        }
    }

    regions.truncate(12);
    regions
}

fn totem_groups(binary: &GrayImage, image_width: u32, image_height: u32) -> Vec<TotemGroup> {
    let mut columns = Vec::new();
    for x in 0..image_width {
        let mut runs = Vec::new();
        let mut run_start = None;
        for y in 0..image_height {
            let is_dark = binary.get_pixel(x, y).0[0] == 0;
            match (run_start, is_dark) {
                (None, true) => run_start = Some(y),
                (Some(start), false) => {
                    if y.saturating_sub(start) >= 2 {
                        runs.push((start, y - 1));
                    }
                    run_start = None;
                }
                _ => {}
            }
        }
        if let Some(start) = run_start
            && image_height.saturating_sub(start) >= 2
        {
            runs.push((start, image_height - 1));
        }

        let Some((min_y, max_y, dark, transitions)) = best_totem_column_cluster(&runs) else {
            continue;
        };
        let span = max_y.saturating_sub(min_y).saturating_add(1);
        let density = dark as f32 / span as f32;
        if span >= 160 && transitions >= 12 && (0.08..=0.55).contains(&density) {
            columns.push(TotemGroup {
                x,
                min_y,
                max_y,
                dark,
                columns: 1,
            });
        }
    }

    let mut groups = Vec::new();
    let mut current: Option<TotemGroup> = None;
    for column in columns {
        match current {
            Some(mut group) if column.x <= group.x + 4 => {
                let total = group.columns + 1;
                group.x = (group.x * group.columns + column.x) / total;
                group.min_y = group.min_y.min(column.min_y);
                group.max_y = group.max_y.max(column.max_y);
                group.dark += column.dark;
                group.columns = total;
                current = Some(group);
            }
            Some(group) => {
                groups.push(group);
                current = Some(column);
            }
            None => current = Some(column),
        }
    }
    if let Some(group) = current {
        groups.push(group);
    }

    groups.sort_by(|a, b| {
        let a_span = a.max_y.saturating_sub(a.min_y);
        let b_span = b.max_y.saturating_sub(b.min_y);
        b_span.cmp(&a_span).then_with(|| b.dark.cmp(&a.dark))
    });
    groups.truncate(48);
    groups
}

fn best_totem_column_cluster(runs: &[(u32, u32)]) -> Option<(u32, u32, u32, u32)> {
    let mut best: Option<(u32, u32, u32, u32)> = None;
    let mut index = 0usize;
    while index < runs.len() {
        let min_y = runs[index].0;
        let mut max_y = runs[index].1;
        let mut dark = runs[index]
            .1
            .saturating_sub(runs[index].0)
            .saturating_add(1);
        let mut transitions = 2u32;
        index += 1;
        while index < runs.len() && runs[index].0 <= max_y + 64 {
            max_y = runs[index].1;
            dark += runs[index]
                .1
                .saturating_sub(runs[index].0)
                .saturating_add(1);
            transitions += 2;
            index += 1;
        }

        let span = max_y.saturating_sub(min_y).saturating_add(1);
        let score = span.saturating_mul(transitions).saturating_mul(dark);
        let best_score = best
            .map(|(best_min, best_max, best_dark, best_transitions)| {
                best_max
                    .saturating_sub(best_min)
                    .saturating_add(1)
                    .saturating_mul(best_transitions)
                    .saturating_mul(best_dark)
            })
            .unwrap_or(0);
        if score > best_score {
            best = Some((min_y, max_y, dark, transitions));
        }
    }
    best
}

fn ribbon_rail_regions(
    binary: &GrayImage,
    image_width: u32,
    image_height: u32,
) -> Vec<ScanCandidate> {
    if image_width < 160 || image_height < 80 {
        return Vec::new();
    }

    let groups = rail_groups(binary, image_width, image_height);
    let mut regions = Vec::new();

    for group in &groups {
        let span = group.max_x.saturating_sub(group.min_x).saturating_add(1);
        if span < 120 {
            continue;
        }

        let estimated_module = (span as f32 / 68.0).round() as i32;
        for module_px in (estimated_module - 1)..=(estimated_module + 1) {
            if !(2..=24).contains(&module_px) {
                continue;
            }
            let module_px = module_px as u32;
            for rail_module_y in [7_u32, 9] {
                let base_x = group.min_x.saturating_sub(20 * module_px).saturating_add(2);
                let base_y = group
                    .y
                    .saturating_sub(rail_module_y * module_px + module_px / 2 + 2);
                for x_nudge in [0, 2] {
                    for y_nudge in [0, module_px / 2 + 1] {
                        let region = ScanRegion {
                            x: base_x.saturating_add(x_nudge),
                            y: base_y.saturating_add(y_nudge),
                            width: 104 * module_px,
                            height: 44 * module_px,
                        };
                        if region_fits(region, image_width, image_height) {
                            push_unique_candidate(
                                &mut regions,
                                CandidateDetector::RibbonWeave,
                                Some(LayoutFamily::RibbonWeave),
                                "signature-window",
                                region,
                            );
                        }
                    }
                }
            }
        }

        push_scaled_signature_regions(
            &mut regions,
            group.min_x,
            group.y,
            span,
            image_width,
            image_height,
        );
    }

    for upper in &groups {
        for lower in &groups {
            if lower.y <= upper.y + 48 {
                continue;
            }

            let upper_span = upper.max_x.saturating_sub(upper.min_x).saturating_add(1);
            let lower_span = lower.max_x.saturating_sub(lower.min_x).saturating_add(1);
            let overlap = upper
                .max_x
                .min(lower.max_x)
                .saturating_sub(upper.min_x.max(lower.min_x));
            if overlap < upper_span.min(lower_span) / 2 {
                continue;
            }

            let span = upper_span.max(lower_span);
            let estimated_module = (span as f32 / 68.0).round() as i32;
            for module_px in (estimated_module - 1)..=(estimated_module + 1) {
                if !(2..=24).contains(&module_px) {
                    continue;
                }
                let module_px = module_px as u32;
                let expected_symbol_height = 44 * module_px;
                let vertical_distance = lower.y.saturating_sub(upper.y);
                if vertical_distance + 10 * module_px < 20 * module_px
                    || vertical_distance > expected_symbol_height
                {
                    continue;
                }
                for rail_module_y in [7_u32, 9] {
                    let min_x = upper.min_x.min(lower.min_x);
                    let base_x = min_x.saturating_sub(20 * module_px).saturating_add(2);
                    let base_y = upper
                        .y
                        .saturating_sub(rail_module_y * module_px + module_px / 2 + 2);
                    for x_nudge in [0, 2] {
                        for y_nudge in [0, module_px / 2 + 1] {
                            let region = ScanRegion {
                                x: base_x.saturating_add(x_nudge),
                                y: base_y.saturating_add(y_nudge),
                                width: 104 * module_px,
                                height: expected_symbol_height,
                            };
                            if region_fits(region, image_width, image_height) {
                                push_unique_candidate(
                                    &mut regions,
                                    CandidateDetector::RibbonWeave,
                                    Some(LayoutFamily::RibbonWeave),
                                    "signature-window",
                                    region,
                                );
                            }
                        }
                    }
                }
            }

            push_scaled_signature_regions(
                &mut regions,
                upper.min_x.min(lower.min_x),
                upper.y,
                span,
                image_width,
                image_height,
            );
        }
    }

    regions.truncate(32);
    regions
}

fn push_scaled_signature_regions(
    regions: &mut Vec<ScanCandidate>,
    rail_min_x: u32,
    rail_y: u32,
    rail_span: u32,
    image_width: u32,
    image_height: u32,
) {
    let scale = rail_span as f32 / 68.0;
    if scale < 1.5 {
        return;
    }
    let width = (104.0 * scale).round().max(104.0) as u32;
    let height = (44.0 * scale).round().max(44.0) as u32;
    let x_base = rail_min_x as i64 - (18.0 * scale).round() as i64;
    for rail_module_y in [7.0_f32, 9.0] {
        let y_base = rail_y as i64 - (rail_module_y * scale).round() as i64;
        for x_nudge in [-2_i64, 0, 2] {
            for y_nudge in [-2_i64, 0, 2] {
                let x = (x_base + x_nudge).max(0) as u32;
                let y = (y_base + y_nudge).max(0) as u32;
                let region = ScanRegion {
                    x,
                    y,
                    width,
                    height,
                };
                if region_fits(region, image_width, image_height) {
                    push_unique_candidate(
                        regions,
                        CandidateDetector::RibbonWeave,
                        Some(LayoutFamily::RibbonWeave),
                        "signature-window",
                        region,
                    );
                }
            }
        }
    }
}

fn push_fractional_region(
    regions: &mut Vec<ScanCandidate>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    image_width: u32,
    image_height: u32,
) {
    let region = ScanRegion {
        x: x.round().max(0.0) as u32,
        y: y.round().max(0.0) as u32,
        width: width.round().max(104.0) as u32,
        height: height.round().max(44.0) as u32,
    };
    if region_fits(region, image_width, image_height) {
        push_unique_candidate(
            regions,
            CandidateDetector::RibbonWeave,
            Some(LayoutFamily::RibbonWeave),
            "signature-window",
            region,
        );
    }
}

fn rail_groups(binary: &GrayImage, image_width: u32, image_height: u32) -> Vec<RailGroup> {
    let mut rows = Vec::new();
    for y in 0..image_height {
        if let Some(row) = rail_row(binary, image_width, y) {
            rows.push(row);
        }
    }

    let mut groups = Vec::new();
    let mut current: Option<RailGroup> = None;
    for row in rows {
        match current {
            Some(mut group) if row.y <= group.y + 4 => {
                let total_rows = group.rows + 1;
                group.y = (group.y * group.rows + row.y) / total_rows;
                group.min_x = group.min_x.min(row.min_x);
                group.max_x = group.max_x.max(row.max_x);
                group.transitions += row.transitions;
                group.dark += row.dark;
                group.rows = total_rows;
                current = Some(group);
            }
            Some(group) => {
                if group.rows >= 2 {
                    groups.push(group);
                }
                current = Some(RailGroup {
                    y: row.y,
                    min_x: row.min_x,
                    max_x: row.max_x,
                    transitions: row.transitions,
                    dark: row.dark,
                    rows: 1,
                });
            }
            None => {
                current = Some(RailGroup {
                    y: row.y,
                    min_x: row.min_x,
                    max_x: row.max_x,
                    transitions: row.transitions,
                    dark: row.dark,
                    rows: 1,
                });
            }
        }
    }
    if let Some(group) = current
        && group.rows >= 2
    {
        groups.push(group);
    }

    groups.sort_by(|a, b| rail_group_score(*a).total_cmp(&rail_group_score(*b)));
    groups.truncate(32);
    groups
}

fn rail_group_score(group: RailGroup) -> f32 {
    let span = group.max_x.saturating_sub(group.min_x).saturating_add(1);
    let module_px = (span as f32 / 68.0).round().max(1.0);
    let residual = (span as f32 - 68.0 * module_px).abs() / module_px;
    let density = group.dark as f32 / (span.saturating_mul(group.rows).max(1)) as f32;
    let density_error = (density - 0.25).abs() * 24.0;
    density_error + residual
}

fn rail_row(binary: &GrayImage, image_width: u32, y: u32) -> Option<RailRow> {
    let mut dark = 0u32;
    let mut transition_positions = Vec::new();
    let mut last_dark = false;
    let mut seen = false;

    for x in 0..image_width {
        let is_dark = binary.get_pixel(x, y).0[0] == 0;
        if is_dark {
            dark += 1;
        }
        if seen && is_dark != last_dark {
            transition_positions.push(x);
        }
        seen = true;
        last_dark = is_dark;
    }

    if dark == 0 || transition_positions.len() < 14 {
        return None;
    }

    let mut best: Option<(usize, usize)> = None;
    let mut start = 0usize;
    for index in 1..transition_positions.len() {
        if transition_positions[index].saturating_sub(transition_positions[index - 1]) > 64 {
            best = choose_transition_cluster(best, start, index - 1);
            start = index;
        }
    }
    best = choose_transition_cluster(best, start, transition_positions.len() - 1);

    let (start, end) = best?;
    let transitions = (end - start + 1) as u32;
    if transitions < 14 {
        return None;
    }

    let min_x = transition_positions[start].saturating_sub(12);
    let max_x = transition_positions[end]
        .saturating_add(12)
        .min(image_width.saturating_sub(1));
    let span = max_x.saturating_sub(min_x).saturating_add(1);
    if span < 120 {
        return None;
    }

    let mut cluster_dark = 0u32;
    for x in min_x..=max_x {
        if binary.get_pixel(x, y).0[0] == 0 {
            cluster_dark += 1;
        }
    }
    let density = cluster_dark as f32 / span as f32;
    if !(0.18..=0.78).contains(&density) {
        return None;
    }

    Some(RailRow {
        y,
        min_x,
        max_x,
        transitions,
        dark: cluster_dark,
    })
}

fn choose_transition_cluster(
    best: Option<(usize, usize)>,
    start: usize,
    end: usize,
) -> Option<(usize, usize)> {
    if end < start {
        return best;
    }
    match best {
        Some((best_start, best_end)) if best_end - best_start >= end - start => {
            Some((best_start, best_end))
        }
        _ => Some((start, end)),
    }
}

fn region_fits(region: ScanRegion, image_width: u32, image_height: u32) -> bool {
    region.width >= 104
        && region.height >= 44
        && region.x.saturating_add(region.width) <= image_width
        && region.y.saturating_add(region.height) <= image_height
}

fn horizontal_band_regions(
    binary: &GrayImage,
    padding: u32,
    image_width: u32,
    image_height: u32,
) -> Vec<ScanCandidate> {
    if image_width == 0 || image_height == 0 {
        return Vec::new();
    }
    let row_threshold = (image_width / 120).clamp(8, 80);
    let mut rows = Vec::new();
    for y in 0..image_height {
        let mut dark = 0u32;
        let mut min_x = image_width;
        let mut max_x = 0u32;
        for x in 0..image_width {
            if binary.get_pixel(x, y).0[0] == 0 {
                dark += 1;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
            }
        }
        if dark >= row_threshold {
            rows.push((y, min_x, max_x, dark));
        }
    }

    let mut regions = Vec::new();
    let mut index = 0usize;
    while index < rows.len() {
        let start_y = rows[index].0;
        let mut end_y = start_y;
        let mut min_x = rows[index].1;
        let mut max_x = rows[index].2;
        let mut dark_sum = rows[index].3;
        index += 1;

        while index < rows.len() && rows[index].0 <= end_y + 10 {
            end_y = rows[index].0;
            min_x = min_x.min(rows[index].1);
            max_x = max_x.max(rows[index].2);
            dark_sum += rows[index].3;
            index += 1;
        }

        if let Some((dominant_min_x, dominant_max_x)) =
            dominant_x_span(binary, start_y, end_y, min_x, max_x)
        {
            min_x = dominant_min_x;
            max_x = dominant_max_x;
        }

        let width = max_x.saturating_sub(min_x).saturating_add(1);
        let height = end_y.saturating_sub(start_y).saturating_add(1);
        if width < 48 || height < 8 || dark_sum < 80 {
            continue;
        }

        let band = ScanRegion {
            x: min_x,
            y: start_y,
            width,
            height,
        };
        let vertical_padding = padding.max(height);
        let expanded = expand_region(
            band,
            padding.max(width / 40).max(vertical_padding),
            image_width,
            image_height,
        );
        if plausible_region(expanded) {
            push_unique_candidate(
                &mut regions,
                CandidateDetector::GenericBinary,
                None,
                "horizontal-band",
                expanded,
            );
        }
        for aspect in [2.35_f32, 2.7, 3.2] {
            let target_height = ((width as f32 / aspect).round() as u32).max(height);
            let center_y = start_y + height / 2;
            let starts = [
                center_y.saturating_sub(target_height / 2),
                start_y.saturating_sub(padding),
                start_y,
                start_y.saturating_add(padding),
                end_y.saturating_sub(target_height / 2),
            ];
            for y in starts {
                let region = expand_region(
                    ScanRegion {
                        x: min_x,
                        y,
                        width,
                        height: target_height.min(image_height.saturating_sub(y).max(1)),
                    },
                    padding.saturating_mul(2).max(width / 40),
                    image_width,
                    image_height,
                );
                if plausible_region(region) {
                    push_unique_candidate(
                        &mut regions,
                        CandidateDetector::GenericBinary,
                        None,
                        "horizontal-aspect",
                        region,
                    );
                }
            }
        }
    }

    regions.sort_by(|a, b| region_score(b.region).total_cmp(&region_score(a.region)));
    regions.truncate(48);
    regions
}

fn dominant_x_span(
    binary: &GrayImage,
    start_y: u32,
    end_y: u32,
    min_x: u32,
    max_x: u32,
) -> Option<(u32, u32)> {
    if min_x > max_x {
        return None;
    }
    let height = end_y.saturating_sub(start_y).saturating_add(1);
    let threshold = (height / 24).max(2);
    let mut groups = Vec::new();
    let mut current: Option<(u32, u32, u32)> = None;
    let mut gap = 0u32;

    for x in min_x..=max_x {
        let mut dark = 0u32;
        for y in start_y..=end_y {
            if binary.get_pixel(x, y).0[0] == 0 {
                dark += 1;
            }
        }
        if dark >= threshold {
            match &mut current {
                Some((_, end, score)) => {
                    *end = x;
                    *score += dark;
                }
                None => current = Some((x, x, dark)),
            }
            gap = 0;
        } else if let Some((start, end, score)) = current {
            gap += 1;
            if gap > 16 {
                groups.push((start, end, score));
                current = None;
                gap = 0;
            } else {
                current = Some((start, end, score));
            }
        }
    }
    if let Some(group) = current {
        groups.push(group);
    }

    groups
        .into_iter()
        .filter(|(start, end, _)| end.saturating_sub(*start).saturating_add(1) >= 48)
        .max_by_key(|(start, end, score)| {
            score.saturating_mul(end.saturating_sub(*start).saturating_add(1))
        })
        .map(|(start, end, _)| (start, end))
}

fn plausible_region(region: ScanRegion) -> bool {
    if region.width < 80 || region.height < 28 {
        return false;
    }
    let aspect = region.width as f32 / region.height.max(1) as f32;
    (1.2..=12.0).contains(&aspect)
}

fn region_score(region: ScanRegion) -> f32 {
    let area = region.width.saturating_mul(region.height) as f32;
    let aspect = region.width as f32 / region.height.max(1) as f32;
    let aspect_score = 1.0 / (aspect - 3.0).abs().max(0.25);
    area.sqrt() * aspect_score
}

fn union_region(a: ScanRegion, b: ScanRegion) -> ScanRegion {
    let min_x = a.x.min(b.x);
    let min_y = a.y.min(b.y);
    let max_x = a.x.saturating_add(a.width).max(b.x.saturating_add(b.width));
    let max_y =
        a.y.saturating_add(a.height)
            .max(b.y.saturating_add(b.height));
    ScanRegion {
        x: min_x,
        y: min_y,
        width: max_x.saturating_sub(min_x).max(1),
        height: max_y.saturating_sub(min_y).max(1),
    }
}

fn expand_region(region: ScanRegion, padding: u32, width: u32, height: u32) -> ScanRegion {
    let max_x = region.x.saturating_add(region.width).min(width);
    let max_y = region.y.saturating_add(region.height).min(height);
    let x = region.x.saturating_sub(padding);
    let y = region.y.saturating_sub(padding);
    let expanded_max_x = max_x.saturating_add(padding).min(width);
    let expanded_max_y = max_y.saturating_add(padding).min(height);
    ScanRegion {
        x,
        y,
        width: expanded_max_x.saturating_sub(x).max(1),
        height: expanded_max_y.saturating_sub(y).max(1),
    }
}
