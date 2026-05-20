use serde::{Deserialize, Serialize};

use crate::{GlyphError, LayoutFamily, Result, layout};

/// A visual module within a symbol matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cell {
    /// Unset or background module.
    Light,
    /// Explicit dark module.
    Dark,
    /// Payload data bit.
    Data(bool),
    /// Matrix-compatibility finder marker module.
    Finder(bool),
    /// Non-corner constellation or halo anchor module.
    Anchor(bool),
    /// Timing marker module.
    Timing(bool),
    /// Alignment marker module.
    Alignment(bool),
    /// Fixed visual signature module used to distinguish GlyphNet from QR-like codes.
    Signature(bool),
    /// Reserved module that should render as light.
    Reserved,
}

impl Cell {
    /// Return true when this module should render as dark in monochrome output.
    pub const fn is_dark(self) -> bool {
        match self {
            Self::Light | Self::Reserved => false,
            Self::Dark => true,
            Self::Data(value)
            | Self::Finder(value)
            | Self::Anchor(value)
            | Self::Timing(value)
            | Self::Alignment(value)
            | Self::Signature(value) => value,
        }
    }

    /// Return a payload bit if this cell carries one.
    pub const fn data_bit(self) -> Option<bool> {
        match self {
            Self::Data(value) => Some(value),
            _ => None,
        }
    }
}

/// Dense symbol matrix with row-major module storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolMatrix {
    width: u16,
    height: u16,
    layout: LayoutFamily,
    cells: Vec<Cell>,
}

impl SymbolMatrix {
    /// Create a light-filled matrix and apply the reference function patterns.
    pub fn new(width: u16, height: u16) -> Self {
        Self::with_layout(width, height, LayoutFamily::RibbonWeave)
    }

    /// Create a light-filled matrix with an explicit layout family.
    pub fn with_layout(width: u16, height: u16, layout: LayoutFamily) -> Self {
        let mut matrix = Self {
            width,
            height,
            layout,
            cells: vec![Cell::Light; usize::from(width) * usize::from(height)],
        };
        matrix.apply_function_patterns();
        matrix
    }

    /// Width in modules.
    pub const fn width(&self) -> u16 {
        self.width
    }

    /// Height in modules.
    pub const fn height(&self) -> u16 {
        self.height
    }

    /// Layout family used by this matrix.
    pub const fn layout(&self) -> LayoutFamily {
        self.layout
    }

    /// Immutable row-major cell slice.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// Read a matrix cell.
    pub fn get(&self, x: u16, y: u16) -> Result<Cell> {
        let index = self.index(x, y)?;
        Ok(self.cells[index])
    }

    /// Set a matrix cell.
    pub fn set(&mut self, x: u16, y: u16, cell: Cell) -> Result<()> {
        let index = self.index(x, y)?;
        self.cells[index] = cell;
        Ok(())
    }

    /// Apply canonical synchronization and orientation patterns.
    pub fn apply_function_patterns(&mut self) {
        for y in 0..self.height {
            for x in 0..self.width {
                if let Some(cell) =
                    layout::function_cell_for(self.layout, self.width, self.height, x, y)
                {
                    let index = self.linear_index(x, y);
                    self.cells[index] = cell;
                }
            }
        }
    }

    /// Number of payload-capable modules.
    pub fn data_capacity_bits(&self) -> usize {
        layout::data_capacity_bits_for(self.layout, self.width, self.height)
    }

    /// Fill data modules from an iterator of bits.
    pub fn write_data_bits(&mut self, bits: impl IntoIterator<Item = bool>) -> usize {
        let mut written = 0usize;
        let mut iter = bits.into_iter();
        for y in 0..self.height {
            for x in 0..self.width {
                if layout::is_data_module_for(self.layout, self.width, self.height, x, y) {
                    let bit = iter.next().unwrap_or(false);
                    let index = self.linear_index(x, y);
                    self.cells[index] = Cell::Data(bit);
                    written += 1;
                }
            }
        }
        written
    }

    /// Extract payload bits in canonical module order.
    pub fn read_data_bits(&self) -> Vec<bool> {
        let mut bits = Vec::with_capacity(self.data_capacity_bits());
        for y in 0..self.height {
            for x in 0..self.width {
                if layout::is_data_module_for(self.layout, self.width, self.height, x, y) {
                    let index = self.linear_index(x, y);
                    bits.push(self.cells[index].is_dark());
                }
            }
        }
        bits
    }

    fn index(&self, x: u16, y: u16) -> Result<usize> {
        if x >= self.width || y >= self.height {
            return Err(GlyphError::MatrixBounds {
                x,
                y,
                width: self.width,
                height: self.height,
            });
        }
        Ok(self.linear_index(x, y))
    }

    fn linear_index(&self, x: u16, y: u16) -> usize {
        usize::from(y) * usize::from(self.width) + usize::from(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_and_read_data_bits_are_stable() {
        let mut matrix = SymbolMatrix::new(45, 45);
        let input = [true, false, true, true, false];
        matrix.write_data_bits(input);
        assert_eq!(&matrix.read_data_bits()[..input.len()], input);
    }

    #[test]
    fn bounds_check_coordinates() {
        let matrix = SymbolMatrix::new(9, 9);
        assert!(matches!(
            matrix.get(9, 0),
            Err(GlyphError::MatrixBounds { .. })
        ));
    }
}
