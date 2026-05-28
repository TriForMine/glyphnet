import initWasm, * as wasm from "../pkg/glyphnet_wasm.js";

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

export interface GlyphNetDescriptor {
  version: { major: number; minor: number; patch: number };
  mode: "Print" | "Screen" | "Burst";
  ecc_level: "Low" | "Medium" | "High" | "Adaptive";
  layout:
    | "RibbonWeave"
    | "SpectralMesh"
    | "PulseStream"
    | "Constellation"
    | "FrameGrid"
    | "Matrix"
    | "Hexagonal"
    | "Radial";
  color: "Mono" | "LimitedPalette" | "Rgb" | "Adaptive";
  width: number;
  height: number;
  payload_len: number;
  stream_id: number;
  frame_index: number;
  frame_count: number;
  data_capacity_bits: number;
}

export interface AuthVerificationJson {
  verified: boolean;
  key_id: number | null;
  error: string | null;
  reason?: AuthReasonCode | null;
}

export interface GlyphNetWasmBindings {
  encodeSvg(input: string): string;
  descriptorJson(input: string): string;
  encodeSvgWithGeometry(
    input: string,
    modulePx: number,
    quietZoneModules: number,
  ): string;
  encodePngWithGeometry(
    input: string,
    modulePx: number,
    quietZoneModules: number,
  ): Uint8Array;
  scanRgbaJson(
    rgba: Uint8Array,
    width: number,
    height: number,
    mode: string,
  ): string;
  scanRgbaJsonWithVerification(
    rgba: Uint8Array,
    width: number,
    height: number,
    mode: string,
    verifyKeyHex: string,
    verifyKeyId: number,
  ): string;
}

export async function initGlyphNet(
  input?: RequestInfo | URL | Response | BufferSource | WebAssembly.Module,
): Promise<GlyphNetBrowser> {
  await initWasm(input as never);
  return new GlyphNetBrowser(wasm as unknown as GlyphNetWasmBindings);
}

export class GlyphNetBrowser {
  constructor(private readonly bindings: GlyphNetWasmBindings) {}

  encodeSvg(input: string): string {
    return this.bindings.encodeSvg(input);
  }

  encodeSvgWithGeometry(
    input: string,
    modulePx: number,
    quietZoneModules: number,
  ): string {
    return this.bindings.encodeSvgWithGeometry(input, modulePx, quietZoneModules);
  }

  descriptor(input: string): GlyphNetDescriptor {
    return JSON.parse(this.bindings.descriptorJson(input)) as GlyphNetDescriptor;
  }

  encodeElement(input: string): SVGSVGElement {
    const template = document.createElement("template");
    template.innerHTML = this.encodeSvg(input).trim();
    const node = template.content.firstElementChild;
    if (!(node instanceof SVGSVGElement)) {
      throw new Error("GlyphNet WASM returned non-SVG output");
    }
    return node;
  }

  encodePng(input: string, modulePx = 4, quietZoneModules = 4): Uint8Array {
    return this.bindings.encodePngWithGeometry(input, modulePx, quietZoneModules);
  }

  scanRgba(
    rgba: Uint8Array,
    width: number,
    height: number,
    mode: ScanMode = "print",
  ): unknown {
    return JSON.parse(this.bindings.scanRgbaJson(rgba, width, height, mode));
  }

  scanRgbaWithVerification(
    rgba: Uint8Array,
    width: number,
    height: number,
    mode: ScanMode,
    verifyKeyHex: string,
    verifyKeyId: number,
  ): { ok: boolean; auth: AuthVerificationJson } & Record<string, unknown> {
    return JSON.parse(
      this.bindings.scanRgbaJsonWithVerification(
        rgba,
        width,
        height,
        mode,
        verifyKeyHex,
        verifyKeyId,
      ),
    ) as { ok: boolean; auth: AuthVerificationJson } & Record<string, unknown>;
  }
}
