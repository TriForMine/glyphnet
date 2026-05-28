#!/usr/bin/env bash
set -euo pipefail

echo "[jni-fetch] starting pre-install JNI artifact fetch"

if [[ "${EAS_BUILD_PLATFORM:-}" != "android" ]]; then
  echo "[jni-fetch] non-android build; skipping"
  exit 0
fi

if [[ "${GLYPHNET_FETCH_JNI_ARTIFACT:-1}" != "1" ]]; then
  echo "[jni-fetch] GLYPHNET_FETCH_JNI_ARTIFACT disabled; skipping"
  exit 0
fi

if [[ -z "${GLYPHNET_GITHUB_TOKEN:-}" ]]; then
  echo "[jni-fetch] no GLYPHNET_GITHUB_TOKEN provided; skipping"
  exit 0
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "[jni-fetch] curl not available; skipping"
  exit 0
fi

if ! command -v unzip >/dev/null 2>&1; then
  echo "[jni-fetch] unzip not available; skipping"
  exit 0
fi

if command -v node >/dev/null 2>&1; then
  JSON_EXTRACTOR="node"
elif command -v python3 >/dev/null 2>&1; then
  JSON_EXTRACTOR="python3"
else
  echo "[jni-fetch] neither node nor python3 available; skipping"
  exit 0
fi

OWNER="${GLYPHNET_GITHUB_OWNER:-TriForMine}"
REPO="${GLYPHNET_GITHUB_REPO:-glyphnet}"
WORKFLOW_FILE="${GLYPHNET_JNI_WORKFLOW_FILE:-android-jni.yml}"
ARTIFACT_NAME="${GLYPHNET_JNI_ARTIFACT_NAME:-glyphnet-android-jni-libs}"
DEST_DIR="modules/glyphnet-scanner/android/src/main/jniLibs"

API_ROOT="https://api.github.com/repos/${OWNER}/${REPO}"
AUTH_HEADER="Authorization: Bearer ${GLYPHNET_GITHUB_TOKEN}"

echo "[jni-fetch] resolving workflow id for ${WORKFLOW_FILE}"
WORKFLOW_ID="$(
  curl -fsSL \
    -H "${AUTH_HEADER}" \
    -H "Accept: application/vnd.github+json" \
    "${API_ROOT}/actions/workflows/${WORKFLOW_FILE}" \
    | if [[ "${JSON_EXTRACTOR}" == "node" ]]; then
        node -e "const fs=require('fs'); const d=JSON.parse(fs.readFileSync(0,'utf8')); console.log(d.id ?? '');"
      else
        python3 -c "import sys, json; print(json.load(sys.stdin).get('id',''))"
      fi
)"

echo "[jni-fetch] resolving latest successful workflow run"
RUN_ID="$(
  curl -fsSL \
    -H "${AUTH_HEADER}" \
    -H "Accept: application/vnd.github+json" \
    "${API_ROOT}/actions/workflows/${WORKFLOW_ID}/runs?status=success&per_page=1" \
    | if [[ "${JSON_EXTRACTOR}" == "node" ]]; then
        node -e "const fs=require('fs'); const d=JSON.parse(fs.readFileSync(0,'utf8')); const runs=d.workflow_runs||[]; console.log(runs.length ? (runs[0].id ?? '') : '');"
      else
        python3 -c "import sys, json; d=json.load(sys.stdin); runs=d.get('workflow_runs', []); print(runs[0]['id'] if runs else '')"
      fi
)"

if [[ -z "${RUN_ID}" ]]; then
  echo "[jni-fetch] no successful workflow run found; skipping"
  exit 0
fi

echo "[jni-fetch] resolving artifact id for ${ARTIFACT_NAME} in run ${RUN_ID}"
ARTIFACT_ID="$(
  curl -fsSL \
    -H "${AUTH_HEADER}" \
    -H "Accept: application/vnd.github+json" \
    "${API_ROOT}/actions/runs/${RUN_ID}/artifacts?per_page=100" \
    | if [[ "${JSON_EXTRACTOR}" == "node" ]]; then
        node -e "const fs=require('fs'); const d=JSON.parse(fs.readFileSync(0,'utf8')); const arts=d.artifacts||[]; const name=process.argv[1]; const m=arts.find(a => a && a.name===name && !a.expired); console.log(m ? (m.id ?? '') : '');" "${ARTIFACT_NAME}"
      else
        python3 -c "import sys, json; d=json.load(sys.stdin); arts=d.get('artifacts', []); name='${ARTIFACT_NAME}'; m=[a for a in arts if a.get('name')==name and not a.get('expired', False)]; print(m[0]['id'] if m else '')"
      fi
)"

if [[ -z "${ARTIFACT_ID}" ]]; then
  echo "[jni-fetch] artifact ${ARTIFACT_NAME} not found in run ${RUN_ID}; skipping"
  exit 0
fi

TMP_ZIP="$(mktemp /tmp/glyphnet-jni-artifact.XXXXXX.zip)"
TMP_DIR="$(mktemp -d /tmp/glyphnet-jni-artifact.XXXXXX)"

echo "[jni-fetch] downloading artifact id ${ARTIFACT_ID}"
curl -fL \
  -H "${AUTH_HEADER}" \
  -H "Accept: application/vnd.github+json" \
  "${API_ROOT}/actions/artifacts/${ARTIFACT_ID}/zip" \
  -o "${TMP_ZIP}"

unzip -q "${TMP_ZIP}" -d "${TMP_DIR}"

ABI_DIRS=("arm64-v8a" "armeabi-v7a" "x86_64")
SOURCE_ROOT=""
for candidate in \
  "${TMP_DIR}" \
  "${TMP_DIR}/jniLibs" \
  "${TMP_DIR}/out/jniLibs"
do
  if [[ -d "${candidate}" ]]; then
    for abi in "${ABI_DIRS[@]}"; do
      if [[ -d "${candidate}/${abi}" ]]; then
        SOURCE_ROOT="${candidate}"
        break 2
      fi
    done
  fi
done

if [[ -z "${SOURCE_ROOT}" ]]; then
  echo "[jni-fetch] unable to find ABI folders in artifact; aborting"
  find "${TMP_DIR}" -maxdepth 3 -type d | sed 's/^/[jni-fetch] found dir: /'
  exit 1
fi

mkdir -p "${DEST_DIR}"
rm -rf "${DEST_DIR:?}"/*
for abi in "${ABI_DIRS[@]}"; do
  if [[ -d "${SOURCE_ROOT}/${abi}" ]]; then
    cp -R "${SOURCE_ROOT}/${abi}" "${DEST_DIR}/"
  fi
done

echo "[jni-fetch] installed JNI libs into ${DEST_DIR}"
