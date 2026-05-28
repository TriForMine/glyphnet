import { ScanJson, ScanRequest, ScannerAdapter } from "./types";

const MOCK_PAYLOAD = "glyphnet-mobile-mock";

export const mockScannerAdapter: ScannerAdapter = {
  async scanStill(request: ScanRequest): Promise<ScanJson> {
    const base: ScanJson = {
      ok: true,
      payload_utf8_lossy: MOCK_PAYLOAD,
      payload_len: MOCK_PAYLOAD.length,
    };
    if (!request.verifyKeyHex) {
      return {
        ...base,
        auth: {
          verified: false,
          key_id: null,
          error: "authenticated payload detected but no verification key was provided",
          reason: "missing_verification_key",
        },
      };
    }
    return {
      ...base,
      auth: {
        verified: true,
        key_id: request.verifyKeyId ?? 1,
        error: null,
        reason: null,
      },
    };
  },

  async encodeSvg(payload: string): Promise<string> {
    return `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="120"><rect width="320" height="120" fill="#f4f7fb"/><text x="16" y="64" fill="#112035" font-size="18">MOCK:${payload}</text></svg>`;
  },
};
