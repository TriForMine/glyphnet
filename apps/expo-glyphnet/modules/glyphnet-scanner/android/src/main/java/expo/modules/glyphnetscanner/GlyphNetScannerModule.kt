package expo.modules.glyphnetscanner

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition

class GlyphNetScannerModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("GlyphNetScanner")

    AsyncFunction("scanStill") { requestJson: String ->
      GlyphNetNativeBridge.scanStill(requestJson)
    }

    AsyncFunction("encodeSvg") { payload: String ->
      GlyphNetNativeBridge.encodeSvg(payload)
    }
  }
}
