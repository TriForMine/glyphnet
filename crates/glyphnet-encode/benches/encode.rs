use criterion::{Criterion, criterion_group, criterion_main};
use glyphnet_core::ProfileId;
use glyphnet_encode::{Encoder, EncoderConfig};

fn encode_profiles(c: &mut Criterion) {
    let cases = [
        (ProfileId::RibbonPrint, "ribbon_print_256b", 256usize),
        (ProfileId::SpectralScreen, "spectral_screen_1kb", 1024usize),
        (ProfileId::PulseBurst, "pulse_burst_64kb", 64 * 1024usize),
    ];

    for (profile, name, payload_len) in cases {
        let encoder = Encoder::new(EncoderConfig::for_profile(profile));
        let payload = vec![0x5au8; payload_len];
        c.bench_function(name, |b| {
            b.iter(|| {
                if matches!(profile, ProfileId::PulseBurst) {
                    encoder.encode_burst(&payload).unwrap();
                } else {
                    encoder.encode_static(&payload).unwrap();
                }
            });
        });
    }
}

criterion_group!(benches, encode_profiles);
criterion_main!(benches);
