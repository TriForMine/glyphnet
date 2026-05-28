import { registerWebModule, NativeModule } from 'expo';

// GlyphNetScannerModule is not available on the web platform.
class GlyphNetScannerModule extends NativeModule<{}> {
  scanStill(_requestJson: string): Promise<string> {
    return Promise.resolve('{"ok":false,"error":"native_scan_not_available_on_web"}');
  }

  encodeSvg(payload: string): Promise<string> {
    return Promise.resolve(
      `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="120"><rect width="320" height="120" fill="#101827"/><text x="16" y="64" fill="#E5E7EB" font-size="16">GlyphNet web fallback: ${payload}</text></svg>`
    );
  }
}

export default registerWebModule(GlyphNetScannerModule, 'GlyphNetScannerModule');
