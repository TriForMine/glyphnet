use crate::ScanRegion;

pub(crate) fn should_try_quad_rectification(
    image_width: u32,
    image_height: u32,
    robust: bool,
) -> bool {
    robust && image_width.saturating_mul(image_height) <= 500_000
}

pub(crate) fn scan_quad_candidates(
    estimated_quad: Option<glyphnet_cv::Quad>,
    dark_bounds: Option<ScanRegion>,
    image_width: u32,
    image_height: u32,
) -> Vec<glyphnet_cv::Quad> {
    let mut quads = Vec::new();
    if let Some(quad) = estimated_quad {
        quads.push(quad);
        quads.extend(quad_variants(quad, image_width, image_height));
    }
    if let Some(bounds) = dark_bounds {
        let quad = region_to_quad(bounds);
        quads.push(quad);
        quads.extend(quad_variants(quad, image_width, image_height));
    }
    quads
}

fn region_to_quad(region: ScanRegion) -> glyphnet_cv::Quad {
    let left = region.x as f32;
    let top = region.y as f32;
    let right = region.x.saturating_add(region.width.saturating_sub(1)) as f32;
    let bottom = region.y.saturating_add(region.height.saturating_sub(1)) as f32;
    glyphnet_cv::Quad {
        top_left: glyphnet_cv::Point { x: left, y: top },
        top_right: glyphnet_cv::Point { x: right, y: top },
        bottom_right: glyphnet_cv::Point {
            x: right,
            y: bottom,
        },
        bottom_left: glyphnet_cv::Point { x: left, y: bottom },
    }
}

fn quad_variants(
    quad: glyphnet_cv::Quad,
    image_width: u32,
    image_height: u32,
) -> Vec<glyphnet_cv::Quad> {
    let center_x =
        (quad.top_left.x + quad.top_right.x + quad.bottom_left.x + quad.bottom_right.x) * 0.25;
    let center_y =
        (quad.top_left.y + quad.top_right.y + quad.bottom_left.y + quad.bottom_right.y) * 0.25;
    let mut out = Vec::new();
    for scale in [0.94_f32, 0.97, 1.03, 1.06] {
        let scaled = glyphnet_cv::Quad {
            top_left: scale_quad_point(quad.top_left, center_x, center_y, scale),
            top_right: scale_quad_point(quad.top_right, center_x, center_y, scale),
            bottom_right: scale_quad_point(quad.bottom_right, center_x, center_y, scale),
            bottom_left: scale_quad_point(quad.bottom_left, center_x, center_y, scale),
        };
        if quad_in_bounds(scaled, image_width, image_height) {
            out.push(scaled);
        }
    }
    out
}

fn scale_quad_point(
    point: glyphnet_cv::Point,
    center_x: f32,
    center_y: f32,
    scale: f32,
) -> glyphnet_cv::Point {
    glyphnet_cv::Point {
        x: center_x + (point.x - center_x) * scale,
        y: center_y + (point.y - center_y) * scale,
    }
}

fn quad_in_bounds(quad: glyphnet_cv::Quad, image_width: u32, image_height: u32) -> bool {
    let min_x = quad
        .top_left
        .x
        .min(quad.top_right.x)
        .min(quad.bottom_left.x)
        .min(quad.bottom_right.x);
    let max_x = quad
        .top_left
        .x
        .max(quad.top_right.x)
        .max(quad.bottom_left.x)
        .max(quad.bottom_right.x);
    let min_y = quad
        .top_left
        .y
        .min(quad.top_right.y)
        .min(quad.bottom_left.y)
        .min(quad.bottom_right.y);
    let max_y = quad
        .top_left
        .y
        .max(quad.top_right.y)
        .max(quad.bottom_left.y)
        .max(quad.bottom_right.y);

    min_x >= 0.0
        && min_y >= 0.0
        && max_x < image_width as f32
        && max_y < image_height as f32
        && (max_x - min_x) >= 32.0
        && (max_y - min_y) >= 32.0
}
