#!/usr/bin/env bash

set -euo pipefail

JSON_MODE=false
ARGS=()

while [ $# -gt 0 ]; do
    case "$1" in
        --json)
            JSON_MODE=true
            shift
            ;;
        --help|-h)
            cat <<'USAGE'
使い方: create-new-feature.sh [--json] <機能説明>

オプション:
  --json   JSON 形式で出力
  --help   ヘルプ表示
USAGE
            exit 0
            ;;
        *)
            ARGS+=("$1")
            shift
            ;;
    esac
done

FEATURE_DESCRIPTION="${ARGS[*]}"
if [ -z "$FEATURE_DESCRIPTION" ]; then
    echo "ERROR: 機能説明が空です" >&2
    exit 1
fi

SCRIPT_DIR="$(CDPATH="" cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

REPO_ROOT="$(get_repo_root)"
if [ -z "$REPO_ROOT" ]; then
    echo "ERROR: リポジトリルートを特定できません" >&2
    exit 1
fi

SPECS_DIR="$REPO_ROOT/specs"
mkdir -p "$SPECS_DIR"

random_hex() {
    if command -v python3 >/dev/null 2>&1; then
        python3 - <<'PY'
import secrets
print(secrets.token_hex(4))
PY
        return
    fi
    if command -v python >/dev/null 2>&1; then
        python - <<'PY'
import os
print(os.urandom(4).hex())
PY
        return
    fi
    if command -v openssl >/dev/null 2>&1; then
        openssl rand -hex 4
        return
    fi
    od -An -N4 -tx1 /dev/urandom | tr -d ' \n'
}

SPEC_ID=""
for _ in $(seq 1 20); do
    SPEC_ID="SPEC-$(random_hex)"
    if [ ! -d "$SPECS_DIR/$SPEC_ID" ]; then
        break
    fi
    SPEC_ID=""
done

if [ -z "$SPEC_ID" ]; then
    echo "ERROR: SPEC_ID を生成できませんでした" >&2
    exit 1
fi

FEATURE_DIR="$SPECS_DIR/$SPEC_ID"
mkdir -p "$FEATURE_DIR"

SPEC_FILE="$FEATURE_DIR/spec.md"
TEMPLATE="$REPO_ROOT/.specify/templates/spec-template.md"

if [ -f "$TEMPLATE" ]; then
    cp "$TEMPLATE" "$SPEC_FILE"
else
    cat <<'FALLBACK' > "$SPEC_FILE"
# 機能仕様: [FEATURE_NAME]

**仕様ID**: `[SPEC_ID]`
**作成日**: [DATE]
**ステータス**: ドラフト
**カテゴリ**: GUI
**入力**: ユーザー説明: "[INPUT]"
FALLBACK
fi

TODAY="$(date +%Y-%m-%d)"

if command -v python3 >/dev/null 2>&1; then
    python3 - <<'PY' "$SPEC_FILE" "$SPEC_ID" "$TODAY" "$FEATURE_DESCRIPTION"
import sys
from pathlib import Path

spec_file = Path(sys.argv[1])
spec_id = sys.argv[2]
created = sys.argv[3]
feature_description = sys.argv[4]

# タイトルは入力を短く整形
feature_title = feature_description.strip().replace("\n", " ")
if len(feature_title) > 80:
    feature_title = feature_title[:77] + "..."

text = spec_file.read_text(encoding="utf-8")
text = text.replace("[SPEC_ID]", spec_id)
text = text.replace("[DATE]", created)
text = text.replace("[UPDATED_DATE]", created)
text = text.replace("[INPUT]", feature_description)
text = text.replace("[FEATURE_NAME]", feature_title)

spec_file.write_text(text, encoding="utf-8")
PY
elif command -v python >/dev/null 2>&1; then
    python - <<'PY' "$SPEC_FILE" "$SPEC_ID" "$TODAY" "$FEATURE_DESCRIPTION"
import sys
from pathlib import Path

spec_file = Path(sys.argv[1])
spec_id = sys.argv[2]
created = sys.argv[3]
feature_description = sys.argv[4]

feature_title = feature_description.strip().replace("\\n", " ")
if len(feature_title) > 80:
    feature_title = feature_title[:77] + "..."

text = spec_file.read_text(encoding="utf-8")
text = text.replace("[SPEC_ID]", spec_id)
text = text.replace("[DATE]", created)
text = text.replace("[UPDATED_DATE]", created)
text = text.replace("[INPUT]", feature_description)
text = text.replace("[FEATURE_NAME]", feature_title)

spec_file.write_text(text, encoding="utf-8")
PY
fi

UPDATE_INDEX="$REPO_ROOT/.specify/scripts/bash/update-specs-index.sh"
if [ -x "$UPDATE_INDEX" ]; then
    "$UPDATE_INDEX" >/dev/null 2>&1 || true
fi

if $JSON_MODE; then
    if command -v python3 >/dev/null 2>&1; then
        python3 - <<'PY' "$SPEC_ID" "$SPEC_FILE" "$FEATURE_DIR"
import json
import sys

spec_id, spec_file, feature_dir = sys.argv[1:4]
print(json.dumps({
    "SPEC_ID": spec_id,
    "SPEC_FILE": spec_file,
    "FEATURE_DIR": feature_dir,
}, ensure_ascii=False))
PY
    elif command -v python >/dev/null 2>&1; then
        python - <<'PY' "$SPEC_ID" "$SPEC_FILE" "$FEATURE_DIR"
import json
import sys

spec_id, spec_file, feature_dir = sys.argv[1:4]
print(json.dumps({
    "SPEC_ID": spec_id,
    "SPEC_FILE": spec_file,
    "FEATURE_DIR": feature_dir,
}, ensure_ascii=False))
PY
    else
        printf '{"SPEC_ID":"%s","SPEC_FILE":"%s","FEATURE_DIR":"%s"}\n' "$SPEC_ID" "$SPEC_FILE" "$FEATURE_DIR"
    fi
else
    echo "SPEC_ID: $SPEC_ID"
    echo "SPEC_FILE: $SPEC_FILE"
    echo "FEATURE_DIR: $FEATURE_DIR"
fi
