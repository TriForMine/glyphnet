use crate::{GlyphError, LayoutFamily, Result, TransmissionMode, layout};

/// Concrete symbol dimensions in logical modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolGeometry {
    /// Width in modules.
    pub width: u16,
    /// Height in modules.
    pub height: u16,
}

impl SymbolGeometry {
    /// Create a validated geometry.
    pub const fn new(width: u16, height: u16) -> Self {
        Self { width, height }
    }

    /// Number of payload-capable modules under the current function-pattern rules.
    pub fn data_capacity_bits(self, layout: LayoutFamily) -> usize {
        layout::data_capacity_bits_for(layout, self.width, self.height)
    }
}

/// Layout sizing policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeometryProfile {
    /// Minimum width in modules.
    pub min_width: u16,
    /// Minimum height in modules.
    pub min_height: u16,
    /// Width growth quantum.
    pub step_width: u16,
    /// Height growth quantum.
    pub step_height: u16,
    /// Desired width/height ratio.
    pub target_aspect: f32,
}

impl GeometryProfile {
    /// Return the reference sizing policy for a mode/layout pair.
    pub const fn for_mode_layout(mode: TransmissionMode, layout: LayoutFamily) -> Self {
        match (mode, layout) {
            (_, LayoutFamily::Matrix) => {
                let dimension = mode.minimum_dimension();
                Self {
                    min_width: dimension,
                    min_height: dimension,
                    step_width: 4,
                    step_height: 4,
                    target_aspect: 1.0,
                }
            }
            (TransmissionMode::Print, LayoutFamily::RibbonWeave) => Self {
                min_width: 96,
                min_height: 36,
                step_width: 12,
                step_height: 4,
                target_aspect: 8.0 / 3.0,
            },
            (TransmissionMode::Screen, LayoutFamily::RibbonWeave) => Self {
                min_width: 128,
                min_height: 36,
                step_width: 16,
                step_height: 4,
                target_aspect: 32.0 / 9.0,
            },
            (TransmissionMode::Burst, LayoutFamily::RibbonWeave) => Self {
                min_width: 160,
                min_height: 28,
                step_width: 20,
                step_height: 4,
                target_aspect: 40.0 / 7.0,
            },
            (TransmissionMode::Print, LayoutFamily::SpectralMesh) => Self {
                min_width: 108,
                min_height: 40,
                step_width: 12,
                step_height: 4,
                target_aspect: 2.7,
            },
            (TransmissionMode::Screen, LayoutFamily::SpectralMesh) => Self {
                min_width: 144,
                min_height: 40,
                step_width: 16,
                step_height: 4,
                target_aspect: 3.6,
            },
            (TransmissionMode::Burst, LayoutFamily::SpectralMesh) => Self {
                min_width: 168,
                min_height: 36,
                step_width: 20,
                step_height: 4,
                target_aspect: 14.0 / 3.0,
            },
            (TransmissionMode::Print, LayoutFamily::PulseStream) => Self {
                min_width: 128,
                min_height: 32,
                step_width: 16,
                step_height: 4,
                target_aspect: 4.0,
            },
            (TransmissionMode::Screen, LayoutFamily::PulseStream) => Self {
                min_width: 160,
                min_height: 32,
                step_width: 20,
                step_height: 4,
                target_aspect: 5.0,
            },
            (TransmissionMode::Burst, LayoutFamily::PulseStream) => Self {
                min_width: 192,
                min_height: 32,
                step_width: 24,
                step_height: 4,
                target_aspect: 6.0,
            },
            (TransmissionMode::Print, LayoutFamily::Constellation) => Self {
                min_width: 68,
                min_height: 42,
                step_width: 8,
                step_height: 4,
                target_aspect: 1.62,
            },
            (TransmissionMode::Screen, LayoutFamily::Constellation) => Self {
                min_width: 88,
                min_height: 46,
                step_width: 8,
                step_height: 4,
                target_aspect: 1.91,
            },
            (TransmissionMode::Burst, LayoutFamily::Constellation) => Self {
                min_width: 112,
                min_height: 38,
                step_width: 12,
                step_height: 4,
                target_aspect: 2.95,
            },
            (TransmissionMode::Print, LayoutFamily::FrameGrid) => Self {
                min_width: 64,
                min_height: 40,
                step_width: 8,
                step_height: 4,
                target_aspect: 1.6,
            },
            (TransmissionMode::Screen, LayoutFamily::FrameGrid) => Self {
                min_width: 80,
                min_height: 45,
                step_width: 8,
                step_height: 5,
                target_aspect: 16.0 / 9.0,
            },
            (TransmissionMode::Burst, LayoutFamily::FrameGrid) => Self {
                min_width: 96,
                min_height: 36,
                step_width: 12,
                step_height: 4,
                target_aspect: 8.0 / 3.0,
            },
            (TransmissionMode::Print, LayoutFamily::Hexagonal) => Self {
                min_width: 72,
                min_height: 42,
                step_width: 6,
                step_height: 4,
                target_aspect: 12.0 / 7.0,
            },
            (TransmissionMode::Screen, LayoutFamily::Hexagonal) => Self {
                min_width: 96,
                min_height: 54,
                step_width: 6,
                step_height: 4,
                target_aspect: 16.0 / 9.0,
            },
            (TransmissionMode::Burst, LayoutFamily::Hexagonal) => Self {
                min_width: 120,
                min_height: 40,
                step_width: 12,
                step_height: 4,
                target_aspect: 3.0,
            },
            (TransmissionMode::Print, LayoutFamily::Radial)
            | (TransmissionMode::Screen, LayoutFamily::Radial) => Self {
                min_width: 72,
                min_height: 72,
                step_width: 6,
                step_height: 6,
                target_aspect: 1.0,
            },
            (TransmissionMode::Burst, LayoutFamily::Radial) => Self {
                min_width: 84,
                min_height: 60,
                step_width: 8,
                step_height: 6,
                target_aspect: 1.4,
            },
        }
    }
}

/// Choose the smallest reference geometry that can carry `required_bits`.
pub fn choose_symbol_geometry(
    mode: TransmissionMode,
    layout: LayoutFamily,
    required_bits: usize,
) -> Result<SymbolGeometry> {
    let profile = GeometryProfile::for_mode_layout(mode, layout);
    let mut geometry = SymbolGeometry::new(profile.min_width, profile.min_height);

    while geometry.data_capacity_bits(layout) < required_bits {
        let aspect = f32::from(geometry.width) / f32::from(geometry.height.max(1));
        if aspect < profile.target_aspect {
            geometry.width = geometry
                .width
                .checked_add(profile.step_width)
                .ok_or(GlyphError::InvalidArgument("symbol width overflow"))?;
        } else {
            geometry.height = geometry
                .height
                .checked_add(profile.step_height)
                .ok_or(GlyphError::InvalidArgument("symbol height overflow"))?;
        }
    }

    Ok(geometry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_geometry_is_wide_ribbon() {
        let print =
            choose_symbol_geometry(TransmissionMode::Print, LayoutFamily::RibbonWeave, 1).unwrap();
        let screen =
            choose_symbol_geometry(TransmissionMode::Screen, LayoutFamily::RibbonWeave, 1).unwrap();
        let burst =
            choose_symbol_geometry(TransmissionMode::Burst, LayoutFamily::RibbonWeave, 1).unwrap();

        assert_ne!(print.width, print.height);
        assert_ne!(screen.width, screen.height);
        assert_ne!(burst.width, burst.height);
        assert!(burst.width > print.width);
    }

    #[test]
    fn matrix_layout_remains_square_when_requested() {
        let geometry =
            choose_symbol_geometry(TransmissionMode::Print, LayoutFamily::Matrix, 1).unwrap();
        assert_eq!(geometry.width, geometry.height);
    }
}
