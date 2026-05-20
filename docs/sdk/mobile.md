# Mobile SDK

Mobile support should reuse Rust crates wherever possible and keep camera/UI
code thin on each platform.

Planned Android path:

- Rust static library or UniFFI component.
- Kotlin wrapper for encode/decode/scanner APIs.
- CameraX frame source.
- JNI only where UniFFI is insufficient.

Planned iOS path:

- Rust static library.
- Swift Package wrapper.
- AVFoundation frame source.
- Metal acceleration hooks for future sampling kernels.

The mobile SDK must expose battery-aware scanning, camera permission handling,
offline operation, telemetry, and strict bounds on memory use.
