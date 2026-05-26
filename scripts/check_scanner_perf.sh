#!/usr/bin/env bash
set -euo pipefail

# Enforce scanner latency regression policy against RibbonPrint decode budget.
# We gate on Criterion's median estimate because CI hosts can produce occasional
# long-tail outliers; median is more stable across runs while still reflecting
# typical scanner latency shifts caused by regressions.

TOLERANCE_PCT="${SCANNER_BENCH_TOLERANCE_PCT:-10}"
WARMUP_SECS="${SCANNER_BENCH_WARMUP_SECS:-3}"
MEASURE_SECS="${SCANNER_BENCH_MEASURE_SECS:-8}"
SAMPLE_SIZE="${SCANNER_BENCH_SAMPLE_SIZE:-40}"
ENFORCE_EXIT=1
if [[ "${1:-}" == "--no-fail" ]]; then
  ENFORCE_EXIT=0
fi
export SCANNER_BENCH_ENFORCE_EXIT="${ENFORCE_EXIT}"

BENCH_NAMES_RAW="${SCANNER_BENCH_NAMES:-}"
if [[ -n "${BENCH_NAMES_RAW}" ]]; then
  IFS=',' read -r -a BENCH_NAMES <<< "${BENCH_NAMES_RAW}"
else
  BENCH_NAME="${SCANNER_BENCH_NAME:-scan_generated_ribbon_canvas_medium}"
  BENCH_NAMES=("${BENCH_NAME}")
fi

NORMALIZED_BENCH_NAMES=()
for name in "${BENCH_NAMES[@]}"; do
  trimmed="$(echo "${name}" | xargs)"
  if [[ -n "${trimmed}" ]]; then
    NORMALIZED_BENCH_NAMES+=("${trimmed}")
  fi
done
BENCH_NAMES=("${NORMALIZED_BENCH_NAMES[@]}")
if [[ ${#BENCH_NAMES[@]} -eq 0 ]]; then
  echo "[scanner-perf] ERROR: no valid benchmark names were provided" >&2
  exit 2
fi
export SCANNER_BENCH_NAMES="$(IFS=,; echo "${BENCH_NAMES[*]}")"
NON_GATING_RAW="${SCANNER_BENCH_NON_GATING:-}"
if [[ -n "${NON_GATING_RAW}" ]]; then
  IFS=',' read -r -a NON_GATING_BENCHES <<< "${NON_GATING_RAW}"
else
  NON_GATING_BENCHES=()
fi
export SCANNER_BENCH_NON_GATING="$(IFS=,; echo "${NON_GATING_BENCHES[*]}")"

echo "[scanner-perf] running benchmarks: ${SCANNER_BENCH_NAMES}"
echo "[scanner-perf] settings: warmup=${WARMUP_SECS}s measurement=${MEASURE_SECS}s sample_size=${SAMPLE_SIZE} tolerance=${TOLERANCE_PCT}%"

BENCH_REGEX="^($(IFS='|'; echo "${BENCH_NAMES[*]}"))$"
echo "[scanner-perf] running benchmark regex '${BENCH_REGEX}'"
cargo bench -p glyphnet-scanner --bench scanner -- "${BENCH_REGEX}" \
  --warm-up-time "${WARMUP_SECS}" \
  --measurement-time "${MEASURE_SECS}" \
  --sample-size "${SAMPLE_SIZE}"

for BENCH_NAME in "${BENCH_NAMES[@]}"; do
  ESTIMATES_PATH="target/criterion/${BENCH_NAME}/new/estimates.json"
  if [[ ! -f "${ESTIMATES_PATH}" ]]; then
    echo "[scanner-perf] ERROR: criterion output missing at ${ESTIMATES_PATH}" >&2
    exit 2
  fi
done

python3 - <<'PY'
import json
import os
import pathlib
import re
import sys

profile_path = pathlib.Path("crates/glyphnet-core/src/profile.rs")
tolerance_pct = float(os.environ.get("SCANNER_BENCH_TOLERANCE_PCT", "10"))
output_json = os.environ.get("SCANNER_BENCH_OUTPUT_JSON", "").strip()
enforce_exit = os.environ.get("SCANNER_BENCH_ENFORCE_EXIT", "1") == "1"
bench_names = [
    name.strip()
    for name in os.environ.get("SCANNER_BENCH_NAMES", "scan_generated_ribbon_canvas_medium").split(",")
    if name.strip()
]
non_gating = {
    name.strip()
    for name in os.environ.get("SCANNER_BENCH_NON_GATING", "").split(",")
    if name.strip()
}

src = profile_path.read_text()
match = re.search(
    r"ProfileId::RibbonPrint(?s:.*?)max_decode_ms:\s*([0-9]+(?:\.[0-9]+)?)",
    src,
)
if not match:
    print("[scanner-perf] ERROR: could not locate RibbonPrint benchmark.max_decode_ms", file=sys.stderr)
    sys.exit(2)

budget_ms = float(match.group(1))
allowed_ms = budget_ms * (1.0 + tolerance_pct / 100.0)
cases = []
overall_status = "pass"

for bench_name in bench_names:
    est_path = pathlib.Path(f"target/criterion/{bench_name}/new/estimates.json")
    est = json.loads(est_path.read_text())
    median_ns = float(est["median"]["point_estimate"])
    median_ms = median_ns / 1_000_000.0
    status = "pass" if median_ms <= allowed_ms else "fail"
    gating = bench_name not in non_gating
    if status == "fail" and gating:
        overall_status = "fail"
    cases.append({
        "bench_name": bench_name,
        "status": status,
        "gating": gating,
        "median_ms": median_ms,
    })
    gate_label = "gating" if gating else "non-gating"
    print(f"[scanner-perf] {bench_name}: {median_ms:.3f} ms ({status}, {gate_label})")

print(f"[scanner-perf] profile budget:  {budget_ms:.3f} ms (RibbonPrint.benchmark.max_decode_ms)")
print(f"[scanner-perf] allowed max:     {allowed_ms:.3f} ms (with {tolerance_pct:.1f}% tolerance)")

if output_json:
    out_path = pathlib.Path(output_json)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "status": overall_status,
        "budget_ms": budget_ms,
        "allowed_ms": allowed_ms,
        "tolerance_pct": tolerance_pct,
        "cases": cases,
    }
    if len(cases) == 1:
        payload["bench_name"] = cases[0]["bench_name"]
        payload["median_ms"] = cases[0]["median_ms"]
    out_path.write_text(json.dumps(payload, indent=2) + "\n")

if overall_status == "fail":
    print("[scanner-perf] FAIL: scanner latency regression detected", file=sys.stderr)
    if enforce_exit:
        sys.exit(1)
else:
    print("[scanner-perf] PASS: scanner latency within allowed threshold")
PY
