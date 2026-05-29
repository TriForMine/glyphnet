export type ScanMode = "print" | "screen" | "burst";

export type AuthReasonCode =
  | "unknown_key_id"
  | "key_not_yet_valid"
  | "key_expired"
  | "missing_verification_key"
  | "auth_mismatch"
  | "invalid_envelope"
  | "verify_failed"
  | "unsigned_payload";

export interface AuthVerificationJson {
  verified: boolean;
  key_id: number | null;
  error: string | null;
  reason?: AuthReasonCode | null;
}

export interface ScanJson {
  ok: boolean;
  payload_utf8_lossy?: string;
  payload_len?: number;
  error?: string;
  auth?: AuthVerificationJson;
}

export interface ScanRequest {
  mode: ScanMode;
  verifyKeyHex?: string;
  verifyKeyId?: number;
  imageBase64?: string;
  roiX?: number;
  roiY?: number;
  roiW?: number;
  roiH?: number;
  width?: number;
  height?: number;
  rgbaBase64?: string;
}

export interface ScannerAdapter {
  scanStill(request: ScanRequest): Promise<ScanJson>;
  encodeSvg(payload: string): Promise<string>;
}
