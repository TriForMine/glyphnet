import GlyphNetScannerModule from "../../../modules/glyphnet-scanner/src/GlyphNetScannerModule";

import { ScanJson, ScanRequest, ScannerAdapter } from "./types";

const BRIDGE_UNAVAILABLE_ERROR =
  "GlyphNet native scanner bridge unavailable on this platform/build";

export const nativeScannerAdapter: ScannerAdapter = {
  async scanStill(request: ScanRequest): Promise<ScanJson> {
    try {
      const raw = await GlyphNetScannerModule.scanStill(JSON.stringify(request));
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
    return GlyphNetScannerModule.encodeSvg(payload);
  },
};
