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

Build target:

```powershell
wasm-pack build crates/glyphnet-wasm --target web --out-dir ../../sdk/browser/pkg
```
