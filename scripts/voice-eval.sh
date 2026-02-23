#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST_PATH="${GWT_VOICE_EVAL_MANIFEST:-$ROOT_DIR/tests/voice_eval/manifest.json}"
OUTPUT_PATH="${GWT_VOICE_EVAL_OUTPUT:-$ROOT_DIR/tests/voice_eval/latest-report.json}"
BASELINE_PATH="${GWT_VOICE_EVAL_BASELINE:-$ROOT_DIR/tests/voice_eval/baseline.json}"
QUALITIES="${GWT_VOICE_EVAL_QUALITIES:-fast,balanced,accurate}"
MODELS="${GWT_VOICE_EVAL_MODELS:-}"

if [[ ! -f "$MANIFEST_PATH" ]]; then
  echo "voice-eval: manifest not found: $MANIFEST_PATH" >&2
  echo "Create one from: $ROOT_DIR/tests/voice_eval/manifest.template.json" >&2
  exit 1
fi

EXTRA_ARGS=()
if [[ -f "$BASELINE_PATH" ]]; then
  EXTRA_ARGS+=(--baseline "$BASELINE_PATH")
fi

echo "[voice-eval] manifest: $MANIFEST_PATH"
echo "[voice-eval] output:   $OUTPUT_PATH"
echo "[voice-eval] qualities: $QUALITIES"
if [[ -n "$MODELS" ]]; then
  echo "[voice-eval] models:   $MODELS"
fi

CMD=(
  cargo run -p gwt-tauri --bin voice_eval --
  --manifest "$MANIFEST_PATH"
  --output "$OUTPUT_PATH"
)

if [[ -n "$QUALITIES" ]]; then
  CMD+=(--qualities "$QUALITIES")
fi
if [[ -n "$MODELS" ]]; then
  CMD+=(--models "$MODELS")
fi
if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
  CMD+=("${EXTRA_ARGS[@]}")
fi
if [[ $# -gt 0 ]]; then
  CMD+=("$@")
fi

"${CMD[@]}"
