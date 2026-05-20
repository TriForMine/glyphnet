use crate::{Cell, LayoutFamily};

const FINDER_SIZE: u16 = 7;
const ALIGNMENT_SIZE: u16 = 5;
const TIMING_INDEX: u16 = 8;
const HALO_RADIUS: u16 = 5;
const HALO_MARGIN: u16 = HALO_RADIUS + 2;

/// Return true when `(x, y)` is reserved for synchronization or orientation data.
pub fn is_function_module(width: u16, height: u16, x: u16, y: u16) -> bool {
    function_cell(width, height, x, y).is_some()
}

/// Return true when `(x, y)` is reserved by the selected layout family.
pub fn is_function_module_for(
    layout: LayoutFamily,
    width: u16,
    height: u16,
    x: u16,
    y: u16,
) -> bool {
    function_cell_for(layout, width, height, x, y).is_some()
}

/// Return true when `(x, y)` can carry payload data bits.
pub fn is_data_module(width: u16, height: u16, x: u16, y: u16) -> bool {
    x < width && y < height && !is_function_module(width, height, x, y)
}

/// Return true when `(x, y)` can carry payload data bits for the selected layout family.
pub fn is_data_module_for(layout: LayoutFamily, width: u16, height: u16, x: u16, y: u16) -> bool {
    x < width && y < height && !is_function_module_for(layout, width, height, x, y)
}

/// Count payload-carrying modules in a matrix.
pub fn data_capacity_bits(width: u16, height: u16) -> usize {
    data_capacity_bits_for(LayoutFamily::Matrix, width, height)
}

/// Count payload-carrying modules for the selected layout family.
pub fn data_capacity_bits_for(layout: LayoutFamily, width: u16, height: u16) -> usize {
    let mut count = 0usize;
    for y in 0..height {
        for x in 0..width {
            if is_data_module_for(layout, width, height, x, y) {
                count += 1;
            }
        }
    }
    count
}

/// Determine the matrix-compatibility function-pattern cell at `(x, y)`, if any.
pub fn function_cell(width: u16, height: u16, x: u16, y: u16) -> Option<Cell> {
    matrix_function_cell(width, height, x, y)
}

/// Determine the selected layout's function-pattern cell at `(x, y)`, if any.
pub fn function_cell_for(
    layout: LayoutFamily,
    width: u16,
    height: u16,
    x: u16,
    y: u16,
) -> Option<Cell> {
    match layout {
        LayoutFamily::Matrix => matrix_function_cell(width, height, x, y),
        LayoutFamily::RibbonWeave | LayoutFamily::SpectralMesh | LayoutFamily::PulseStream => {
            ribbon_weave_function_cell(width, height, x, y)
        }
        LayoutFamily::Constellation
        | LayoutFamily::FrameGrid
        | LayoutFamily::Hexagonal
        | LayoutFamily::Radial => constellation_function_cell(width, height, x, y),
    }
}

fn ribbon_weave_function_cell(width: u16, height: u16, x: u16, y: u16) -> Option<Cell> {
    if x >= width || y >= height {
        return None;
    }

    if let Some(cell) = weave_totem_cell(width, height, x, y) {
        return Some(cell);
    }
    if let Some(cell) = weave_chevron_rail_cell(width, height, x, y) {
        return Some(cell);
    }
    if let Some(cell) = weave_phase_trace_cell(width, height, x, y) {
        return Some(cell);
    }

    None
}

fn matrix_function_cell(width: u16, height: u16, x: u16, y: u16) -> Option<Cell> {
    if x >= width || y >= height {
        return None;
    }

    if let Some(cell) = finder_cell(width, height, x, y) {
        return Some(cell);
    }

    if width > TIMING_INDEX + FINDER_SIZE
        && height > TIMING_INDEX + FINDER_SIZE
        && (x == TIMING_INDEX || y == TIMING_INDEX)
    {
        return Some(Cell::Timing((x + y) % 2 == 0));
    }

    if width >= 33 && height >= 33 {
        let cx = width / 2;
        let cy = height / 2;
        if x.abs_diff(cx) <= ALIGNMENT_SIZE / 2 && y.abs_diff(cy) <= ALIGNMENT_SIZE / 2 {
            let edge = x.abs_diff(cx) == ALIGNMENT_SIZE / 2 || y.abs_diff(cy) == ALIGNMENT_SIZE / 2;
            let center = x == cx && y == cy;
            return Some(Cell::Alignment(edge || center));
        }
    }

    None
}

fn weave_totem_cell(width: u16, height: u16, x: u16, y: u16) -> Option<Cell> {
    if width < 48 || height < 20 || y < 3 || y >= height.saturating_sub(3) {
        return None;
    }

    let left_a = 5;
    let left_b = 7;
    let right_a = width.saturating_sub(8);
    let right_b = width.saturating_sub(6);
    let in_totem = x == left_a || x == left_b || x == right_a || x == right_b;
    if !in_totem {
        return None;
    }

    let local = y - 3;
    let dark = matches!(local % 7, 0 | 1 | 4) || (x == left_a && local % 5 == 2);
    Some(Cell::Anchor(dark))
}

fn weave_chevron_rail_cell(width: u16, height: u16, x: u16, y: u16) -> Option<Cell> {
    if width < 64 || height < 24 {
        return None;
    }
    let top_a = 3;
    let top_b = 5;
    let bottom_a = height.saturating_sub(4);
    let bottom_b = height.saturating_sub(6);
    if y != top_a && y != top_b && y != bottom_a && y != bottom_b {
        return None;
    }
    if x < 14 || x >= width.saturating_sub(14) {
        return None;
    }

    let phase = (x - 14) % 12;
    let rising = matches!(phase, 0 | 1 | 2 | 6 | 7 | 8);
    let dark = match y {
        value if value == top_a || value == bottom_b => rising,
        _ => !rising,
    };
    Some(Cell::Signature(dark))
}

fn weave_phase_trace_cell(width: u16, height: u16, x: u16, y: u16) -> Option<Cell> {
    if width < 64 || height < 24 || x < 16 || x >= width.saturating_sub(16) {
        return None;
    }
    let center = height / 2;
    let offset = match (x / 5) % 4 {
        0 => 0,
        1 => 1,
        2 => 0,
        _ => u16::MAX,
    };
    let trace_y = if offset == u16::MAX {
        center.saturating_sub(1)
    } else {
        center + offset
    };
    if y == trace_y {
        return Some(Cell::Timing((x / 2 + y) % 2 == 0));
    }
    None
}

fn constellation_function_cell(width: u16, height: u16, x: u16, y: u16) -> Option<Cell> {
    if x >= width || y >= height {
        return None;
    }

    if let Some(cell) = signature_rail_cell(width, height, x, y) {
        return Some(cell);
    }
    let anchors = constellation_anchor_centers(width, height);
    if let Some(cell) = halo_anchor_cell(anchors, x, y) {
        return Some(cell);
    }
    if let Some(cell) = diagonal_timing_cell(anchors, x, y) {
        return Some(cell);
    }
    if let Some(cell) = orientation_glyph_cell(width, height, x, y) {
        return Some(cell);
    }

    None
}

fn signature_rail_cell(width: u16, height: u16, x: u16, y: u16) -> Option<Cell> {
    if width < 48 || height < 28 {
        return None;
    }

    let start_x = HALO_MARGIN;
    let end_x = (start_x + 34).min(width.saturating_sub(HALO_MARGIN + 1));
    let rail_a = height.saturating_sub(5);
    let rail_b = height.saturating_sub(7);
    if x < start_x || x > end_x || (y != rail_a && y != rail_b) {
        return None;
    }

    let lane = x - start_x;
    let base = matches!(lane % 8, 0 | 1 | 3 | 6);
    let dark = if y == rail_a {
        base
    } else {
        !base && lane % 8 != 7
    };
    Some(Cell::Signature(dark))
}

fn finder_cell(width: u16, height: u16, x: u16, y: u16) -> Option<Cell> {
    let origins = [
        (0, 0),
        (width.saturating_sub(FINDER_SIZE), 0),
        (0, height.saturating_sub(FINDER_SIZE)),
    ];

    for (origin_x, origin_y) in origins {
        if x >= origin_x
            && x < origin_x + FINDER_SIZE
            && y >= origin_y
            && y < origin_y + FINDER_SIZE
        {
            let lx = x - origin_x;
            let ly = y - origin_y;
            let border = lx == 0 || ly == 0 || lx == FINDER_SIZE - 1 || ly == FINDER_SIZE - 1;
            let center = (2..=4).contains(&lx) && (2..=4).contains(&ly);
            return Some(Cell::Finder(border || center));
        }
    }

    None
}

fn constellation_anchor_centers(width: u16, height: u16) -> [(u16, u16); 3] {
    [
        (
            clamp_anchor(width / 6, width),
            clamp_anchor(height / 2, height),
        ),
        (
            clamp_anchor(width * 3 / 4, width),
            clamp_anchor(height / 4, height),
        ),
        (
            clamp_anchor(width * 5 / 6, width),
            clamp_anchor(height * 3 / 4, height),
        ),
    ]
}

fn clamp_anchor(value: u16, limit: u16) -> u16 {
    if limit <= HALO_MARGIN * 2 {
        return limit / 2;
    }
    value.clamp(HALO_MARGIN, limit - HALO_MARGIN - 1)
}

fn halo_anchor_cell(anchors: [(u16, u16); 3], x: u16, y: u16) -> Option<Cell> {
    for (center_x, center_y) in anchors {
        let dx = i32::from(x) - i32::from(center_x);
        let dy = i32::from(y) - i32::from(center_y);
        let d2 = dx * dx + dy * dy;
        if d2 <= i32::from(HALO_RADIUS * HALO_RADIUS) {
            let dark = (16..=25).contains(&d2)
                || d2 <= 1
                || (dx == 0 && dy.abs() <= i32::from(HALO_RADIUS))
                || (dy == 0 && dx.abs() <= i32::from(HALO_RADIUS));
            return Some(Cell::Anchor(dark));
        }
    }
    None
}

fn diagonal_timing_cell(anchors: [(u16, u16); 3], x: u16, y: u16) -> Option<Cell> {
    let lines = [(anchors[0], anchors[1]), (anchors[0], anchors[2])];
    for (line_index, (start, end)) in lines.into_iter().enumerate() {
        let Some(line_y) = line_y_at_x(start, end, x) else {
            continue;
        };
        if i32::from(y).abs_diff(line_y) == 0 {
            return Some(Cell::Timing((x + y + line_index as u16) % 2 == 0));
        }
    }
    None
}

fn line_y_at_x(start: (u16, u16), end: (u16, u16), x: u16) -> Option<i32> {
    let start_x = i32::from(start.0);
    let start_y = i32::from(start.1);
    let end_x = i32::from(end.0);
    let end_y = i32::from(end.1);
    let x = i32::from(x);
    let min_x = start_x.min(end_x);
    let max_x = start_x.max(end_x);
    if x <= min_x + i32::from(HALO_RADIUS) || x >= max_x - i32::from(HALO_RADIUS) {
        return None;
    }

    let dx = end_x - start_x;
    if dx == 0 {
        return None;
    }
    let dy = end_y - start_y;
    Some(start_y + ((x - start_x) * dy + dx.signum() * (dx.abs() / 2)) / dx)
}

fn orientation_glyph_cell(width: u16, height: u16, x: u16, y: u16) -> Option<Cell> {
    if width < 27 || height < 27 {
        return None;
    }
    let center_x = i32::from(width / 2);
    let center_y = i32::from(height / 2);
    let dx = i32::from(x) - center_x;
    let dy = i32::from(y) - center_y;
    let manhattan = dx.abs() + dy.abs();
    if manhattan <= 5 {
        let dark = manhattan == 5 || (dx == 0 && dy.abs() <= 2) || (dy == 0 && dx.abs() <= 2);
        return Some(Cell::Alignment(dark));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_three_finder_patterns() {
        assert!(matches!(
            function_cell(45, 45, 0, 0),
            Some(Cell::Finder(true))
        ));
        assert!(matches!(
            function_cell(45, 45, 44, 0),
            Some(Cell::Finder(true))
        ));
        assert!(matches!(
            function_cell(45, 45, 0, 44),
            Some(Cell::Finder(true))
        ));
    }

    #[test]
    fn capacity_excludes_function_patterns() {
        let capacity = data_capacity_bits(45, 45);
        assert!(capacity < 45 * 45);
        assert!(capacity > 1_700);
    }

    #[test]
    fn constellation_has_no_corner_markers() {
        assert!(function_cell_for(LayoutFamily::Constellation, 64, 40, 0, 0).is_none());
        assert!(function_cell_for(LayoutFamily::Constellation, 64, 40, 63, 0).is_none());
        assert!(function_cell_for(LayoutFamily::Constellation, 64, 40, 0, 39).is_none());
        assert!(matches!(
            function_cell_for(LayoutFamily::Constellation, 64, 40, 10, 20),
            Some(Cell::Anchor(true))
        ));
    }

    #[test]
    fn constellation_reserves_signature_rail() {
        assert!(matches!(
            function_cell_for(LayoutFamily::Constellation, 68, 42, HALO_MARGIN, 37),
            Some(Cell::Signature(true))
        ));
        assert!(matches!(
            function_cell_for(LayoutFamily::Constellation, 68, 42, HALO_MARGIN + 2, 37),
            Some(Cell::Signature(false))
        ));
    }

    #[test]
    fn ribbon_weave_has_totems_and_chevrons_without_corner_boxes() {
        assert!(function_cell_for(LayoutFamily::RibbonWeave, 96, 36, 0, 0).is_none());
        assert!(matches!(
            function_cell_for(LayoutFamily::RibbonWeave, 96, 36, 5, 3),
            Some(Cell::Anchor(true))
        ));
        assert!(matches!(
            function_cell_for(LayoutFamily::RibbonWeave, 96, 36, 14, 3),
            Some(Cell::Signature(true))
        ));
    }
}
