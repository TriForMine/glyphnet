import { ScanJson, ScanRequest, ScannerAdapter } from "./types";

const BRIDGE_UNAVAILABLE_ERROR =
  "GlyphNet native scanner bridge unavailable on this platform/build";

type NativeModule = {
  scanStill: (requestJson: string) => Promise<string>;
  encodeSvg: (payload: string) => Promise<string>;
};

function getNativeModule(): NativeModule | null {
  try {
    // Lazy require so app can boot even when module is not linked in this build.
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const mod = require("../../../modules/glyphnet-scanner/src/GlyphNetScannerModule");
    return (mod?.default ?? mod) as NativeModule;
  } catch {
    return null;
  }
}

export function isNativeScannerAvailable(): boolean {
  return getNativeModule() !== null;
}

export const nativeScannerAdapter: ScannerAdapter = {
  async scanStill(request: ScanRequest): Promise<ScanJson> {
    const nativeModule = getNativeModule();
    if (!nativeModule) {
      return {
        ok: false,
        error: BRIDGE_UNAVAILABLE_ERROR,
      };
    }
    try {
      const raw = await nativeModule.scanStill(JSON.stringify(request));
      return JSON.parse(raw) as ScanJson;
    } catch (error) {
      return {
        ok: false,
        error:
          error instanceof Error
            ? `native_scan_failed: ${error.message}`
            : BRIDGE_UNAVAILABLE_ERROR,
      };
    }
  },

  async encodeSvg(payload: string): Promise<string> {
    const nativeModule = getNativeModule();
    if (!nativeModule) {
      return JSON.stringify({
        ok: false,
        error: BRIDGE_UNAVAILABLE_ERROR,
      });
    }
    return nativeModule.encodeSvg(payload);
  },
};
