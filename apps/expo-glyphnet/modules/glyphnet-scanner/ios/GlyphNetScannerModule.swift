import ExpoModulesCore

public class GlyphNetScannerModule: Module {
  public func definition() -> ModuleDefinition {
    Name("GlyphNetScanner")

    AsyncFunction("scanStill") { (requestJson: String) -> String in
      return "{\"ok\":false,\"error\":\"native_scan_not_implemented_ios\",\"request_json_len\":\(requestJson.count)}"
    }

    AsyncFunction("encodeSvg") { (payload: String) -> String in
      let escaped = payload.replacingOccurrences(of: "\"", with: "\\\"")
      return "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"320\" height=\"120\"><rect width=\"320\" height=\"120\" fill=\"#101827\"/><text x=\"16\" y=\"64\" fill=\"#E5E7EB\" font-size=\"16\">GlyphNet iOS bridge pending: \(escaped)</text></svg>"
    }
  }
}
