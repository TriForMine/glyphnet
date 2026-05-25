use criterion::{Criterion, criterion_group, criterion_main};
use glyphnet_core::TransmissionMode;
use glyphnet_scanner::scan_still;

fn real_debugger_screenshot(c: &mut Criterion) {
    let image =
        image::load_from_memory(include_bytes!("../fixtures/screenshot-debug-sample.png")).unwrap();

    c.bench_function("scan_real_debugger_screenshot", |b| {
        b.iter(|| {
            let result = scan_still(&image, TransmissionMode::Print).unwrap();
            assert_eq!(result.decoded.decoded.frame.payload, b"debug sample");
        });
    });
}

criterion_group!(benches, real_debugger_screenshot);
criterion_main!(benches);
