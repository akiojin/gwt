#!/usr/bin/env bash

# specs/ 配下の仕様（SPEC-xxxxxxxx）一覧を specs/specs.md に出力します。
# - 仕様ディレクトリ: specs/SPEC-[a-f0-9]{8}/
# - 仕様ファイル: specs/SPEC-xxxxxxx/spec.md

set -e

# Function to find the repository root by searching for existing project markers
find_repo_root() {
    local dir="$1"
    while [ "$dir" != "/" ]; do
        if [ -d "$dir/.git" ] || [ -d "$dir/.specify" ]; then
            echo "$dir"
            return 0
        fi
        dir="$(dirname "$dir")"
    done
    return 1
}

get_repo_root() {
    if git rev-parse --show-toplevel >/dev/null 2>&1; then
        git rev-parse --show-toplevel
        return 0
    fi

    local script_dir
    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    find_repo_root "$script_dir"
}

get_mtime_epoch() {
    local path="$1"
    # GNU coreutils
    if stat -c %Y "$path" >/dev/null 2>&1; then
        stat -c %Y "$path"
        return 0
    fi
    # BSD/macOS
    stat -f %m "$path" 2>/dev/null || echo "0"
}

escape_md_table_cell() {
    # Escape `|` which breaks markdown tables
    echo "$1" | sed 's/|/\\|/g'
}

REPO_ROOT="$(get_repo_root)"
if [[ -z "$REPO_ROOT" ]]; then
    echo "[specify] エラー: リポジトリルートを特定できませんでした" >&2
    exit 1
fi

SPECS_DIR="$REPO_ROOT/specs"
OUTPUT_FILE="$SPECS_DIR/specs.md"

mkdir -p "$SPECS_DIR"

tmp_file="$(mktemp)"
entries_file=""
cleanup() {
    rm -f "$tmp_file"
    rm -f "$entries_file"
}
trap cleanup EXIT

today="$(date +%Y-%m-%d)"

{
    echo "# 仕様一覧"
    echo ""
    echo "**最終更新**: $today"
    echo ""
    echo 'このファイルは `.specify/scripts/bash/update-specs-index.sh` により自動生成されました。'
    echo ""
    echo "| SPEC ID | タイトル | 作成日 |"
    echo "| --- | --- | --- |"
} >"$tmp_file"

nullglob_was_enabled=false
if shopt -q nullglob; then
    nullglob_was_enabled=true
fi
shopt -s nullglob
spec_dirs=("$SPECS_DIR"/SPEC-*)
if $nullglob_was_enabled; then
    shopt -s nullglob
else
    shopt -u nullglob
fi

if [[ ${#spec_dirs[@]} -eq 0 ]]; then
    echo "| - | （仕様がまだありません） | - |" >>"$tmp_file"
    mv "$tmp_file" "$OUTPUT_FILE"
    exit 0
fi

# Collect entries as tab-separated lines: mtime<TAB>spec_id<TAB>title<TAB>created
entries_file="$(mktemp)"

for dir in "${spec_dirs[@]}"; do
    [[ -d "$dir" ]] || continue
    spec_id="$(basename "$dir")"

    # specs/SPEC-xxxxxxxx 以外は除外
    if [[ ! "$spec_id" =~ ^SPEC-[a-f0-9]{8}$ ]]; then
        continue
    fi

    spec_file="$dir/spec.md"
    title=""
    created=""

    if [[ -f "$spec_file" ]]; then
        title="$(grep -m 1 -E '^#' "$spec_file" 2>/dev/null | sed -E 's/^#+[[:space:]]*//;s/[[:space:]]*$//')"
        created="$(grep -m 1 -F '**作成日**:' "$spec_file" 2>/dev/null | cut -d ':' -f2- | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
    fi

    if [[ -z "$title" ]]; then
        title="（タイトル未設定）"
    fi
    if [[ -z "$created" ]]; then
        created="-"
    fi

    mtime="$(get_mtime_epoch "$dir")"
    printf '%s\t%s\t%s\t%s\n' "$mtime" "$spec_id" "$title" "$created" >>"$entries_file"
done

if [[ ! -s "$entries_file" ]]; then
    echo "| - | （仕様がまだありません） | - |" >>"$tmp_file"
    mv "$tmp_file" "$OUTPUT_FILE"
    exit 0
fi

sort -rn "$entries_file" | while IFS=$'\t' read -r _mtime spec_id title created; do
    title_escaped="$(escape_md_table_cell "$title")"
    created_escaped="$(escape_md_table_cell "$created")"
    echo "| [$spec_id]($spec_id/spec.md) | $title_escaped | $created_escaped |" >>"$tmp_file"
done

mv "$tmp_file" "$OUTPUT_FILE"
