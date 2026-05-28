use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use glyphnet_core::{EccLevel, Frame, TransmissionMode};
use glyphnet_ecc::{
    encode_for_mode, try_recover_for_mode_with_suspects_and_telemetry, verify_for_mode,
};

fn sample_wire(len: usize) -> Vec<u8> {
    let payload = vec![0x5A; len];
    Frame::new(TransmissionMode::Screen, EccLevel::High, 0, 1, 7, payload)
        .expect("frame")
        .encode()
}

fn bench_screen_encode_verify(c: &mut Criterion) {
    let mut group = c.benchmark_group("screen_encode_verify");
    for payload_len in [64usize, 256, 768] {
        let wire = sample_wire(payload_len);
        group.throughput(Throughput::Bytes(wire.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("encode", payload_len),
            &wire,
            |b, input| {
                b.iter(|| {
                    let _ = encode_for_mode(TransmissionMode::Screen, EccLevel::High, input);
                })
            },
        );
        let encoded = encode_for_mode(TransmissionMode::Screen, EccLevel::High, &wire);
        group.bench_with_input(
            BenchmarkId::new("verify", payload_len),
            &(encoded, wire.len()),
            |b, (input, data_len)| {
                b.iter(|| {
                    let _ =
                        verify_for_mode(TransmissionMode::Screen, EccLevel::High, input, *data_len);
                })
            },
        );
    }
    group.finish();
}

fn bench_screen_recover(c: &mut Criterion) {
    let mut group = c.benchmark_group("screen_recover");
    for payload_len in [64usize, 256] {
        let wire = sample_wire(payload_len);
        let mut encoded = encode_for_mode(TransmissionMode::Screen, EccLevel::High, &wire);
        let corrupt_index = (wire.len() / 2).min(encoded.len().saturating_sub(1));
        encoded[corrupt_index] ^= 0x11;
        group.bench_with_input(
            BenchmarkId::new("recover_single_byte", payload_len),
            &(encoded, wire.len(), corrupt_index),
            |b, (input, data_len, idx)| {
                b.iter(|| {
                    let _ = try_recover_for_mode_with_suspects_and_telemetry(
                        TransmissionMode::Screen,
                        EccLevel::High,
                        input,
                        *data_len,
                        &[*idx],
                        1024,
                    );
                })
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_screen_encode_verify, bench_screen_recover);
criterion_main!(benches);
