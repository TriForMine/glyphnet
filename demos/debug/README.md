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
- Rust input selection: full image or manual drag crop;
- source image dimensions and displayed canvas bounds;
- Rust candidate crop attempts;
- Rust stage timings and per-attempt decode durations;
- Rust-estimated quadrilateral overlay;
- JSON export of the active diagnostics.

The built-in sample is generated as PNG bytes by Rust/WASM
`encodePngWithGeometry`, then scanned by the same Rust `scanRgbaJson` path used
for imported images. This avoids browser SVG rasterization differences hiding
scanner regressions.

Scanner behavior to inspect here:

- fast RibbonWeave signature localization from side totems and chevron rails;
- fractional-grid sampling for screenshot/camera resampling where modules are
  not an integer number of pixels;
- phase and scale search with a header precheck before full ECC validation;
- large-image fast failure that avoids expensive generic crop crawling.

Drag on the image to set a manual crop. The Rust scanner will receive the
corresponding source-image pixels, which is useful when testing screenshots that
include browser chrome, UI panels, or multiple nested captures.
