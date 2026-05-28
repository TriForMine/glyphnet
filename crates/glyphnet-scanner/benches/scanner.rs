use criterion::{Criterion, criterion_group, criterion_main};
use glyphnet_core::{LayoutFamily, ProfileId, TransmissionMode, profile_spec};
use glyphnet_encode::{Encoder, EncoderConfig};
use glyphnet_render::{RasterRenderer, RenderOptions};
use glyphnet_scanner::scan_still;
use image::{DynamicImage, Rgba, RgbaImage};

fn rendered_ribbon(payload: &[u8], module_px: u32) -> RgbaImage {
    let encoded = Encoder::new(EncoderConfig {
        layout: LayoutFamily::RibbonWeave,
        ..EncoderConfig::default()
    })
    .encode_static(payload)
    .unwrap();
    RasterRenderer::new(RenderOptions {
        module_px,
        quiet_zone_modules: 4,
        ..RenderOptions::default()
    })
    .render(&encoded.matrix)
    .unwrap()
}

fn ribbon_roi(payload: &[u8], module_px: u32, border_px: u32) -> DynamicImage {
    let symbol = rendered_ribbon(payload, module_px);
    let canvas_width = symbol.width() + border_px * 2;
    let canvas_height = symbol.height() + border_px * 2;
    let mut canvas = RgbaImage::from_pixel(canvas_width, canvas_height, Rgba([255, 255, 255, 255]));
    image::imageops::overlay(
        &mut canvas,
        &symbol,
        i64::from(border_px),
        i64::from(border_px),
    );
    DynamicImage::ImageRgba8(canvas)
}

fn real_debugger_screenshot(c: &mut Criterion) {
    let image =
        image::load_from_memory(include_bytes!("../fixtures/screenshot-debug-sample.png")).unwrap();

    let target = profile_spec(ProfileId::RibbonPrint).benchmark.max_decode_ms;

    c.bench_function("scan_real_debugger_screenshot", |b| {
        // Keep benchmark naming tied to profile-level latency objective.
        let target = criterion::black_box(target);
        let _ = target;

        b.iter(|| {
            let result = scan_still(&image, TransmissionMode::Print).unwrap();
            assert_eq!(result.decoded.decoded.frame.payload, b"debug sample");
        });
    });
}

fn generated_ribbon_canvas_small(c: &mut Criterion) {
    let payload = b"ribbon small";
    let image = ribbon_roi(payload, 4, 12);
    c.bench_function("scan_generated_ribbon_canvas_small", |b| {
        b.iter(|| {
            let result = scan_still(&image, TransmissionMode::Print).unwrap();
            assert_eq!(result.decoded.decoded.frame.payload, payload);
            assert_eq!(result.decoded.info.layout, LayoutFamily::RibbonWeave);
        });
    });
}

fn generated_ribbon_canvas_medium(c: &mut Criterion) {
    let payload = b"ribbon medium";
    let image = ribbon_roi(payload, 4, 24);
    c.bench_function("scan_generated_ribbon_canvas_medium", |b| {
        b.iter(|| {
            let result = scan_still(&image, TransmissionMode::Print).unwrap();
            assert_eq!(result.decoded.decoded.frame.payload, payload);
            assert_eq!(result.decoded.info.layout, LayoutFamily::RibbonWeave);
        });
    });
}

fn generated_ribbon_canvas_large(c: &mut Criterion) {
    let payload = b"ribbon large";
    let image = ribbon_roi(payload, 4, 48);
    c.bench_function("scan_generated_ribbon_canvas_large", |b| {
        b.iter(|| {
            let result = scan_still(&image, TransmissionMode::Print).unwrap();
            assert_eq!(result.decoded.decoded.frame.payload, payload);
            assert_eq!(result.decoded.info.layout, LayoutFamily::RibbonWeave);
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

fn generated_matrix_roi(c: &mut Criterion) {
    let payload = b"matrix roi";
    let encoded = Encoder::new(EncoderConfig {
        layout: LayoutFamily::Matrix,
        ..EncoderConfig::default()
    })
    .encode_static(payload)
    .unwrap();
    let symbol = RasterRenderer::new(RenderOptions {
        module_px: 4,
        quiet_zone_modules: 4,
        ..RenderOptions::default()
    })
    .render(&encoded.matrix)
    .unwrap();
    let border = 16u32;
    let mut canvas = RgbaImage::from_pixel(
        symbol.width() + border * 2,
        symbol.height() + border * 2,
        Rgba([255, 255, 255, 255]),
    );
    image::imageops::overlay(&mut canvas, &symbol, i64::from(border), i64::from(border));
    let image = DynamicImage::ImageRgba8(canvas);

    c.bench_function("scan_generated_matrix_roi", |b| {
        b.iter(|| {
            let result = scan_still(&image, TransmissionMode::Print).unwrap();
            assert_eq!(result.decoded.decoded.frame.payload, payload);
            assert_eq!(result.decoded.info.layout, LayoutFamily::Matrix);
        });
    });
}

criterion_group!(
    benches,
    real_debugger_screenshot,
    generated_ribbon_canvas_small,
    generated_ribbon_canvas_medium,
    generated_ribbon_canvas_large,
    generated_matrix_canvas,
    generated_matrix_roi
);
criterion_main!(benches);
