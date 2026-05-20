//! Rendering pipeline for GlyphNet matrices.

use glyphnet_core::{Cell, ColorEncoding, LayoutFamily, SymbolDescriptor, SymbolMatrix};
use image::{Rgba, RgbaImage};
use thiserror::Error;

/// Result type for rendering operations.
pub type Result<T> = std::result::Result<T, RenderError>;

/// Renderer errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// The requested module size is zero.
    #[error("module size must be greater than zero")]
    ZeroModuleSize,
    /// The requested image dimensions overflowed `u32`.
    #[error("rendered image dimensions overflowed")]
    DimensionOverflow,
}

/// RGBA color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

impl Color {
    /// Opaque black.
    pub const BLACK: Self = Self([0, 0, 0, 255]);
    /// Opaque white.
    pub const WHITE: Self = Self([255, 255, 255, 255]);
    /// Blue tracking marker color for diagnostics and screen mode experiments.
    pub const TRACKING_BLUE: Self = Self([0, 90, 220, 255]);
    /// Warm timing marker color for diagnostics.
    pub const TIMING_AMBER: Self = Self([230, 150, 20, 255]);
    /// Dark blue spectral data lane.
    pub const LANE_BLUE: Self = Self([8, 24, 96, 255]);
    /// Dark teal spectral data lane.
    pub const LANE_TEAL: Self = Self([0, 56, 64, 255]);
    /// Dark violet spectral data lane.
    pub const LANE_VIOLET: Self = Self([52, 24, 100, 255]);
    /// Dark magenta signal marker used by animated profiles.
    pub const SIGNAL_MAGENTA: Self = Self([96, 0, 72, 255]);
}

impl From<Color> for Rgba<u8> {
    fn from(value: Color) -> Self {
        Self(value.0)
    }
}

/// Visual primitive used to draw dark modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleShape {
    /// Full square modules, retained for compatibility and dense print experiments.
    Square,
    /// Single-cell rounded stroke.
    Capsule,
    /// Continuous horizontal strokes used by the default ribbon-weave renderer.
    Ribbon,
    /// Diamond modules for timing, signature, and orientation glyphs.
    Diamond,
}

/// Render configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOptions {
    /// Pixels per module.
    pub module_px: u32,
    /// Quiet-zone width in modules.
    pub quiet_zone_modules: u32,
    /// Background color.
    pub light: Color,
    /// Data dark color.
    pub dark: Color,
    /// Anchor marker dark color.
    pub anchor: Color,
    /// Timing marker dark color.
    pub timing: Color,
    /// Alignment marker dark color.
    pub alignment: Color,
    /// Data module shape.
    pub data_shape: ModuleShape,
    /// Synchronization marker shape.
    pub marker_shape: ModuleShape,
    /// Timing/signature shape.
    pub signal_shape: ModuleShape,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            module_px: 8,
            quiet_zone_modules: 4,
            light: Color::WHITE,
            dark: Color::BLACK,
            anchor: Color::BLACK,
            timing: Color::BLACK,
            alignment: Color::BLACK,
            data_shape: ModuleShape::Ribbon,
            marker_shape: ModuleShape::Capsule,
            signal_shape: ModuleShape::Diamond,
        }
    }
}

impl RenderOptions {
    /// Build renderer options from a symbol descriptor.
    ///
    /// This keeps visual choices close to the protocol profile without making
    /// renderers parse profile metadata themselves.
    pub fn for_descriptor(descriptor: &SymbolDescriptor) -> Self {
        let mut options = Self::default();
        match descriptor.layout {
            LayoutFamily::RibbonWeave => {}
            LayoutFamily::SpectralMesh => {
                options.anchor = Color::TRACKING_BLUE;
                options.timing = Color::TIMING_AMBER;
                options.alignment = Color::SIGNAL_MAGENTA;
                options.data_shape = ModuleShape::Ribbon;
                options.marker_shape = ModuleShape::Capsule;
                options.signal_shape = ModuleShape::Diamond;
            }
            LayoutFamily::PulseStream => {
                options.anchor = Color::SIGNAL_MAGENTA;
                options.timing = Color::TIMING_AMBER;
                options.alignment = Color::TRACKING_BLUE;
                options.data_shape = ModuleShape::Ribbon;
                options.marker_shape = ModuleShape::Capsule;
                options.signal_shape = ModuleShape::Diamond;
            }
            LayoutFamily::Constellation
            | LayoutFamily::FrameGrid
            | LayoutFamily::Hexagonal
            | LayoutFamily::Radial => {
                options.anchor = Color::TRACKING_BLUE;
                options.timing = Color::TIMING_AMBER;
                options.data_shape = ModuleShape::Capsule;
                options.marker_shape = ModuleShape::Diamond;
                options.signal_shape = ModuleShape::Diamond;
            }
            LayoutFamily::Matrix => {
                options.data_shape = ModuleShape::Square;
                options.marker_shape = ModuleShape::Square;
                options.signal_shape = ModuleShape::Square;
            }
        }

        match descriptor.color {
            ColorEncoding::Mono => {}
            ColorEncoding::LimitedPalette => {
                options.anchor = Color::TRACKING_BLUE;
                options.timing = Color::TIMING_AMBER;
            }
            ColorEncoding::Rgb => {
                options.dark = Color::LANE_BLUE;
                options.anchor = Color::TRACKING_BLUE;
                options.timing = Color::TIMING_AMBER;
                options.alignment = Color::SIGNAL_MAGENTA;
            }
            ColorEncoding::Adaptive => {
                options.dark = Color::LANE_VIOLET;
                options.anchor = Color::SIGNAL_MAGENTA;
                options.timing = Color::TIMING_AMBER;
                options.alignment = Color::TRACKING_BLUE;
            }
        }
        options
    }

    fn color_for_at(
        &self,
        layout: LayoutFamily,
        module_x: u16,
        module_y: u16,
        cell: Cell,
    ) -> Color {
        match cell {
            Cell::Finder(true) | Cell::Anchor(true) => self.anchor,
            Cell::Timing(true) => self.timing,
            Cell::Alignment(true) | Cell::Signature(true) => self.alignment,
            _ => self.data_color_for(layout, module_x, module_y),
        }
    }

    const fn data_color_for(&self, layout: LayoutFamily, module_x: u16, module_y: u16) -> Color {
        match layout {
            LayoutFamily::SpectralMesh => match (module_x / 5 + module_y / 2) % 3 {
                0 => Color::LANE_BLUE,
                1 => Color::LANE_TEAL,
                _ => Color::LANE_VIOLET,
            },
            LayoutFamily::PulseStream => match (module_x / 8 + module_y) % 3 {
                0 => Color::LANE_VIOLET,
                1 => Color::LANE_BLUE,
                _ => Color::SIGNAL_MAGENTA,
            },
            _ => self.dark,
        }
    }

    const fn shape_for(&self, cell: Cell) -> ModuleShape {
        match cell {
            Cell::Finder(true) | Cell::Anchor(true) => self.marker_shape,
            Cell::Timing(true) | Cell::Alignment(true) | Cell::Signature(true) => self.signal_shape,
            _ => self.data_shape,
        }
    }
}

/// Raster renderer for PNG/JPEG pipelines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterRenderer {
    options: RenderOptions,
}

impl RasterRenderer {
    /// Create a renderer from options.
    pub const fn new(options: RenderOptions) -> Self {
        Self { options }
    }

    /// Render a matrix to an RGBA image.
    pub fn render(&self, matrix: &SymbolMatrix) -> Result<RgbaImage> {
        if self.options.module_px == 0 {
            return Err(RenderError::ZeroModuleSize);
        }

        if is_ribbon_layout(matrix.layout()) {
            return self.render_ribbon_weave(matrix);
        }

        let total_modules_x = u32::from(matrix.width())
            .checked_add(self.options.quiet_zone_modules * 2)
            .ok_or(RenderError::DimensionOverflow)?;
        let total_modules_y = u32::from(matrix.height())
            .checked_add(self.options.quiet_zone_modules * 2)
            .ok_or(RenderError::DimensionOverflow)?;
        let width = total_modules_x
            .checked_mul(self.options.module_px)
            .ok_or(RenderError::DimensionOverflow)?;
        let height = total_modules_y
            .checked_mul(self.options.module_px)
            .ok_or(RenderError::DimensionOverflow)?;

        let mut image = RgbaImage::from_pixel(width, height, self.options.light.into());
        for y in 0..matrix.height() {
            for x in 0..matrix.width() {
                let cell = matrix.get(x, y).expect("coordinates are inside matrix");
                let color = self.options.color_for_at(matrix.layout(), x, y, cell);
                if cell.is_dark() {
                    self.fill_module(&mut image, x, y, cell, color);
                }
            }
        }
        Ok(image)
    }

    fn render_ribbon_weave(&self, matrix: &SymbolMatrix) -> Result<RgbaImage> {
        let width = (u32::from(matrix.width()) + self.options.quiet_zone_modules * 2)
            .checked_mul(self.options.module_px)
            .ok_or(RenderError::DimensionOverflow)?;
        let height = (u32::from(matrix.height()) + self.options.quiet_zone_modules * 2)
            .checked_mul(self.options.module_px)
            .ok_or(RenderError::DimensionOverflow)?;
        let mut image = RgbaImage::from_pixel(width, height, self.options.light.into());

        for y in 0..matrix.height() {
            let mut x = 0u16;
            while x < matrix.width() {
                let cell = matrix.get(x, y).expect("coordinates are inside matrix");
                if matches!(cell, Cell::Data(true) | Cell::Dark) {
                    let start = x;
                    while x < matrix.width() {
                        let run_cell = matrix.get(x, y).expect("coordinates are inside matrix");
                        if !matches!(run_cell, Cell::Data(true) | Cell::Dark) {
                            break;
                        }
                        x += 1;
                    }
                    self.fill_ribbon_run(
                        &mut image,
                        start,
                        y,
                        x - start,
                        self.options.data_color_for(matrix.layout(), start, y),
                    );
                    continue;
                }

                if cell.is_dark() {
                    self.fill_module(
                        &mut image,
                        x,
                        y,
                        cell,
                        self.options.color_for_at(matrix.layout(), x, y, cell),
                    );
                }
                x += 1;
            }
        }

        Ok(image)
    }

    fn fill_module(
        &self,
        image: &mut RgbaImage,
        module_x: u16,
        module_y: u16,
        cell: Cell,
        color: Color,
    ) {
        let start_x =
            (u32::from(module_x) + self.options.quiet_zone_modules) * self.options.module_px;
        let start_y =
            (u32::from(module_y) + self.options.quiet_zone_modules) * self.options.module_px;
        let rgba = color.into();
        match self.options.shape_for(cell) {
            ModuleShape::Square => fill_rect(image, start_x, start_y, self.options.module_px, rgba),
            ModuleShape::Capsule | ModuleShape::Ribbon => {
                fill_capsule(image, start_x, start_y, self.options.module_px, rgba);
            }
            ModuleShape::Diamond => {
                fill_diamond(image, start_x, start_y, self.options.module_px, rgba);
            }
        }
    }

    fn fill_ribbon_run(
        &self,
        image: &mut RgbaImage,
        start_module_x: u16,
        module_y: u16,
        run_len: u16,
        color: Color,
    ) {
        let start_x =
            (u32::from(start_module_x) + self.options.quiet_zone_modules) * self.options.module_px;
        let start_y =
            (u32::from(module_y) + self.options.quiet_zone_modules) * self.options.module_px;
        fill_horizontal_capsule(
            image,
            start_x,
            start_y,
            u32::from(run_len) * self.options.module_px,
            self.options.module_px,
            color.into(),
        );
    }
}

impl Default for RasterRenderer {
    fn default() -> Self {
        Self::new(RenderOptions::default())
    }
}

/// SVG renderer for web/native vector workflows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgRenderer {
    options: RenderOptions,
}

impl SvgRenderer {
    /// Create an SVG renderer from options.
    pub const fn new(options: RenderOptions) -> Self {
        Self { options }
    }

    /// Render a matrix to an SVG document string.
    pub fn render(&self, matrix: &SymbolMatrix) -> Result<String> {
        if self.options.module_px == 0 {
            return Err(RenderError::ZeroModuleSize);
        }
        if is_ribbon_layout(matrix.layout()) {
            return self.render_ribbon_weave(matrix);
        }
        let total_modules_x = u32::from(matrix.width()) + self.options.quiet_zone_modules * 2;
        let total_modules_y = u32::from(matrix.height()) + self.options.quiet_zone_modules * 2;
        let width = total_modules_x * self.options.module_px;
        let height = total_modules_y * self.options.module_px;

        let mut svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}" shape-rendering="geometricPrecision">"#
        );
        svg.push_str(&format!(
            r#"<rect width="100%" height="100%" fill="{}"/>"#,
            hex(self.options.light)
        ));

        for y in 0..matrix.height() {
            for x in 0..matrix.width() {
                let cell = matrix.get(x, y).expect("coordinates are inside matrix");
                if !cell.is_dark() {
                    continue;
                }
                let px = (u32::from(x) + self.options.quiet_zone_modules) * self.options.module_px;
                let py = (u32::from(y) + self.options.quiet_zone_modules) * self.options.module_px;
                svg.push_str(&self.svg_module(matrix.layout(), x, y, px, py, cell));
            }
        }

        svg.push_str("</svg>");
        Ok(svg)
    }

    fn render_ribbon_weave(&self, matrix: &SymbolMatrix) -> Result<String> {
        let total_modules_x = u32::from(matrix.width()) + self.options.quiet_zone_modules * 2;
        let total_modules_y = u32::from(matrix.height()) + self.options.quiet_zone_modules * 2;
        let width = total_modules_x * self.options.module_px;
        let height = total_modules_y * self.options.module_px;
        let mut svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}" shape-rendering="geometricPrecision">"#
        );
        svg.push_str(&format!(
            r#"<rect width="100%" height="100%" fill="{}"/>"#,
            hex(self.options.light)
        ));

        for y in 0..matrix.height() {
            let mut x = 0u16;
            while x < matrix.width() {
                let cell = matrix.get(x, y).expect("coordinates are inside matrix");
                if matches!(cell, Cell::Data(true) | Cell::Dark) {
                    let start = x;
                    while x < matrix.width() {
                        let run_cell = matrix.get(x, y).expect("coordinates are inside matrix");
                        if !matches!(run_cell, Cell::Data(true) | Cell::Dark) {
                            break;
                        }
                        x += 1;
                    }
                    svg.push_str(&self.svg_ribbon_run(matrix.layout(), start, y, x - start));
                    continue;
                }
                if cell.is_dark() {
                    let px =
                        (u32::from(x) + self.options.quiet_zone_modules) * self.options.module_px;
                    let py =
                        (u32::from(y) + self.options.quiet_zone_modules) * self.options.module_px;
                    svg.push_str(&self.svg_module(matrix.layout(), x, y, px, py, cell));
                }
                x += 1;
            }
        }

        svg.push_str("</svg>");
        Ok(svg)
    }

    fn svg_module(
        &self,
        layout: LayoutFamily,
        module_x: u16,
        module_y: u16,
        px: u32,
        py: u32,
        cell: Cell,
    ) -> String {
        let size = self.options.module_px;
        let fill = hex(self.options.color_for_at(layout, module_x, module_y, cell));
        match self.options.shape_for(cell) {
            ModuleShape::Square => {
                format!(r#"<rect x="{px}" y="{py}" width="{size}" height="{size}" fill="{fill}"/>"#)
            }
            ModuleShape::Capsule | ModuleShape::Ribbon => {
                let pad = (size as f32 * 0.14).round() as u32;
                let height = size.saturating_sub(pad * 2).max(1);
                let radius = height as f32 / 2.0;
                format!(
                    r#"<rect class="glyphnet-stroke" x="{px}" y="{}" width="{size}" height="{height}" rx="{radius:.2}" ry="{radius:.2}" fill="{fill}"/>"#,
                    py + pad
                )
            }
            ModuleShape::Diamond => {
                let cx = px + size / 2;
                let cy = py + size / 2;
                let points = format!("{cx},{py} {},{cy} {cx},{} {px},{cy}", px + size, py + size);
                format!(r#"<polygon points="{points}" fill="{fill}"/>"#)
            }
        }
    }

    fn svg_ribbon_run(
        &self,
        layout: LayoutFamily,
        start_module_x: u16,
        module_y: u16,
        run_len: u16,
    ) -> String {
        let size = self.options.module_px;
        let x = (u32::from(start_module_x) + self.options.quiet_zone_modules) * size;
        let y = (u32::from(module_y) + self.options.quiet_zone_modules) * size;
        let width = u32::from(run_len) * size;
        let pad = (size as f32 * 0.14).round() as u32;
        let height = size.saturating_sub(pad * 2).max(1);
        let radius = height as f32 / 2.0;
        format!(
            r#"<rect class="glyphnet-ribbon" x="{x}" y="{}" width="{width}" height="{height}" rx="{radius:.2}" ry="{radius:.2}" fill="{}"/>"#,
            y + pad,
            hex(self
                .options
                .data_color_for(layout, start_module_x, module_y))
        )
    }
}

impl Default for SvgRenderer {
    fn default() -> Self {
        Self::new(RenderOptions::default())
    }
}

fn hex(color: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", color.0[0], color.0[1], color.0[2])
}

const fn is_ribbon_layout(layout: LayoutFamily) -> bool {
    matches!(
        layout,
        LayoutFamily::RibbonWeave | LayoutFamily::SpectralMesh | LayoutFamily::PulseStream
    )
}

fn fill_rect(image: &mut RgbaImage, start_x: u32, start_y: u32, size: u32, color: Rgba<u8>) {
    for y in start_y..start_y + size {
        for x in start_x..start_x + size {
            image.put_pixel(x, y, color);
        }
    }
}

fn fill_capsule(image: &mut RgbaImage, start_x: u32, start_y: u32, size: u32, color: Rgba<u8>) {
    fill_horizontal_capsule(image, start_x, start_y, size, size, color);
}

fn fill_horizontal_capsule(
    image: &mut RgbaImage,
    start_x: u32,
    start_y: u32,
    width: u32,
    module_size: u32,
    color: Rgba<u8>,
) {
    let pad_y = (module_size as f32 * 0.14).round() as u32;
    let height = module_size.saturating_sub(pad_y * 2).max(1);
    let radius = height as f32 / 2.0;
    let center_left = start_x as f32 + radius;
    let center_right = start_x as f32 + width.saturating_sub(1) as f32 - radius;
    let center_y = start_y as f32 + pad_y as f32 + radius;
    let radius_squared = radius * radius;
    let end_x = start_x + width.max(1);
    let end_y = start_y + pad_y + height;
    for y in start_y..start_y + module_size {
        if y < start_y + pad_y || y >= end_y {
            continue;
        }
        for x in start_x..end_x {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let inside_body = px >= center_left && px <= center_right;
            let dx = if px < center_left {
                px - center_left
            } else if px > center_right {
                px - center_right
            } else {
                0.0
            };
            let dy = py - center_y;
            if inside_body || dx * dx + dy * dy <= radius_squared {
                image.put_pixel(x, y, color);
            }
        }
    }
}

fn fill_diamond(image: &mut RgbaImage, start_x: u32, start_y: u32, size: u32, color: Rgba<u8>) {
    let center = size as i32 / 2;
    let radius = center.max(1);
    for y in 0..size {
        for x in 0..size {
            let dx = x as i32 - center;
            let dy = y as i32 - center;
            if dx.abs() + dy.abs() <= radius {
                image.put_pixel(start_x + x, start_y + y, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use glyphnet_core::{LayoutFamily, SymbolMatrix};

    use super::*;

    #[test]
    fn raster_dimensions_include_quiet_zone() {
        let matrix = SymbolMatrix::new(45, 45);
        let image = RasterRenderer::default().render(&matrix).unwrap();
        assert_eq!(image.width(), (45 + 8) * 8);
        assert_eq!(image.height(), (45 + 8) * 8);
    }

    #[test]
    fn svg_contains_ribbon_weave_primitives() {
        let mut matrix = SymbolMatrix::new(96, 36);
        matrix.write_data_bits(std::iter::repeat_n(true, matrix.data_capacity_bits()));
        let svg = SvgRenderer::default().render(&matrix).unwrap();
        assert!(svg.contains("glyphnet-ribbon"));
        assert!(svg.contains("<polygon"));
    }

    #[test]
    fn svg_spectral_mesh_uses_multicolor_ribbons() {
        let mut matrix = SymbolMatrix::with_layout(128, 36, LayoutFamily::SpectralMesh);
        matrix.write_data_bits(std::iter::repeat_n(true, matrix.data_capacity_bits()));
        let svg = SvgRenderer::default().render(&matrix).unwrap();
        assert!(svg.contains("#081860"));
        assert!(svg.contains("#003840"));
        assert!(svg.contains("#341864"));
    }
}
