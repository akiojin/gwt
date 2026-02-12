#!/usr/bin/env bash

set -euo pipefail

JSON_MODE=false
SPEC_ID=""
FORCE=false

while [ $# -gt 0 ]; do
    case "$1" in
        --json)
            JSON_MODE=true
            shift
            ;;
        --spec-id)
            SPEC_ID="${2:-}"
            shift 2
            ;;
        --force)
            FORCE=true
            shift
            ;;
        --help|-h)
            cat <<'USAGE'
使い方: setup-plan.sh [--json] --spec-id <SPEC_ID> [--force]
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

EVAL_OUT=$(get_feature_paths "$SPEC_ID")
# shellcheck disable=SC2086
eval "$EVAL_OUT"

if [ ! -f "$FEATURE_SPEC" ]; then
    echo "ERROR: spec.md が見つかりません: $FEATURE_SPEC" >&2
    exit 1
fi

mkdir -p "$FEATURE_DIR"

TEMPLATE="$REPO_ROOT/.specify/templates/plan-template.md"
if [ -f "$IMPL_PLAN" ] && [ "$FORCE" != true ]; then
    :
else
    if [ -f "$TEMPLATE" ]; then
        cp "$TEMPLATE" "$IMPL_PLAN"
    else
        echo "WARNING: plan-template.md が見つからないため空ファイルを作成します" >&2
        : > "$IMPL_PLAN"
    fi
fi

if $JSON_MODE; then
    if command -v python3 >/dev/null 2>&1; then
        python3 - <<'PY' "$SPEC_ID" "$FEATURE_DIR" "$FEATURE_SPEC" "$IMPL_PLAN"
import json
import sys

spec_id, feature_dir, feature_spec, impl_plan = sys.argv[1:5]
print(json.dumps({
    "SPEC_ID": spec_id,
    "FEATURE_DIR": feature_dir,
    "FEATURE_SPEC": feature_spec,
    "IMPL_PLAN": impl_plan,
}, ensure_ascii=False))
PY
    elif command -v python >/dev/null 2>&1; then
        python - <<'PY' "$SPEC_ID" "$FEATURE_DIR" "$FEATURE_SPEC" "$IMPL_PLAN"
import json
import sys

spec_id, feature_dir, feature_spec, impl_plan = sys.argv[1:5]
print(json.dumps({
    "SPEC_ID": spec_id,
    "FEATURE_DIR": feature_dir,
    "FEATURE_SPEC": feature_spec,
    "IMPL_PLAN": impl_plan,
}, ensure_ascii=False))
PY
    else
        printf '{"SPEC_ID":"%s","FEATURE_DIR":"%s","FEATURE_SPEC":"%s","IMPL_PLAN":"%s"}\n' \
            "$SPEC_ID" "$FEATURE_DIR" "$FEATURE_SPEC" "$IMPL_PLAN"
    fi
else
    echo "SPEC_ID: $SPEC_ID"
    echo "FEATURE_DIR: $FEATURE_DIR"
    echo "FEATURE_SPEC: $FEATURE_SPEC"
    echo "IMPL_PLAN: $IMPL_PLAN"
fi
