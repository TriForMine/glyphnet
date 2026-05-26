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

BENCH_NAME="${SCANNER_BENCH_NAME:-scan_real_debugger_screenshot}"
ESTIMATES_PATH="target/criterion/${BENCH_NAME}/new/estimates.json"
export SCANNER_BENCH_NAME="${BENCH_NAME}"
export SCANNER_BENCH_ESTIMATES_PATH="${ESTIMATES_PATH}"

echo "[scanner-perf] running benchmark '${BENCH_NAME}'"
echo "[scanner-perf] settings: warmup=${WARMUP_SECS}s measurement=${MEASURE_SECS}s sample_size=${SAMPLE_SIZE} tolerance=${TOLERANCE_PCT}%"

cargo bench -p glyphnet-scanner --bench scanner -- "^${BENCH_NAME}$" \
  --warm-up-time "${WARMUP_SECS}" \
  --measurement-time "${MEASURE_SECS}" \
  --sample-size "${SAMPLE_SIZE}"

if [[ ! -f "${ESTIMATES_PATH}" ]]; then
  echo "[scanner-perf] ERROR: criterion output missing at ${ESTIMATES_PATH}" >&2
  exit 2
fi

python3 - <<'PY'
import json
import os
import pathlib
import re
import sys

bench_name = os.environ.get("SCANNER_BENCH_NAME", "scan_real_debugger_screenshot")
est_path = pathlib.Path(os.environ.get("SCANNER_BENCH_ESTIMATES_PATH", f"target/criterion/{bench_name}/new/estimates.json"))
profile_path = pathlib.Path("crates/glyphnet-core/src/profile.rs")
tolerance_pct = float(os.environ.get("SCANNER_BENCH_TOLERANCE_PCT", "10"))
output_json = os.environ.get("SCANNER_BENCH_OUTPUT_JSON", "").strip()
enforce_exit = os.environ.get("SCANNER_BENCH_ENFORCE_EXIT", "1") == "1"

est = json.loads(est_path.read_text())
median_ns = float(est["median"]["point_estimate"])
median_ms = median_ns / 1_000_000.0

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
status = "pass" if median_ms <= allowed_ms else "fail"

print(f"[scanner-perf] measured median: {median_ms:.3f} ms")
print(f"[scanner-perf] profile budget:  {budget_ms:.3f} ms (RibbonPrint.benchmark.max_decode_ms)")
print(f"[scanner-perf] allowed max:     {allowed_ms:.3f} ms (with {tolerance_pct:.1f}% tolerance)")

if output_json:
    out_path = pathlib.Path(output_json)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps({
        "status": status,
        "median_ms": median_ms,
        "budget_ms": budget_ms,
        "allowed_ms": allowed_ms,
        "tolerance_pct": tolerance_pct,
    }, indent=2) + "\n")

if status == "fail":
    print("[scanner-perf] FAIL: scanner latency regression detected", file=sys.stderr)
    if enforce_exit:
        sys.exit(1)
else:
    print("[scanner-perf] PASS: scanner latency within allowed threshold")
PY
