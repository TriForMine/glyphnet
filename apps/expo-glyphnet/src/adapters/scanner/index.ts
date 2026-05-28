import { mockScannerAdapter } from "./mockScannerAdapter";
import { ScannerAdapter } from "./types";

// Phase 5.2+: replace with native Android bridge (Expo module/JNI into Rust).
export const scannerAdapter: ScannerAdapter = mockScannerAdapter;

export * from "./types";
