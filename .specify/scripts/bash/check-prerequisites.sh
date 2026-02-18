#!/usr/bin/env bash

set -euo pipefail

JSON_MODE=false
REQUIRE_TASKS=false
INCLUDE_TASKS=false
SPEC_ID=""

while [ $# -gt 0 ]; do
    case "$1" in
        --json)
            JSON_MODE=true
            shift
            ;;
        --require-tasks)
            REQUIRE_TASKS=true
            shift
            ;;
        --include-tasks)
            INCLUDE_TASKS=true
            shift
            ;;
        --spec-id)
            SPEC_ID="${2:-}"
            shift 2
            ;;
        --help|-h)
            cat <<'USAGE'
使い方: check-prerequisites.sh [--json] [--require-tasks] [--include-tasks] --spec-id <SPEC_ID>
USAGE
            exit 0
            ;;
        *)
            echo "ERROR: 不明な引数: $1" >&2
            exit 1
            ;;
    esac
done

SCRIPT_DIR="$(CDPATH="" cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

if [ -z "$SPEC_ID" ]; then
    SPEC_ID="${SPECIFY_SPEC_ID:-}"
fi
SPEC_ID="$(normalize_spec_id "$SPEC_ID")"
require_spec_id "$SPEC_ID"

# 取得パス
EVAL_OUT=$(get_feature_paths "$SPEC_ID")
# shellcheck disable=SC2086
eval "$EVAL_OUT"

if [ ! -f "$FEATURE_SPEC" ]; then
    echo "ERROR: spec.md が見つかりません: $FEATURE_SPEC" >&2
    exit 1
fi

AVAILABLE_DOCS=()
if [ -f "$FEATURE_SPEC" ]; then AVAILABLE_DOCS+=("spec.md"); fi
if [ -f "$IMPL_PLAN" ]; then AVAILABLE_DOCS+=("plan.md"); fi
if [ -f "$TASKS" ]; then AVAILABLE_DOCS+=("tasks.md"); fi
if [ -f "$RESEARCH" ]; then AVAILABLE_DOCS+=("research.md"); fi
if [ -f "$DATA_MODEL" ]; then AVAILABLE_DOCS+=("data-model.md"); fi
if [ -f "$QUICKSTART" ]; then AVAILABLE_DOCS+=("quickstart.md"); fi
if [ -d "$CONTRACTS_DIR" ] && [ -n "$(ls -A "$CONTRACTS_DIR" 2>/dev/null)" ]; then
    AVAILABLE_DOCS+=("contracts/")
fi

if $REQUIRE_TASKS && [ ! -f "$TASKS" ]; then
    echo "ERROR: tasks.md が必要ですが見つかりません: $TASKS" >&2
    exit 1
fi

if $JSON_MODE; then
    if command -v python3 >/dev/null 2>&1; then
        python3 - <<'PY' \
            "$SPEC_ID" "$FEATURE_DIR" "$FEATURE_SPEC" "$IMPL_PLAN" "$TASKS" \
            "$RESEARCH" "$DATA_MODEL" "$QUICKSTART" "$CONTRACTS_DIR" \
            "${AVAILABLE_DOCS[*]}"
import json
import sys

spec_id, feature_dir, feature_spec, impl_plan, tasks, research, data_model, quickstart, contracts_dir, docs = sys.argv[1:11]
available_docs = [d for d in docs.split() if d]

payload = {
    "SPEC_ID": spec_id,
    "FEATURE_DIR": feature_dir,
    "FEATURE_SPEC": feature_spec,
    "IMPL_PLAN": impl_plan,
    "TASKS": tasks if tasks else None,
    "RESEARCH": research,
    "DATA_MODEL": data_model,
    "QUICKSTART": quickstart,
    "CONTRACTS_DIR": contracts_dir,
    "AVAILABLE_DOCS": available_docs,
}

print(json.dumps(payload, ensure_ascii=False))
PY
    elif command -v python >/dev/null 2>&1; then
        python - <<'PY' \
            "$SPEC_ID" "$FEATURE_DIR" "$FEATURE_SPEC" "$IMPL_PLAN" "$TASKS" \
            "$RESEARCH" "$DATA_MODEL" "$QUICKSTART" "$CONTRACTS_DIR" \
            "${AVAILABLE_DOCS[*]}"
import json
import sys

spec_id, feature_dir, feature_spec, impl_plan, tasks, research, data_model, quickstart, contracts_dir, docs = sys.argv[1:11]
available_docs = [d for d in docs.split() if d]

payload = {
    "SPEC_ID": spec_id,
    "FEATURE_DIR": feature_dir,
    "FEATURE_SPEC": feature_spec,
    "IMPL_PLAN": impl_plan,
    "TASKS": tasks if tasks else None,
    "RESEARCH": research,
    "DATA_MODEL": data_model,
    "QUICKSTART": quickstart,
    "CONTRACTS_DIR": contracts_dir,
    "AVAILABLE_DOCS": available_docs,
}

print(json.dumps(payload, ensure_ascii=False))
PY
    else
        printf '{"SPEC_ID":"%s","FEATURE_DIR":"%s","FEATURE_SPEC":"%s","IMPL_PLAN":"%s","TASKS":"%s","RESEARCH":"%s","DATA_MODEL":"%s","QUICKSTART":"%s","CONTRACTS_DIR":"%s","AVAILABLE_DOCS":["%s"]}\n' \
            "$SPEC_ID" "$FEATURE_DIR" "$FEATURE_SPEC" "$IMPL_PLAN" "$TASKS" \
            "$RESEARCH" "$DATA_MODEL" "$QUICKSTART" "$CONTRACTS_DIR" "${AVAILABLE_DOCS[*]}"
    fi
else
    echo "SPEC_ID: $SPEC_ID"
    echo "FEATURE_DIR: $FEATURE_DIR"
    echo "FEATURE_SPEC: $FEATURE_SPEC"
    echo "IMPL_PLAN: $IMPL_PLAN"
    if $INCLUDE_TASKS; then
        echo "TASKS: $TASKS"
    fi
    echo "AVAILABLE_DOCS: ${AVAILABLE_DOCS[*]}"
fi
