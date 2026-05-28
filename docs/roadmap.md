# Roadmap

The product strategy and benchmark contract are defined in
[`docs/strategy.md`](strategy.md). This roadmap is the implementation sequence
for that strategy.

## Execution Priorities (Current)

- Keep `Matrix` as the public reliability-first baseline until `RibbonWeave`
  consistently meets equivalent robustness targets on real fixtures.
- Prioritize scanner/decode/cv modularization before adding new layouts.
- Keep scanner performance gates enforceable and documented against real
  profile budgets.
- Publish a versioned fixture corpus (synthetic + real + hard negatives) before
  making broad reliability claims.
- Treat burst erasure transport as the main differentiation path for large
  payload transfer.

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

- Totem and chevron detection for screenshot-style still images.
- Signature rail validation before expensive payload decode.
- Adaptive threshold tuned against print degradation fixtures.
- Module pitch estimation from timing markers and detected signature geometry.
- Fractional-grid phase/scale search for non-integer screenshot and camera
  sampling.
- Blur, exposure, and perspective synthetic test suite.
- Snapshot conformance vectors.
- Reliability-first matrix detector path with explicit profile routing.
- Published PR-vs-base scanner reliability/performance reports on fixture sets.

## Phase 2: Strong ECC

- Reed-Solomon profile for print mode.
- LDPC profile for high-density screen mode.
- Interleaving policies selected by mode.
- Erasure telemetry surfaced to scanner clients.
- Scanner-consumable confidence/erasure telemetry contract for SDK clients.

## Phase 2.5: Matrix Baseline Lock

- Keep `Matrix` as the default reliability baseline in scanner routing and
  acceptance tests.
- Expand real matrix fixture coverage (clean, clutter, perspective).
- Add matrix-specific scanner perf/reliability CI reporting alongside ribbon
  fixtures.

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

## Phase 4.5: Trust and Authenticity

- Add optional payload authenticity envelope (detached/embedded signatures).
- Define verification UX for CLI/WASM/SDK consumers.
- Keep transport CRC for corruption detection, separate from authenticity.

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
- Only promote additional layouts when they beat baseline metrics for declared
  target scenarios.
