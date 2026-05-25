# GlyphNet Scan Debugger

Build the WASM package, then serve the repository root and open the debugger:

```powershell
wasm-pack build crates/glyphnet-wasm --target web --out-dir ../../sdk/browser/pkg
python -m http.server 8765 --bind 127.0.0.1
```

Then open:

```text
http://127.0.0.1:8765/demos/debug/index.html
```

The page uses the real Rust `glyphnet-scanner` pipeline through
`glyphnet-wasm`. It does not reimplement scanner heuristics in JavaScript.

Current diagnostics:

- Rust scan result, payload, decode error, inferred layout, and module size;
- Rust crop, quad, and warp diagnostics;
- Rust input selection: full image, JS crop, or manual drag crop;
- source image dimensions and displayed canvas bounds;
- Rust candidate crop attempts;
- Rust-estimated quadrilateral overlay;
- JSON export of the active diagnostics.

Drag on the image to set a manual crop. The Rust scanner will receive the
corresponding source-image pixels, which is useful when testing screenshots that
include browser chrome, UI panels, or multiple nested captures.
