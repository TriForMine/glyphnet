export interface GlyphNetDescriptor {
  version: { major: number; minor: number; patch: number };
  mode: "Print" | "Screen" | "Burst";
  ecc_level: "Low" | "Medium" | "High" | "Adaptive";
  layout: "RibbonWeave" | "Constellation" | "FrameGrid" | "Matrix" | "Hexagonal" | "Radial";
  color: "Mono" | "LimitedPalette" | "Rgb" | "Adaptive";
  width: number;
  height: number;
  payload_len: number;
  stream_id: number;
  frame_index: number;
  frame_count: number;
  data_capacity_bits: number;
}

export interface GlyphNetWasm {
  encodeSvg(input: string): string;
  descriptorJson(input: string): string;
  encodeSvgWithGeometry(input: string, modulePx: number, quietZoneModules: number): string;
  encodePngWithGeometry(input: string, modulePx: number, quietZoneModules: number): Uint8Array;
  scanRgbaJson(rgba: Uint8Array, width: number, height: number, mode: string): string;
}

export class GlyphNetBrowser {
  constructor(private readonly wasm: GlyphNetWasm) {}

  encodeSvg(input: string): string {
    return this.wasm.encodeSvg(input);
  }

  descriptor(input: string): GlyphNetDescriptor {
    return JSON.parse(this.wasm.descriptorJson(input)) as GlyphNetDescriptor;
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
    return this.wasm.encodePngWithGeometry(input, modulePx, quietZoneModules);
  }

  scanRgba(rgba: Uint8Array, width: number, height: number, mode = "print"): unknown {
    return JSON.parse(this.wasm.scanRgbaJson(rgba, width, height, mode));
  }
}
