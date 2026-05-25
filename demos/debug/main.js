const state = {
  source: null,
  imageName: null,
  diagnostics: null,
  wasm: null,
  wasmError: null,
  manualCrop: null,
  activeDrag: null,
};

const nodes = {
  fileInput: document.querySelector("#fileInput"),
  modeSelect: document.querySelector("#modeSelect"),
  overlaySelect: document.querySelector("#overlaySelect"),
  scanTarget: document.querySelector("#scanTarget"),
  loadSample: document.querySelector("#loadSample"),
  clearCrop: document.querySelector("#clearCrop"),
  dropZone: document.querySelector("#dropZone"),
  emptyState: document.querySelector("#emptyState"),
  imageCanvas: document.querySelector("#imageCanvas"),
  overlayCanvas: document.querySelector("#overlayCanvas"),
  thresholdCanvas: document.querySelector("#thresholdCanvas"),
  jsonOutput: document.querySelector("#jsonOutput"),
  wasmStatus: document.querySelector("#wasmStatus"),
  tabs: document.querySelectorAll(".tabs button"),
  views: document.querySelectorAll(".view"),
  metricImage: document.querySelector("#metricImage"),
  metricThreshold: document.querySelector("#metricThreshold"),
  metricRust: document.querySelector("#metricRust"),
  metricRustInput: document.querySelector("#metricRustInput"),
  metricRustCrop: document.querySelector("#metricRustCrop"),
  metricCrop: document.querySelector("#metricCrop"),
  metricComponents: document.querySelector("#metricComponents"),
  metricAnchors: document.querySelector("#metricAnchors"),
  metricQuad: document.querySelector("#metricQuad"),
};

loadWasm();

nodes.fileInput.addEventListener("change", () => {
  const file = nodes.fileInput.files?.[0];
  if (file) loadFile(file);
});

for (const node of [nodes.modeSelect, nodes.overlaySelect, nodes.scanTarget]) {
  node.addEventListener("change", () => analyzeCurrent());
}

nodes.loadSample.addEventListener("click", async () => {
  const canvas = await sampleCanvas();
  const image = new Image();
  image.onload = () => {
    state.source = image;
    state.imageName = "rust-rendered-sample";
    analyzeCurrent();
  };
  image.src = canvas.toDataURL("image/png");
});

nodes.clearCrop.addEventListener("click", () => {
  state.manualCrop = null;
  if (nodes.scanTarget.value === "manual") nodes.scanTarget.value = "full";
  analyzeCurrent();
});

nodes.dropZone.addEventListener("pointerdown", (event) => {
  if (!state.source || !state.diagnostics?.source?.displayed) return;
  const point = canvasPoint(event);
  if (!pointInsideRect(point, state.diagnostics.source.displayed)) return;
  nodes.dropZone.setPointerCapture(event.pointerId);
  nodes.dropZone.classList.add("selecting");
  state.activeDrag = { start: point, current: point };
});

nodes.dropZone.addEventListener("pointermove", (event) => {
  if (!state.activeDrag) return;
  state.activeDrag.current = canvasPoint(event);
  drawOverlay();
});

nodes.dropZone.addEventListener("pointerup", (event) => {
  if (!state.activeDrag) return;
  state.activeDrag.current = canvasPoint(event);
  const crop = normalizeRect(state.activeDrag.start, state.activeDrag.current);
  state.activeDrag = null;
  nodes.dropZone.classList.remove("selecting");
  if (crop.width >= 8 && crop.height >= 8) {
    state.manualCrop = clampRect(crop, state.diagnostics.source.displayed);
    nodes.scanTarget.value = "manual";
  }
  analyzeCurrent();
});

nodes.dropZone.addEventListener("dragover", (event) => {
  event.preventDefault();
  nodes.dropZone.classList.add("dragging");
});

nodes.dropZone.addEventListener("dragleave", () => {
  nodes.dropZone.classList.remove("dragging");
});

nodes.dropZone.addEventListener("drop", (event) => {
  event.preventDefault();
  nodes.dropZone.classList.remove("dragging");
  const file = event.dataTransfer?.files?.[0];
  if (file) loadFile(file);
});

for (const tab of nodes.tabs) {
  tab.addEventListener("click", () => {
    for (const other of nodes.tabs) other.classList.toggle("active", other === tab);
    for (const view of nodes.views) view.classList.toggle("active", view.id === `${tab.dataset.view}View`);
  });
}

window.addEventListener("resize", () => analyzeCurrent());

async function loadWasm() {
  try {
    const wasm = await import("../../sdk/browser/pkg/glyphnet_wasm.js");
    await wasm.default();
    state.wasm = wasm;
    nodes.wasmStatus.textContent = "Rust scanner ready";
    nodes.wasmStatus.classList.add("ready");
    nodes.wasmStatus.classList.remove("missing");
    analyzeCurrent();
  } catch (error) {
    state.wasmError = error;
    nodes.wasmStatus.textContent = "Build WASM first";
    nodes.wasmStatus.classList.add("missing");
    nodes.wasmStatus.title =
      "Run: wasm-pack build crates/glyphnet-wasm --target web --out-dir ../../sdk/browser/pkg";
  }
}

function loadFile(file) {
  const url = URL.createObjectURL(file);
  const image = new Image();
  image.onload = () => {
    URL.revokeObjectURL(url);
    state.source = image;
    state.imageName = file.name;
    state.manualCrop = null;
    if (nodes.scanTarget.value === "manual") nodes.scanTarget.value = "full";
    analyzeCurrent();
  };
  image.src = url;
}

async function analyzeCurrent() {
  if (!state.source) return;
  nodes.emptyState.style.display = "none";

  const rect = nodes.dropZone.getBoundingClientRect();
  const displayed = drawSource(rect.width, rect.height);
  const rust = await scanWithRust();
  state.diagnostics = {
    source: {
      name: state.imageName,
      width: state.source.width,
      height: state.source.height,
      displayed,
    },
    rust,
  };

  drawThresholdPlaceholder();
  drawOverlay();
  updateMetrics();
}

function drawSource(containerWidth, containerHeight) {
  const canvas = nodes.imageCanvas;
  const overlayCanvas = nodes.overlayCanvas;
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.round(containerWidth * dpr));
  canvas.height = Math.max(1, Math.round(containerHeight * dpr));
  overlayCanvas.width = canvas.width;
  overlayCanvas.height = canvas.height;

  const scale = Math.min(canvas.width / state.source.width, canvas.height / state.source.height);
  const displayed = {
    x: (canvas.width - state.source.width * scale) / 2,
    y: (canvas.height - state.source.height * scale) / 2,
    width: state.source.width * scale,
    height: state.source.height * scale,
  };

  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(state.source, displayed.x, displayed.y, displayed.width, displayed.height);
  return displayed;
}

async function scanWithRust() {
  if (!state.wasm) {
    return {
      ok: false,
      unavailable: true,
      error: state.wasmError
        ? `WASM package unavailable: ${state.wasmError.message}`
        : "WASM package is still loading",
    };
  }
  try {
    const input = rustInputImageData();
    const json = state.wasm.scanRgbaJson(
      input.imageData.data,
      input.imageData.width,
      input.imageData.height,
      nodes.modeSelect.value,
    );
    return { ...JSON.parse(json), input: input.info };
  } catch (error) {
    return { ok: false, error: error.message };
  }
}

function rustInputImageData() {
  const target = nodes.scanTarget.value;
  const sourceRect =
    target === "manual" && state.manualCrop && state.diagnostics?.source?.displayed
      ? displayRectToSourceRect(state.manualCrop)
      : { x: 0, y: 0, width: state.source.width, height: state.source.height };

  const clamped = clampRect(sourceRect, {
    x: 0,
    y: 0,
    width: state.source.width,
    height: state.source.height,
  });
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(clamped.width));
  canvas.height = Math.max(1, Math.round(clamped.height));
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  ctx.drawImage(
    state.source,
    clamped.x,
    clamped.y,
    clamped.width,
    clamped.height,
    0,
    0,
    canvas.width,
    canvas.height,
  );

  return {
    imageData: ctx.getImageData(0, 0, canvas.width, canvas.height),
    info: {
      target,
      sourceRect: clamped,
      width: canvas.width,
      height: canvas.height,
    },
  };
}

function drawOverlay() {
  const diagnostics = state.diagnostics;
  if (!diagnostics) return;
  const canvas = nodes.overlayCanvas;
  const ctx = canvas.getContext("2d");
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  const overlay = nodes.overlaySelect.value;
  if (overlay === "none") return;

  const rustCrop = sourceRectToDisplayRect(diagnostics.rust?.crop);
  if ((overlay === "all" || overlay === "crop") && rustCrop) {
    ctx.strokeStyle = "#d36b00";
    ctx.lineWidth = 4;
    strokeRect(ctx, rustCrop);
  }

  for (const attempt of diagnostics.rust?.attempts ?? []) {
    const rect = sourceRectToDisplayRect(attempt.region);
    if (!rect) continue;
    ctx.strokeStyle = attempt.decoded ? "rgba(0, 135, 90, 0.65)" : "rgba(155, 93, 229, 0.24)";
    ctx.lineWidth = attempt.decoded ? 3 : 1;
    strokeRect(ctx, rect);
  }

  if ((overlay === "all" || overlay === "crop") && state.manualCrop) {
    ctx.strokeStyle = "#ff6b00";
    ctx.lineWidth = 4;
    ctx.setLineDash([10, 6]);
    strokeRect(ctx, state.manualCrop);
    ctx.setLineDash([]);
  }

  if (state.activeDrag) {
    ctx.strokeStyle = "#ff6b00";
    ctx.lineWidth = 3;
    ctx.setLineDash([8, 5]);
    strokeRect(ctx, normalizeRect(state.activeDrag.start, state.activeDrag.current));
    ctx.setLineDash([]);
  }

  const rustQuad = normalizeRustQuad(diagnostics.rust?.quad);
  if (overlay === "all" && rustQuad) {
    ctx.strokeStyle = "#006d77";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(rustQuad.topLeft.x, rustQuad.topLeft.y);
    ctx.lineTo(rustQuad.topRight.x, rustQuad.topRight.y);
    ctx.lineTo(rustQuad.bottomRight.x, rustQuad.bottomRight.y);
    ctx.lineTo(rustQuad.bottomLeft.x, rustQuad.bottomLeft.y);
    ctx.closePath();
    ctx.stroke();
  }
}

function normalizeRustQuad(quad) {
  if (!quad || !state.diagnostics?.source?.displayed) return null;
  const points = {
    topLeft: quad.top_left,
    topRight: quad.top_right,
    bottomRight: quad.bottom_right,
    bottomLeft: quad.bottom_left,
  };
  return Object.fromEntries(
    Object.entries(points).map(([key, point]) => [key, sourcePointToDisplayPoint(point)]),
  );
}

function drawThresholdPlaceholder() {
  const ctx = nodes.thresholdCanvas.getContext("2d");
  ctx.clearRect(0, 0, nodes.thresholdCanvas.width, nodes.thresholdCanvas.height);
  ctx.fillStyle = "#fff";
  ctx.fillRect(0, 0, nodes.thresholdCanvas.width, nodes.thresholdCanvas.height);
  ctx.fillStyle = "#66736b";
  ctx.font = "13px system-ui";
  ctx.fillText("Threshold image is produced by Rust diagnostics only when exported.", 14, 30);
}

function updateMetrics() {
  const diagnostics = state.diagnostics;
  const rust = diagnostics.rust;
  nodes.metricImage.textContent = `${diagnostics.source.name} (${diagnostics.source.width}x${diagnostics.source.height})`;
  nodes.metricThreshold.textContent = rust?.auto
    ? `${rust.auto.threshold} / module ${rust.auto.module_px} / ${rust.auto.layout}`
    : "-";
  nodes.metricRust.textContent = rust?.ok
    ? `decoded ${rust.payload_len} bytes: ${rust.payload_utf8_lossy}`
    : rust?.error || "not available";
  nodes.metricRustInput.textContent = rust?.input
    ? `${rust.input.target}: ${formatRect(rust.input.sourceRect)} -> ${rust.input.width}x${rust.input.height}`
    : "not available";
  nodes.metricRustCrop.textContent = rust?.crop ? formatRect(rust.crop) : "none";
  nodes.metricCrop.textContent = rust?.attempts?.length
    ? `${rust.attempts.length} Rust candidate regions`
    : "none";
  nodes.metricComponents.textContent = rust?.candidate_count ?? "-";
  nodes.metricAnchors.textContent = rust?.anchor_count ?? "-";
  nodes.metricQuad.textContent = rust?.quad ? "estimated by Rust" : "none";
  nodes.jsonOutput.textContent = JSON.stringify(
    {
      source: diagnostics.source,
      manualCrop: state.manualCrop ? displayRectToSourceRect(state.manualCrop) : null,
      rust,
    },
    null,
    2,
  );
}

function canvasPoint(event) {
  const rect = nodes.overlayCanvas.getBoundingClientRect();
  return {
    x: ((event.clientX - rect.left) / rect.width) * nodes.overlayCanvas.width,
    y: ((event.clientY - rect.top) / rect.height) * nodes.overlayCanvas.height,
  };
}

function normalizeRect(a, b) {
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  return { x, y, width: Math.abs(a.x - b.x), height: Math.abs(a.y - b.y) };
}

function clampRect(rect, bounds) {
  const x = Math.max(bounds.x, Math.min(rect.x, bounds.x + bounds.width));
  const y = Math.max(bounds.y, Math.min(rect.y, bounds.y + bounds.height));
  const maxX = Math.max(x, Math.min(rect.x + rect.width, bounds.x + bounds.width));
  const maxY = Math.max(y, Math.min(rect.y + rect.height, bounds.y + bounds.height));
  return { x, y, width: Math.max(1, maxX - x), height: Math.max(1, maxY - y) };
}

function pointInsideRect(point, rect) {
  return (
    point.x >= rect.x &&
    point.y >= rect.y &&
    point.x <= rect.x + rect.width &&
    point.y <= rect.y + rect.height
  );
}

function displayRectToSourceRect(rect) {
  const displayed = state.diagnostics.source.displayed;
  const scaleX = state.source.width / displayed.width;
  const scaleY = state.source.height / displayed.height;
  return {
    x: Math.round((rect.x - displayed.x) * scaleX),
    y: Math.round((rect.y - displayed.y) * scaleY),
    width: Math.round(rect.width * scaleX),
    height: Math.round(rect.height * scaleY),
  };
}

function sourceRectToDisplayRect(rect) {
  if (!rect || !state.diagnostics?.source?.displayed) return null;
  const displayed = state.diagnostics.source.displayed;
  const scaleX = displayed.width / state.source.width;
  const scaleY = displayed.height / state.source.height;
  return {
    x: displayed.x + rect.x * scaleX,
    y: displayed.y + rect.y * scaleY,
    width: rect.width * scaleX,
    height: rect.height * scaleY,
  };
}

function sourcePointToDisplayPoint(point) {
  const displayed = state.diagnostics.source.displayed;
  return {
    x: displayed.x + point.x * (displayed.width / state.source.width),
    y: displayed.y + point.y * (displayed.height / state.source.height),
  };
}

function strokeRect(ctx, rect) {
  ctx.strokeRect(rect.x + 0.5, rect.y + 0.5, rect.width, rect.height);
}

function formatRect(rect) {
  return `${Math.round(rect.x)}, ${Math.round(rect.y)}, ${Math.round(rect.width)}x${Math.round(rect.height)}`;
}

async function sampleCanvas() {
  const canvas = document.createElement("canvas");
  canvas.width = 960;
  canvas.height = 360;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  if (!state.wasm) return canvas;
  const svg = state.wasm.encodeSvgWithGeometry("debug sample", 4, 4);
  const image = new Image();
  const blob = new Blob([svg], { type: "image/svg+xml" });
  const url = URL.createObjectURL(blob);
  await new Promise((resolve, reject) => {
    image.onload = resolve;
    image.onerror = reject;
    image.src = url;
  });
  try {
    ctx.drawImage(image, 110, 84);
  } finally {
    URL.revokeObjectURL(url);
  }
  return canvas;
}
