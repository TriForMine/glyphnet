import { ScannerAdapter } from "./types";

import { mockScannerAdapter } from "./mockScannerAdapter";
import { isNativeScannerAvailable, nativeScannerAdapter } from "./nativeScannerAdapter";

export const scannerAdapter: ScannerAdapter = isNativeScannerAvailable()
  ? nativeScannerAdapter
  : mockScannerAdapter;

export * from "./types";
