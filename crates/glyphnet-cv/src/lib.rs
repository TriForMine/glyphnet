//! Computer-vision primitives for GlyphNet scanners.

use glyphnet_core::TransmissionMode;
use image::{DynamicImage, GrayImage, Luma};
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
}
