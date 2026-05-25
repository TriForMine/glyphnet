# Browser SDK

The browser SDK is expected to wrap `glyphnet-wasm`.

Planned capabilities:

- SVG and PNG generation from JavaScript or TypeScript.
- Camera scanning through `MediaDevices.getUserMedia`.
- Burst-mode display and receive loops synchronized with `requestAnimationFrame`.
- Web Worker offload for decode and CV work.
- Optional WebGPU acceleration for thresholding and rectification.

Current scaffold:

- `sdk/browser/package.json`
- `sdk/browser/src/index.ts`
- `demos/browser/index.html`
- `demos/debug/index.html`

Debug workbench:

Build `glyphnet-wasm`, serve the repository root, then open
`/demos/debug/index.html` to import PNG/JPEG images and inspect the real Rust
scanner output, crop attempts, estimated quads, and JSON diagnostics.

```powershell
wasm-pack build crates/glyphnet-wasm --target web --out-dir ../../sdk/browser/pkg
python -m http.server 8765 --bind 127.0.0.1
```

Build target:

```powershell
wasm-pack build crates/glyphnet-wasm --target web --out-dir ../../sdk/browser/pkg
```
