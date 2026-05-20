#![no_main]

use glyphnet_core::{Cell, SymbolMatrix};
use glyphnet_decode::decode_matrix;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }
    let dimension = 21 + u16::from(data[0] % 16) * 4;
    let mut matrix = SymbolMatrix::new(dimension, dimension);
    let bits = data[1..].iter().flat_map(|byte| {
        (0..8)
            .rev()
            .map(move |shift| Cell::Data((byte >> shift) & 1 == 1))
    });
    let mut iter = bits;
    for y in 0..matrix.height() {
        for x in 0..matrix.width() {
            if glyphnet_core::layout::is_data_module_for(
                matrix.layout(),
                matrix.width(),
                matrix.height(),
                x,
                y,
            ) {
                if let Some(cell) = iter.next() {
                    let _ = matrix.set(x, y, cell);
                }
            }
        }
    }
    let _ = decode_matrix(&matrix);
});
