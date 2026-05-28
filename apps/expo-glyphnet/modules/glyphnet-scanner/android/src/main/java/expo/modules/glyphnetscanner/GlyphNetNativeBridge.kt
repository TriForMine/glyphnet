package expo.modules.glyphnetscanner

internal object GlyphNetNativeBridge {
  private const val LIB_NAME = "glyphnet_scanner_bridge"

  private val nativeReady: Boolean by lazy {
    try {
      System.loadLibrary(LIB_NAME)
      true
    } catch (_: UnsatisfiedLinkError) {
      false
    } catch (_: SecurityException) {
      false
    }
  }

  fun scanStill(requestJson: String): String {
    if (!nativeReady) {
      return """{"ok":false,"error":"native_bridge_library_not_loaded","request_json_len":${requestJson.length}}"""
    }
    return scanStillNative(requestJson)
  }

  fun encodeSvg(payload: String): String {
    if (!nativeReady) {
      val escaped = payload.replace("\"", "\\\"")
      return """<svg xmlns="http://www.w3.org/2000/svg" width="320" height="120"><rect width="320" height="120" fill="#101827"/><text x="16" y="64" fill="#E5E7EB" font-size="16">GlyphNet Android JNI pending: ${escaped}</text></svg>"""
    }
    return encodeSvgNative(payload)
  }

  private external fun scanStillNative(requestJson: String): String
  private external fun encodeSvgNative(payload: String): String
}

