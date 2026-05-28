#!/usr/bin/env bash
set -euo pipefail

ENFORCE_EXIT=1
if [[ "${1:-}" == "--no-fail" ]]; then
  ENFORCE_EXIT=0
fi
export BURST_RELIABILITY_ENFORCE_EXIT="${ENFORCE_EXIT}"

OUTPUT_JSON="${BURST_RELIABILITY_OUTPUT_JSON:-target/burst-reliability/pr.json}"
mkdir -p "$(dirname "${OUTPUT_JSON}")"
NON_GATING_RAW="${BURST_RELIABILITY_NON_GATING:-0.40}"
export BURST_RELIABILITY_NON_GATING="${NON_GATING_RAW}"

LOG_FILE="$(mktemp)"
echo "[burst-reliability] running scanner loss-sweep test"
cargo test -p glyphnet-scanner scanner_erasure_burst_loss_sweep_meets_baseline_targets -- --ignored --nocapture 2>&1 | tee "${LOG_FILE}"

python3 - "${LOG_FILE}" "${OUTPUT_JSON}" <<'PY'
import json
import pathlib
import re
import sys
import os

log_path = pathlib.Path(sys.argv[1])
out_path = pathlib.Path(sys.argv[2])
line_re = re.compile(
    r"\[burst-reliability\]\s+drop_rate=(?P<drop>[0-9.]+)\s+success_rate=(?P<success>[0-9.]+)\s+median_frames=(?P<frames>[0-9-]+)\s+median_completion_ms=(?P<ms>[0-9-]+)"
)
cases = []
for line in log_path.read_text().splitlines():
    m = line_re.search(line)
    if not m:
        continue
    frames = m.group("frames")
    completion_ms = m.group("ms")
    cases.append(
        {
            "drop_rate": float(m.group("drop")),
            "success_rate": float(m.group("success")),
            "median_frames": None if frames == "-" else int(frames),
            "median_completion_ms": None if completion_ms == "-" else int(completion_ms),
        }
    )

if not cases:
    print("[burst-reliability] ERROR: no metrics found in test output", file=sys.stderr)
    sys.exit(2)

targets = {
    0.10: 0.95,
    0.20: 0.90,
    0.30: 0.65,
    0.40: 0.45,
}
non_gating = {
    round(float(item.strip()), 2)
    for item in os.environ.get("BURST_RELIABILITY_NON_GATING", "").split(",")
    if item.strip()
}
status = "pass"
for case in cases:
    drop_key = round(case["drop_rate"], 2)
    threshold = targets.get(drop_key, 0.0)
    gating = drop_key not in non_gating
    case["gating"] = gating
    case["target_success_rate"] = threshold
    case["status"] = "pass" if case["success_rate"] >= threshold else "fail"
    gate_label = "gating" if gating else "non-gating"
    print(
        f"[burst-reliability] drop_rate={case['drop_rate']:.2f} success={case['success_rate']:.3f} target={threshold:.3f} ({case['status']}, {gate_label})"
    )
    if case["status"] == "fail" and gating:
        status = "fail"

payload = {
    "status": status,
    "cases": sorted(cases, key=lambda c: c["drop_rate"]),
}
out_path.write_text(json.dumps(payload, indent=2) + "\n")

if status == "fail":
    print("[burst-reliability] FAIL: reliability below baseline targets", file=sys.stderr)
    if os.environ.get("BURST_RELIABILITY_ENFORCE_EXIT", "1") == "1":
        sys.exit(1)
else:
    print("[burst-reliability] PASS: reliability within baseline targets")
PY
