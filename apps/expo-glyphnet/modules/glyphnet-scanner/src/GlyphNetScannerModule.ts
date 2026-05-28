import { NativeModule, requireNativeModule } from 'expo';

declare class GlyphNetScannerModule extends NativeModule<{}> {
  scanStill(requestJson: string): Promise<string>;
  encodeSvg(payload: string): Promise<string>;
}

export default requireNativeModule<GlyphNetScannerModule>('GlyphNetScanner');
