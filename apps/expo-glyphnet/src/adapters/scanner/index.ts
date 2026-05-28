import { ScannerAdapter } from "./types";

import { nativeScannerAdapter } from "./nativeScannerAdapter";

export const scannerAdapter: ScannerAdapter = nativeScannerAdapter;

export * from "./types";
