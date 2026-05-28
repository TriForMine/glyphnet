# Plan Status

This file tracks execution status for the roadmap priorities in
[`docs/roadmap.md`](roadmap.md).

## Current Priorities

- [x] Align public strategy/docs with reliability-first baseline and execution order.
- [x] Fix repository metadata and harden CI/release policy defaults.
- [x] Modularize monolithic crates, starting with `glyphnet-scanner` (phase 1).
- [ ] Keep scanner performance gate enforceable while converging to profile budgets.
- [x] Publish a versioned fixture corpus (synthetic + real + hard negatives)
  scaffold.
- [x] Complete Phase 2 ECC: LDPC screen profile and scanner-facing telemetry contract.
- [ ] Lock matrix as scanner reliability baseline in CI (fixtures + perf/reliability rows).
- [x] Deliver burst erasure transport (fountain/RaptorQ-like direction) baseline.
- [x] Add payload authenticity envelope above transport CRC.
- [x] Complete Phase 4.5 auth trust contract (shared reason codes + validity windows).

## In Progress

- `feat/phase5-browser-sdk-phase1`: make `@glyphnet/browser` publish-ready
  (built `dist` exports, wasm init wrapper API, typed auth reason contract,
  and SDK docs alignment).

## Next Up

1. Complete Phase 5.1 browser SDK publish path and release checks.
2. Keep scanner performance gate enforceable while converging to profile
   budgets.
3. Lock matrix as scanner reliability baseline in CI:
   add matrix-first scan path checks, real matrix fixtures, and matrix-specific
   perf/reliability gate rows.
4. Implement `feat/matrix-fastpath-phase1`:
   downscaled/ROI-first matrix detection so a realistic matrix benchmark can be
   promoted to gating.
5. Improve burst high-loss reliability (30-40%) and promote burst CI thresholds
   from non-gating baseline tracking to gated targets once stable.
6. Define and implement versioned key-discovery/distribution format for SDK
   integrations (key rotation lifecycle, trust roots, and multi-key metadata).
   Baseline keyset schema + trust-policy reason contract are implemented;
   remaining work is trust-root distribution and revocation lifecycle policy.
