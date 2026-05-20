# Desktop Demo Scaffold

The desktop demo should become a camera scanner application using
`glyphnet-scanner`.

Candidate stacks:

- `eframe` or `tauri` for UI.
- `nokhwa`, `opencv`, or platform camera APIs for frame capture.
- `wgpu` for future GPU thresholding and rectification.

The first production milestone is a deterministic file-based demo:

```powershell
cargo run -p glyphnet-cli -- encode --data "desktop demo" --output demo.png
cargo run -p glyphnet-cli -- decode demo.png
```
