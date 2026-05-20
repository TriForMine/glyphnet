# Roadmap

## Phase 0: Reference Protocol

- Deterministic frame wire format.
- Non-square `RibbonWeave` layout with continuous strokes, side totems, chevron
  rails, and a center phase trace.
- Named profiles for print, color screen, animated burst, experimental
  constellation, and matrix baseline use cases.
- Static encode-render-decode path.
- Burst frame construction and assembly.
- CLI, WASM bridge, SDK scaffolds, CI, fuzzing, benchmarks, docs.

## Phase 1: Robust Static Scanning

- Totem and chevron detection with perspective rectification.
- Signature rail validation before expensive payload decode.
- Adaptive threshold tuned against print degradation fixtures.
- Module pitch estimation from timing markers.
- Blur, exposure, and perspective synthetic test suite.
- Snapshot conformance vectors.

## Phase 2: Strong ECC

- Reed-Solomon profile for print mode.
- LDPC profile for high-density screen mode.
- Interleaving policies selected by mode.
- Erasure telemetry surfaced to scanner clients.

## Phase 3: Screen Density

- Harden `SpectralScreen` color palettes and RGB modulation.
- Display gamma and camera white-balance calibration.
- Micro-pattern experiments behind feature flags.
- Browser camera demo using WebAssembly.

## Phase 4: Burst Transport

- Temporal synchronization preamble.
- Fountain packet schedule and RaptorQ-like recovery.
- Adaptive bitrate controller.
- Motion blur and rolling-shutter mitigation.
- Video export and WebRTC sender/receiver demos.

## Phase 5: Platform SDKs

- TypeScript browser SDK published from `glyphnet-wasm`.
- Swift Package wrapper for iOS.
- Kotlin/Android wrapper around Rust core through UniFFI or JNI.
- Desktop camera scanner backends.
- Industrial camera integration examples.

## Phase 6: Advanced Layouts

- Hexagonal screen layout with staggered sampling.
- Radial layout for large posters and lens-distorted surfaces.
- AR/VR marker tracking profile.
- GPU accelerated rectification and sampling.
