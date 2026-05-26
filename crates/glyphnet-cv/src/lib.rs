//! Computer-vision primitives for GlyphNet scanners.

use glyphnet_core::TransmissionMode;
use image::{DynamicImage, GrayImage, Luma};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use thiserror::Error;

/// Result type for CV operations.
pub type Result<T> = std::result::Result<T, CvError>;

/// Computer-vision errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CvError {
    /// The source image has no pixels.
    #[error("empty image")]
    EmptyImage,
    /// The requested sampling window is invalid.
    #[error("invalid sampling window")]
    InvalidWindow,
    /// Failed to compute a perspective transform.
    #[error("failed to compute perspective transform")]
    WarpFailed,
}

/// 2D image-space point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// X coordinate in pixels.
    pub x: f32,
    /// Y coordinate in pixels.
    pub y: f32,
}

/// Perspective quadrilateral ordered clockwise from top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    /// Top-left corner.
    pub top_left: Point,
    /// Top-right corner.
    pub top_right: Point,
    /// Bottom-right corner.
    pub bottom_right: Point,
    /// Bottom-left corner.
    pub bottom_left: Point,
}

impl Quad {
    /// Approximate quadrilateral area using the shoelace formula.
    pub fn area(self) -> f32 {
        let points = [
            self.top_left,
            self.top_right,
            self.bottom_right,
            self.bottom_left,
        ];
        let mut sum = 0.0f32;
        for index in 0..points.len() {
            let a = points[index];
            let b = points[(index + 1) % points.len()];
            sum += a.x * b.y - b.x * a.y;
        }
        sum.abs() * 0.5
    }
}

/// Visual anchor candidate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnchorCandidate {
    /// Candidate center.
    pub center: Point,
    /// Approximate marker size in pixels.
    pub size_px: f32,
    /// Confidence in the range `0.0..=1.0`.
    pub confidence: f32,
}

/// Scanner quality profile derived from protocol mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisionProfile {
    /// Transmission mode.
    pub mode: TransmissionMode,
    /// Adaptive-threshold window radius.
    pub threshold_radius: u32,
    /// Threshold bias subtracted from the local mean.
    pub threshold_bias: u8,
    /// Minimum candidate anchor size.
    pub min_anchor_px: u32,
}

impl VisionProfile {
    /// Return a conservative profile for a protocol mode.
    pub const fn for_mode(mode: TransmissionMode) -> Self {
        match mode {
            TransmissionMode::Print => Self {
                mode,
                threshold_radius: 12,
                threshold_bias: 8,
                min_anchor_px: 18,
            },
            TransmissionMode::Screen => Self {
                mode,
                threshold_radius: 6,
                threshold_bias: 4,
                min_anchor_px: 12,
            },
            TransmissionMode::Burst => Self {
                mode,
                threshold_radius: 4,
                threshold_bias: 2,
                min_anchor_px: 10,
            },
        }
    }
}

/// Convert an image into grayscale luminance.
pub fn grayscale(image: &DynamicImage) -> Result<GrayImage> {
    if image.width() == 0 || image.height() == 0 {
        return Err(CvError::EmptyImage);
    }
    Ok(image.to_luma8())
}

/// Compute a global Otsu-like threshold using a histogram.
pub fn global_threshold(image: &GrayImage) -> Result<u8> {
    if image.width() == 0 || image.height() == 0 {
        return Err(CvError::EmptyImage);
    }

    let mut hist = [0u32; 256];
    for pixel in image.pixels() {
        hist[usize::from(pixel.0[0])] += 1;
    }

    let total = f64::from(image.width()) * f64::from(image.height());
    let mut sum = 0.0;
    for (value, count) in hist.iter().enumerate() {
        sum += value as f64 * f64::from(*count);
    }

    let mut sum_background = 0.0;
    let mut weight_background = 0.0;
    let mut best_threshold = 127u8;
    let mut best_variance = 0.0;

    for (value, count) in hist.iter().enumerate() {
        weight_background += f64::from(*count);
        if weight_background == 0.0 {
            continue;
        }
        let weight_foreground = total - weight_background;
        if weight_foreground == 0.0 {
            break;
        }
        sum_background += value as f64 * f64::from(*count);
        let mean_background = sum_background / weight_background;
        let mean_foreground = (sum - sum_background) / weight_foreground;
        let variance =
            weight_background * weight_foreground * (mean_background - mean_foreground).powi(2);
        if variance > best_variance {
            best_variance = variance;
            best_threshold = value as u8;
        }
    }

    Ok(best_threshold)
}

/// Adaptive local thresholding using an integral image.
pub fn adaptive_threshold(image: &GrayImage, radius: u32, bias: u8) -> Result<GrayImage> {
    if image.width() == 0 || image.height() == 0 {
        return Err(CvError::EmptyImage);
    }
    if radius == 0 {
        return Err(CvError::InvalidWindow);
    }

    let integral = integral_image(image);
    let mut out = GrayImage::new(image.width(), image.height());
    let area = image.width().saturating_mul(image.height());

    #[cfg(not(target_arch = "wasm32"))]
    {
        if area >= 500_000 {
            let width = image.width();
            let height = image.height();
            let src = image.as_raw();
            out.as_mut()
                .par_chunks_mut(width as usize)
                .enumerate()
                .for_each(|(row_index, row)| {
                    let y = row_index as u32;
                    for x in 0..width {
                        let x0 = x.saturating_sub(radius);
                        let y0 = y.saturating_sub(radius);
                        let x1 = (x + radius).min(width - 1);
                        let y1 = (y + radius).min(height - 1);
                        let area = (x1 - x0 + 1) * (y1 - y0 + 1);
                        let sum = rect_sum(&integral, width, x0, y0, x1, y1);
                        let mean = (sum / area) as u8;
                        let threshold = mean.saturating_sub(bias);
                        let index = (y * width + x) as usize;
                        row[x as usize] = if src[index] < threshold { 0 } else { 255 };
                    }
                });
        } else {
            for y in 0..image.height() {
                for x in 0..image.width() {
                    let x0 = x.saturating_sub(radius);
                    let y0 = y.saturating_sub(radius);
                    let x1 = (x + radius).min(image.width() - 1);
                    let y1 = (y + radius).min(image.height() - 1);
                    let area = (x1 - x0 + 1) * (y1 - y0 + 1);
                    let sum = rect_sum(&integral, image.width(), x0, y0, x1, y1);
                    let mean = (sum / area) as u8;
                    let threshold = mean.saturating_sub(bias);
                    let value = if image.get_pixel(x, y).0[0] < threshold {
                        0
                    } else {
                        255
                    };
                    out.put_pixel(x, y, Luma([value]));
                }
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        for y in 0..image.height() {
            for x in 0..image.width() {
                let x0 = x.saturating_sub(radius);
                let y0 = y.saturating_sub(radius);
                let x1 = (x + radius).min(image.width() - 1);
                let y1 = (y + radius).min(image.height() - 1);
                let area = (x1 - x0 + 1) * (y1 - y0 + 1);
                let sum = rect_sum(&integral, image.width(), x0, y0, x1, y1);
                let mean = (sum / area) as u8;
                let threshold = mean.saturating_sub(bias);
                let value = if image.get_pixel(x, y).0[0] < threshold {
                    0
                } else {
                    255
                };
                out.put_pixel(x, y, Luma([value]));
            }
        }
    }

    Ok(out)
}

/// Find coarse anchor candidates from a thresholded image.
pub fn find_anchor_candidates(
    binary: &GrayImage,
    profile: VisionProfile,
) -> Result<Vec<AnchorCandidate>> {
    if binary.width() == 0 || binary.height() == 0 {
        return Err(CvError::EmptyImage);
    }

    let step = profile.min_anchor_px.max(4);
    let mut candidates = Vec::new();
    let mut y = 0;
    while y + step < binary.height() {
        let mut x = 0;
        while x + step < binary.width() {
            let darkness = dark_ratio(binary, x, y, step, step);
            if (0.35..=0.75).contains(&darkness) {
                candidates.push(AnchorCandidate {
                    center: Point {
                        x: (x + step / 2) as f32,
                        y: (y + step / 2) as f32,
                    },
                    size_px: step as f32,
                    confidence: 1.0 - (darkness - 0.55).abs() * 2.0,
                });
            }
            x += step / 2;
        }
        y += step / 2;
    }
    candidates.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    candidates.truncate(64);
    Ok(candidates)
}

/// Estimate a quadrilateral from anchor candidates or fallback image bounds.
pub fn estimate_quad(binary: &GrayImage, candidates: &[AnchorCandidate]) -> Option<Quad> {
    if candidates.len() >= 4 {
        let mut min_sum = f32::INFINITY;
        let mut max_sum = f32::NEG_INFINITY;
        let mut min_diff = f32::INFINITY;
        let mut max_diff = f32::NEG_INFINITY;
        let mut top_left = candidates[0].center;
        let mut top_right = candidates[0].center;
        let mut bottom_left = candidates[0].center;
        let mut bottom_right = candidates[0].center;
        for candidate in candidates {
            let point = candidate.center;
            let sum = point.x + point.y;
            let diff = point.x - point.y;
            if sum < min_sum {
                min_sum = sum;
                top_left = point;
            }
            if sum > max_sum {
                max_sum = sum;
                bottom_right = point;
            }
            if diff < min_diff {
                min_diff = diff;
                bottom_left = point;
            }
            if diff > max_diff {
                max_diff = diff;
                top_right = point;
            }
        }
        return Some(Quad {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        });
    }

    bounds_quad(binary)
}

/// Compute target dimensions for a quadrilateral warp.
pub fn quad_dimensions(quad: Quad) -> (u32, u32) {
    let width_top = distance(quad.top_left, quad.top_right);
    let width_bottom = distance(quad.bottom_left, quad.bottom_right);
    let height_left = distance(quad.top_left, quad.bottom_left);
    let height_right = distance(quad.top_right, quad.bottom_right);
    let width = width_top.max(width_bottom).round().max(1.0) as u32;
    let height = height_left.max(height_right).round().max(1.0) as u32;
    (width, height)
}

/// Warp a grayscale image into a rectified rectangle based on the given quad.
pub fn warp_perspective_gray(
    image: &GrayImage,
    quad: Quad,
    width: u32,
    height: u32,
) -> Result<GrayImage> {
    if image.width() == 0 || image.height() == 0 || width == 0 || height == 0 {
        return Err(CvError::EmptyImage);
    }
    let src = [
        Point { x: 0.0, y: 0.0 },
        Point {
            x: (width - 1) as f32,
            y: 0.0,
        },
        Point {
            x: (width - 1) as f32,
            y: (height - 1) as f32,
        },
        Point {
            x: 0.0,
            y: (height - 1) as f32,
        },
    ];
    let dst = [
        quad.top_left,
        quad.top_right,
        quad.bottom_right,
        quad.bottom_left,
    ];
    let homography = solve_homography(src, dst).ok_or(CvError::WarpFailed)?;

    let mut out = GrayImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = apply_homography(&homography, x as f32, y as f32);
            let value = sample_bilinear(image, sx, sy);
            out.put_pixel(x, y, Luma([value]));
        }
    }
    Ok(out)
}

fn bounds_quad(binary: &GrayImage) -> Option<Quad> {
    if binary.width() == 0 || binary.height() == 0 {
        return None;
    }
    let mut min_x = binary.width();
    let mut min_y = binary.height();
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;
    for y in 0..binary.height() {
        for x in 0..binary.width() {
            if binary.get_pixel(x, y).0[0] == 0 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                found = true;
            }
        }
    }
    if !found {
        return None;
    }
    let tl = Point {
        x: min_x as f32,
        y: min_y as f32,
    };
    let tr = Point {
        x: max_x as f32,
        y: min_y as f32,
    };
    let br = Point {
        x: max_x as f32,
        y: max_y as f32,
    };
    let bl = Point {
        x: min_x as f32,
        y: max_y as f32,
    };
    Some(Quad {
        top_left: tl,
        top_right: tr,
        bottom_right: br,
        bottom_left: bl,
    })
}

fn distance(a: Point, b: Point) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

fn solve_homography(src: [Point; 4], dst: [Point; 4]) -> Option<[f32; 9]> {
    let mut a = [[0f32; 8]; 8];
    let mut b = [0f32; 8];
    for (i, (s, d)) in src.iter().zip(dst.iter()).enumerate() {
        let row = i * 2;
        a[row] = [s.x, s.y, 1.0, 0.0, 0.0, 0.0, -d.x * s.x, -d.x * s.y];
        b[row] = d.x;
        a[row + 1] = [0.0, 0.0, 0.0, s.x, s.y, 1.0, -d.y * s.x, -d.y * s.y];
        b[row + 1] = d.y;
    }
    let solution = solve_linear_system(a, b)?;
    Some([
        solution[0],
        solution[1],
        solution[2],
        solution[3],
        solution[4],
        solution[5],
        solution[6],
        solution[7],
        1.0,
    ])
}

#[allow(clippy::needless_range_loop)]
fn solve_linear_system(mut a: [[f32; 8]; 8], mut b: [f32; 8]) -> Option<[f32; 8]> {
    let size = 8;
    for i in 0..size {
        let mut pivot = i;
        let mut pivot_value = a[i][i].abs();
        for row in (i + 1)..size {
            let value = a[row][i].abs();
            if value > pivot_value {
                pivot_value = value;
                pivot = row;
            }
        }
        if pivot_value < 1e-6 {
            return None;
        }
        if pivot != i {
            a.swap(i, pivot);
            b.swap(i, pivot);
        }
        let inv = 1.0 / a[i][i];
        for col in i..size {
            a[i][col] *= inv;
        }
        b[i] *= inv;
        for row in 0..size {
            if row == i {
                continue;
            }
            let factor = a[row][i];
            if factor == 0.0 {
                continue;
            }
            for col in i..size {
                a[row][col] -= factor * a[i][col];
            }
            b[row] -= factor * b[i];
        }
    }
    Some(b)
}

fn apply_homography(h: &[f32; 9], x: f32, y: f32) -> (f32, f32) {
    let denom = h[6] * x + h[7] * y + h[8];
    if denom.abs() < 1e-6 {
        return (x, y);
    }
    let sx = (h[0] * x + h[1] * y + h[2]) / denom;
    let sy = (h[3] * x + h[4] * y + h[5]) / denom;
    (sx, sy)
}

fn sample_bilinear(image: &GrayImage, x: f32, y: f32) -> u8 {
    if x.is_nan() || y.is_nan() {
        return 255;
    }
    let x0 = x.floor();
    let y0 = y.floor();
    let x1 = x0 + 1.0;
    let y1 = y0 + 1.0;
    let width = image.width() as f32;
    let height = image.height() as f32;
    let clamp = |value: f32, max: f32| value.max(0.0).min(max - 1.0);
    let sx0 = clamp(x0, width) as u32;
    let sy0 = clamp(y0, height) as u32;
    let sx1 = clamp(x1, width) as u32;
    let sy1 = clamp(y1, height) as u32;
    let tx = x - x0;
    let ty = y - y0;
    let p00 = image.get_pixel(sx0, sy0).0[0] as f32;
    let p10 = image.get_pixel(sx1, sy0).0[0] as f32;
    let p01 = image.get_pixel(sx0, sy1).0[0] as f32;
    let p11 = image.get_pixel(sx1, sy1).0[0] as f32;
    let top = p00 + (p10 - p00) * tx;
    let bottom = p01 + (p11 - p01) * tx;
    let value = top + (bottom - top) * ty;
    value.round().clamp(0.0, 255.0) as u8
}

fn integral_image(image: &GrayImage) -> Vec<u32> {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let mut integral = vec![0u32; (width + 1) * (height + 1)];
    for y in 0..height {
        let mut row_sum = 0u32;
        for x in 0..width {
            row_sum += u32::from(image.get_pixel(x as u32, y as u32).0[0]);
            let index = (y + 1) * (width + 1) + (x + 1);
            integral[index] = integral[index - (width + 1)] + row_sum;
        }
    }
    integral
}

fn rect_sum(integral: &[u32], width: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> u32 {
    let stride = width as usize + 1;
    let ax = x0 as usize;
    let ay = y0 as usize;
    let bx = x1 as usize + 1;
    let by = y1 as usize + 1;
    integral[by * stride + bx] + integral[ay * stride + ax]
        - integral[ay * stride + bx]
        - integral[by * stride + ax]
}

fn dark_ratio(image: &GrayImage, x0: u32, y0: u32, width: u32, height: u32) -> f32 {
    let mut dark = 0u32;
    let mut total = 0u32;
    for y in y0..(y0 + height).min(image.height()) {
        for x in x0..(x0 + width).min(image.width()) {
            if image.get_pixel(x, y).0[0] == 0 {
                dark += 1;
            }
            total += 1;
        }
    }
    dark as f32 / total.max(1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_area_uses_all_corners() {
        let quad = Quad {
            top_left: Point { x: 0.0, y: 0.0 },
            top_right: Point { x: 10.0, y: 0.0 },
            bottom_right: Point { x: 10.0, y: 10.0 },
            bottom_left: Point { x: 0.0, y: 10.0 },
        };
        assert_eq!(quad.area(), 100.0);
    }

    #[test]
    fn threshold_separates_dark_and_light_regions() {
        let mut image = GrayImage::new(10, 1);
        for x in 0..5 {
            image.put_pixel(x, 0, Luma([20]));
        }
        for x in 5..10 {
            image.put_pixel(x, 0, Luma([240]));
        }
        let threshold = global_threshold(&image).unwrap();
        assert!((20..=240).contains(&threshold));
    }

    #[test]
    fn warp_identity_preserves_center_pixel() {
        let mut image = GrayImage::new(3, 3);
        image.put_pixel(1, 1, Luma([200]));
        let quad = Quad {
            top_left: Point { x: 0.0, y: 0.0 },
            top_right: Point { x: 2.0, y: 0.0 },
            bottom_right: Point { x: 2.0, y: 2.0 },
            bottom_left: Point { x: 0.0, y: 2.0 },
        };
        let warped = warp_perspective_gray(&image, quad, 3, 3).unwrap();
        assert_eq!(warped.get_pixel(1, 1).0[0], 200);
    }
}
