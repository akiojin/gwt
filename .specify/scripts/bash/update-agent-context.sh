#!/usr/bin/env bash

set -euo pipefail

SPEC_ID=""
AGENT_TYPE=""
FORCE=false

while [ $# -gt 0 ]; do
    case "$1" in
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
使い方: update-agent-context.sh --spec-id <SPEC_ID> [agent]
USAGE
            exit 0
            ;;
        *)
            AGENT_TYPE="$1"
            shift
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

REPO_ROOT="$(get_repo_root)"
if [ -z "$REPO_ROOT" ]; then
    echo "ERROR: リポジトリルートを特定できません" >&2
    exit 1
fi

TEMPLATE="$REPO_ROOT/.specify/templates/agent-file-template.md"
if [ ! -f "$TEMPLATE" ]; then
    echo "WARNING: agent-file-template.md が見つかりません: $TEMPLATE" >&2
    exit 0
fi

case "$AGENT_TYPE" in
    ""|claude)
        TARGET_FILE="$REPO_ROOT/CLAUDE.md"
        ;;
    gemini)
        TARGET_FILE="$REPO_ROOT/GEMINI.md"
        ;;
    codex)
        TARGET_FILE="$REPO_ROOT/AGENTS.md"
        ;;
    *)
        echo "WARNING: 未対応の agent 指定のためスキップします: $AGENT_TYPE" >&2
        exit 0
        ;;
esac

PROJECT_NAME="$(basename "$REPO_ROOT")"
TODAY="$(date +%Y-%m-%d)"

if [ -f "$TARGET_FILE" ] && [ "$FORCE" != true ]; then
    if ! grep -q "SPECIFY: AUTO-GENERATED" "$TARGET_FILE"; then
        echo "WARNING: $TARGET_FILE は手動管理のため更新をスキップします" >&2
        exit 0
    fi
fi

manual_additions=""
if [ -f "$TARGET_FILE" ]; then
    if grep -q "MANUAL ADDITIONS START" "$TARGET_FILE"; then
        manual_additions=$(awk '/MANUAL ADDITIONS START/{flag=1;next}/MANUAL ADDITIONS END/{flag=0}flag' "$TARGET_FILE")
    fi
fi

if command -v python3 >/dev/null 2>&1; then
    content=$(python3 - <<'PY' "$TEMPLATE" "$PROJECT_NAME" "$TODAY"
import sys
from pathlib import Path

template = Path(sys.argv[1]).read_text(encoding="utf-8")
project_name = sys.argv[2]
today = sys.argv[3]

text = template.replace("[PROJECT_NAME]", project_name)
text = text.replace("[DATE]", today)
print(text)
PY
    )
elif command -v python >/dev/null 2>&1; then
    content=$(python - <<'PY' "$TEMPLATE" "$PROJECT_NAME" "$TODAY"
import sys
from pathlib import Path

template = Path(sys.argv[1]).read_text(encoding="utf-8")
project_name = sys.argv[2]
today = sys.argv[3]

text = template.replace("[PROJECT_NAME]", project_name)
text = text.replace("[DATE]", today)
print(text)
PY
    )
else
    content=$(sed -e "s/\\[PROJECT_NAME\\]/$PROJECT_NAME/g" -e "s/\\[DATE\\]/$TODAY/g" "$TEMPLATE")
fi

{
    echo "<!-- SPECIFY: AUTO-GENERATED -->"
    printf '%s\n' "$content"
} > "$TARGET_FILE"

if [ -n "$manual_additions" ]; then
    # 手動追加部分を復元
    awk -v manual="$manual_additions" '
        /MANUAL ADDITIONS START/ { print; print manual; skip=1; next }
        /MANUAL ADDITIONS END/ { skip=0 }
        skip==1 { next }
        { print }
    ' "$TARGET_FILE" > "$TARGET_FILE.tmp"
    mv "$TARGET_FILE.tmp" "$TARGET_FILE"
fi

echo "UPDATED: $TARGET_FILE"
