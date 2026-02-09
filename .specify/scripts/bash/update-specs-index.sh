#!/usr/bin/env bash

# specs/ 配下の仕様（SPEC-xxxxxxxx）一覧を specs/specs.md に出力します。
# - 仕様ディレクトリ: specs/SPEC-[a-f0-9]{8}/
# - 仕様ファイル: specs/SPEC-xxxxxxx/spec.md
# - 過去要件: specs/archive/SPEC-[a-f0-9]{8}/

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
    local input="$1"
    echo "${input//|/\\|}"
}

extract_created() {
    local spec_file="$1"

    # Support both:
    # - **作成日**: 2026-02-08
    # - - **作成日**: 2026-02-08
    local created
    created="$(grep -m 1 -E '^\*{0,2}-?[[:space:]]*\*\*(作成日|Created)\*\*:' "$spec_file" 2>/dev/null | cut -d ':' -f2- | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"

    if [[ -z "$created" ]]; then
        echo "-"
        return 0
    fi

    echo "$created"
}

extract_category() {
    local spec_file="$1"

    # Support both:
    # - **カテゴリ**: GUI
    # - - **カテゴリ**: Porting
    local category
    category="$(grep -m 1 -E '^\*{0,2}-?[[:space:]]*\*\*カテゴリ\*\*:' "$spec_file" 2>/dev/null | cut -d ':' -f2- | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"

    if [[ -z "$category" ]]; then
        echo "-"
        return 0
    fi

    echo "$category"
}

extract_deps() {
    local spec_file="$1"

    if [[ ! -f "$spec_file" ]]; then
        return 0
    fi

    # Extract SPEC IDs from:
    # - **依存仕様**: `SPEC-xxxxxxxx`
    # - ## 依存関係 ... (until next ## header)
    awk '
function print_specs(line,   s) {
    s = line
    while (match(s, /SPEC-[a-f0-9]{8}/)) {
        print substr(s, RSTART, RLENGTH)
        s = substr(s, RSTART + RLENGTH)
    }
}
BEGIN { in_deps = 0 }
/\*\*依存仕様\*\*:/ { print_specs($0) }
/^##[[:space:]]+依存関係/ { in_deps = 1; next }
in_deps && /^##[[:space:]]+/ { in_deps = 0 }
in_deps { print_specs($0) }
' "$spec_file" | sort -u
}

REPO_ROOT="$(get_repo_root)"
if [[ -z "$REPO_ROOT" ]]; then
    echo "[specify] エラー: リポジトリルートを特定できませんでした" >&2
    exit 1
fi

SPECS_DIR="$REPO_ROOT/specs"
ARCHIVE_DIR="$SPECS_DIR/archive"
OUTPUT_FILE="$SPECS_DIR/specs.md"

mkdir -p "$SPECS_DIR"

tmp_file="$(mktemp)"
entries_file=""
archive_entries_file=""
active_entries_file=""
gui_entries_file=""
porting_entries_file=""
uncategorized_entries_file=""
trimmed_file=""
cleanup() {
    rm -f "$tmp_file"
    rm -f "$entries_file"
    rm -f "$archive_entries_file"
    rm -f "$active_entries_file"
    rm -f "$gui_entries_file"
    rm -f "$porting_entries_file"
    rm -f "$uncategorized_entries_file"
    rm -f "$trimmed_file"
}
trap cleanup EXIT

today="$(date +%Y-%m-%d)"

{
    echo "<!-- markdownlint-disable MD013 -->"
    echo "# 仕様一覧"
    echo ""
    echo "**最終更新**: $today"
    echo ""
    echo 'このファイルは `.specify/scripts/bash/update-specs-index.sh` により自動生成されました。'
    echo ""
} >"$tmp_file"

nullglob_was_enabled=false
if shopt -q nullglob; then
    nullglob_was_enabled=true
fi
shopt -s nullglob
active_spec_dirs=("$SPECS_DIR"/SPEC-*)
archive_spec_dirs=("$ARCHIVE_DIR"/SPEC-*)
if ! $nullglob_was_enabled; then
    shopt -u nullglob
fi

active_entries_file="$(mktemp)"
archive_entries_file="$(mktemp)"

collect_entries() {
    local entries_out="$1"
    local base_dir="$2"
    local kind="$3" # active | archive

    local dir
    for dir in "$base_dir"/SPEC-*; do
        [[ -d "$dir" ]] || continue
        local spec_id
        spec_id="$(basename "$dir")"

        # specs/SPEC-xxxxxxxx 以外は除外
        if [[ ! "$spec_id" =~ ^SPEC-[a-f0-9]{8}$ ]]; then
            continue
        fi

        local spec_file="$dir/spec.md"
        local title="（タイトル未設定）"
        local created="-"
        local deps=""
        local category="-"

        # spec.md が無いディレクトリは索引対象外（プレースホルダ生成を避ける）
        if [[ ! -f "$spec_file" ]]; then
            continue
        fi

        title="$(grep -m 1 -E '^#' "$spec_file" 2>/dev/null | sed -E 's/^#+[[:space:]]*//;s/[[:space:]]*$//')"
        if [[ -z "$title" ]]; then
            title="（タイトル未設定）"
        fi

        created="$(extract_created "$spec_file")"

        if [[ "$kind" = "active" ]]; then
            deps="$(extract_deps "$spec_file" | tr '\n' ' ' | sed -E 's/[[:space:]]+/ /g;s/^[[:space:]]+//;s/[[:space:]]+$//')"
            category="$(extract_category "$spec_file")"
        fi

        # Strip tabs to keep TSV well-formed
        title="${title//$'\t'/ }"
        created="${created//$'\t'/ }"
        deps="${deps//$'\t'/ }"
        category="${category//$'\t'/ }"

        printf '%s\t%s\t%s\t%s\t%s\n' "$spec_id" "$title" "$created" "$deps" "$category" >>"$entries_out"
    done
}

if [[ -d "$SPECS_DIR" ]]; then
    collect_entries "$active_entries_file" "$SPECS_DIR" "active"
fi
if [[ -d "$ARCHIVE_DIR" ]]; then
    collect_entries "$archive_entries_file" "$ARCHIVE_DIR" "archive"
fi

gui_entries_file="$(mktemp)"
porting_entries_file="$(mktemp)"
uncategorized_entries_file="$(mktemp)"

# Split active entries by category
awk -F'\t' '$5 == "GUI"' "$active_entries_file" >"$gui_entries_file" || true
awk -F'\t' '$5 == "Porting"' "$active_entries_file" >"$porting_entries_file" || true
awk -F'\t' '$5 == "-" || $5 == ""' "$active_entries_file" >"$uncategorized_entries_file" || true

render_table() {
    local title="$1"
    local kind="$2" # active | archive
    local entries_in="$3"

    echo "## $title" >>"$tmp_file"
    echo "" >>"$tmp_file"
    echo "| SPEC ID | タイトル | 作成日 |" >>"$tmp_file"
    echo "| --- | --- | --- |" >>"$tmp_file"

    if [[ ! -s "$entries_in" ]]; then
        echo "| - | （仕様がまだありません） | - |" >>"$tmp_file"
        echo "" >>"$tmp_file"
        return 0
    fi

    if [[ "$kind" = "active" ]]; then
        # Topological sort by dependencies (within active set), tie-break by created desc then SPEC ID asc.
        awk -F'\t' '
function datekey(s,   d) {
    d = "0000-00-00"
    if (match(s, /[0-9]{4}-[0-9]{2}-[0-9]{2}/)) {
        d = substr(s, RSTART, RLENGTH)
    }
    return d
}
function better(a, b,   da, db) {
    da = dk[a]; db = dk[b]
    if (da != db) return (da > db)
    return (a < b)
}
function pick_best(require_zero,   best, sid) {
    best = ""
    for (sid in nodes) {
        if (done[sid]) continue
        if (require_zero && indeg[sid] != 0) continue
        if (best == "" || better(sid, best)) best = sid
    }
    return best
}
BEGIN { n = 0 }
{
    sid = $1
    title[sid] = $2
    created[sid] = $3
    depsline[sid] = $4
    dk[sid] = datekey($3)
    nodes[sid] = 1
    n++
}
END {
    # Build graph edges dep -> sid (sid depends on dep)
    for (sid in nodes) {
        n_deps = split(depsline[sid], deps, /[[:space:]]+/)
        for (i = 1; i <= n_deps; i++) {
            dep = deps[i]
            if (dep == "" || dep == sid) continue
            if (dep in nodes) {
                adj[dep] = adj[dep] " " sid
                indeg[sid]++
            }
        }
    }

    processed = 0
    while (processed < n) {
        best = pick_best(1)
        if (best == "") {
            # Cycle or disconnected remainder: output remaining in tie-break order
            while (processed < n) {
                best = pick_best(0)
                if (best == "") break
                print best "\t" title[best] "\t" created[best]
                done[best] = 1
                processed++
            }
            break
        }

        print best "\t" title[best] "\t" created[best]
        done[best] = 1
        processed++

        n_kids = split(adj[best], kids, /[[:space:]]+/)
        for (k = 1; k <= n_kids; k++) {
            kid = kids[k]
            if (kid == "") continue
            indeg[kid]--
        }
    }
}
' "$entries_in" | while IFS=$'\t' read -r spec_id spec_title spec_created; do
            title_escaped="$(escape_md_table_cell "$spec_title")"
            created_escaped="$(escape_md_table_cell "$spec_created")"
            echo "| [$spec_id]($spec_id/spec.md) | $title_escaped | $created_escaped |" >>"$tmp_file"
        done
    else
        # Archive: created desc then SPEC ID asc
        awk -F'\t' '
function datekey(s,   d) {
    d = "0000-00-00"
    if (match(s, /[0-9]{4}-[0-9]{2}-[0-9]{2}/)) {
        d = substr(s, RSTART, RLENGTH)
    }
    return d
}
{
    print datekey($3) "\t" $1 "\t" $2 "\t" $3
}' "$entries_in" | sort -t $'\t' -k1,1r -k2,2 | while IFS=$'\t' read -r _date_key spec_id spec_title spec_created; do
            title_escaped="$(escape_md_table_cell "$spec_title")"
            created_escaped="$(escape_md_table_cell "$spec_created")"
            echo "| [$spec_id](archive/$spec_id/spec.md) | $title_escaped | $created_escaped |" >>"$tmp_file"
        done
    fi

    echo "" >>"$tmp_file"
}

{
    echo "## 運用ルール"
    echo ""
    echo '- `カテゴリ: GUI` は、現行のTauri GUI実装で有効な要件（binding）です。'
    echo '- `カテゴリ: Porting` は、TUI/WebUI由来の移植待ち（non-binding）です。未実装でも不具合ではありません。'
    echo "- Porting を実装対象にする場合は、次のどちらかを実施します:"
    echo '1. 既存 spec の内容を GUI 前提に更新し、`カテゴリ` を `GUI` に変更する'
    echo '2. 新しい GUI spec を作成し、元の Porting spec を `**依存仕様**:` で参照する'
    echo ""
} >>"$tmp_file"

render_table "現行仕様（GUI）" "active" "$gui_entries_file"
render_table "移植待ち（Porting）" "active" "$porting_entries_file"
if [[ -s "$uncategorized_entries_file" ]]; then
    render_table "カテゴリ未設定（要対応）" "active" "$uncategorized_entries_file"
fi
render_table "過去要件（archive）" "archive" "$archive_entries_file"

# markdownlint MD012: avoid trailing blank lines in generated output.
trimmed_file="$(mktemp)"
awk '
{ lines[NR] = $0 }
END {
    n = NR
    while (n > 0 && lines[n] ~ /^[[:space:]]*$/) n--
    for (i = 1; i <= n; i++) print lines[i]
}
' "$tmp_file" >"$trimmed_file"
mv "$trimmed_file" "$tmp_file"

mv "$tmp_file" "$OUTPUT_FILE"
