use criterion::{Criterion, criterion_group, criterion_main};
use glyphnet_core::{LayoutFamily, ProfileId, TransmissionMode, profile_spec};
use glyphnet_encode::{Encoder, EncoderConfig};
use glyphnet_render::{RasterRenderer, RenderOptions};
use glyphnet_scanner::scan_still;
use image::{DynamicImage, Rgba, RgbaImage};

fn real_debugger_screenshot(c: &mut Criterion) {
    let image =
        image::load_from_memory(include_bytes!("../fixtures/screenshot-debug-sample.png")).unwrap();

    let target = profile_spec(ProfileId::RibbonPrint).benchmark.max_decode_ms;

    c.bench_function("scan_real_debugger_screenshot", |b| {
        b.iter(|| {
            let result = scan_still(&image, TransmissionMode::Print).unwrap();
            // Keep benchmark naming tied to profile-level latency objective.
            criterion::black_box(target);
            assert_eq!(result.decoded.decoded.frame.payload, b"debug sample");
        });
    });
}

fn generated_matrix_canvas(c: &mut Criterion) {
    let encoded = Encoder::new(EncoderConfig {
        layout: LayoutFamily::Matrix,
        ..EncoderConfig::default()
    })
    .encode_static(b"matrix baseline")
    .unwrap();
    let symbol = RasterRenderer::new(RenderOptions {
        module_px: 4,
        quiet_zone_modules: 4,
        ..RenderOptions::default()
    })
    .render(&encoded.matrix)
    .unwrap();
    let mut canvas = RgbaImage::from_pixel(960, 360, Rgba([255, 255, 255, 255]));
    image::imageops::overlay(&mut canvas, &symbol, 128, 56);
    let image = DynamicImage::ImageRgba8(canvas);

    c.bench_function("scan_generated_matrix_canvas", |b| {
        b.iter(|| {
            let result = scan_still(&image, TransmissionMode::Print).unwrap();
            assert_eq!(result.decoded.decoded.frame.payload, b"matrix baseline");
            assert_eq!(result.decoded.info.layout, LayoutFamily::Matrix);
        });
    });
}

criterion_group!(benches, real_debugger_screenshot, generated_matrix_canvas);
criterion_main!(benches);
