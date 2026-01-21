#!/usr/bin/env bash
# すべてのスクリプトで使用する共通関数と変数

# SPEC ID（specs/SPEC-xxxxxxxx）の形式を正規化して返す
# - 入力許容: SPEC-xxxxxxxx / xxxxxxxx / feature/SPEC-xxxxxxxx（大文字小文字は問わない）
# - 出力: SPEC-xxxxxxxx（suffixは小文字16進数）
normalize_spec_id() {
    local raw="$1"
    raw=$(echo "$raw" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')

    if [[ -z "$raw" ]]; then
        return 1
    fi

    # feature/SPEC-XXXXXXXX 形式を許容（上流互換）
    if [[ "$raw" =~ ^feature/(SPEC-[A-Za-z0-9]{8})$ ]]; then
        raw="${BASH_REMATCH[1]}"
    fi

    # SPEC-xxxxxxxx（16進数8桁）
    if [[ "$raw" =~ ^SPEC-[A-Fa-f0-9]{8}$ ]]; then
        local suffix="${raw#SPEC-}"
        suffix=$(echo "$suffix" | tr '[:upper:]' '[:lower:]')
        echo "SPEC-${suffix}"
        return 0
    fi

    # xxxxxxxx（16進数8桁）
    if [[ "$raw" =~ ^[A-Fa-f0-9]{8}$ ]]; then
        local suffix
        suffix=$(echo "$raw" | tr '[:upper:]' '[:lower:]')
        echo "SPEC-${suffix}"
        return 0
    fi

    return 1
}

# リポジトリルートを取得（非gitリポジトリのフォールバック付き）
get_repo_root() {
    if git rev-parse --show-toplevel >/dev/null 2>&1; then
        git rev-parse --show-toplevel
    else
        # 非gitリポジトリの場合はスクリプトの場所にフォールバック
        local script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
        (cd "$script_dir/../../.." && pwd)
    fi
}

# gitが利用可能かチェック
has_git() {
    git rev-parse --show-toplevel >/dev/null 2>&1
}

# 現在のgitブランチ名を取得（非gitリポジトリの場合は空）
get_git_branch() {
    if git rev-parse --abbrev-ref HEAD >/dev/null 2>&1; then
        git rev-parse --abbrev-ref HEAD
    else
        echo ""
    fi
}

# specs/ 配下の SPEC-* ディレクトリ一覧を取得（存在しない場合は空）
list_spec_dirs() {
    local repo_root="$1"
    local specs_dir="$repo_root/specs"

    if [[ ! -d "$specs_dir" ]]; then
        return 0
    fi

    local nullglob_was_enabled=false
    if shopt -q nullglob; then
        nullglob_was_enabled=true
    fi
    shopt -s nullglob
    local dirs=("$specs_dir"/SPEC-*)
    if ! $nullglob_was_enabled; then
        shopt -u nullglob
    fi

    for dir in "${dirs[@]}"; do
        if [[ -d "$dir" ]]; then
            basename "$dir"
        fi
    done
}

# SPEC ID を決定（失敗したらエラー）
resolve_spec_id() {
    local repo_root
    repo_root=$(get_repo_root)

    # 1) 環境変数から（推奨）
    if [[ -n "${SPECIFY_FEATURE:-}" ]]; then
        local normalized=""
        if normalized=$(normalize_spec_id "$SPECIFY_FEATURE"); then
            echo "$normalized"
            return 0
        fi
        echo "[specify] エラー: SPECIFY_FEATURE が無効です: $SPECIFY_FEATURE" >&2
        echo "[specify] ヒント: 例) export SPECIFY_FEATURE=SPEC-1defd8fd" >&2
        return 1
    fi

    # 2) ブランチ名が SPEC-xxxxxxxx の場合はそこから
    local git_branch
    git_branch=$(get_git_branch)
    if [[ -n "$git_branch" ]]; then
        local normalized=""
        if normalized=$(normalize_spec_id "$git_branch"); then
            echo "$normalized"
            return 0
        fi
    fi

    # 3) specs/ に SPEC-* が1つだけ存在するならそれを採用（安全な場合のみ）
    local spec_dirs=()
    while IFS= read -r line; do
        [[ -n "$line" ]] && spec_dirs+=("$line")
    done < <(list_spec_dirs "$repo_root")

    if [[ ${#spec_dirs[@]} -eq 1 ]]; then
        echo "${spec_dirs[0]}"
        return 0
    fi

    echo "[specify] エラー: SPEC_ID を特定できませんでした。" >&2
    echo "[specify] 対処: --spec-id を指定して実行するか、SPECIFY_FEATURE を設定してください。" >&2
    echo "[specify] 例: .specify/scripts/bash/setup-plan.sh --spec-id SPEC-1defd8fd --json" >&2
    echo "[specify] 一覧: specs/specs.md（存在する場合）" >&2
    return 1
}

check_feature_branch() {
    local spec_id="$1"
    local has_git_repo="$2"

    # 非gitリポジトリの場合、検証はできないが続行
    if [[ "$has_git_repo" != "true" ]]; then
        echo "[specify] 警告: Gitリポジトリが検出されませんでした。検証をスキップしました。" >&2
        return 0
    fi

    if [[ ! "$spec_id" =~ ^SPEC-[a-f0-9]{8}$ ]]; then
        echo "[specify] エラー: 無効なSPEC IDです: $spec_id" >&2
        echo "[specify] 形式: SPEC-xxxxxxxx（xは小文字16進数8桁）" >&2
        return 1
    fi

    return 0
}

get_feature_dir() { echo "$1/specs/$2"; }

load_feature_paths() {
    local repo_root
    repo_root=$(get_repo_root)
    local git_branch
    git_branch=$(get_git_branch)
    local has_git_repo="false"

    if has_git; then
        has_git_repo="true"
    fi

    local spec_id
    spec_id=$(resolve_spec_id) || return 1
    check_feature_branch "$spec_id" "$has_git_repo" || return 1

    local feature_dir
    feature_dir=$(get_feature_dir "$repo_root" "$spec_id")

    REPO_ROOT="$repo_root"
    GIT_BRANCH="$git_branch"
    HAS_GIT="$has_git_repo"
    SPEC_ID="$spec_id"
    FEATURE_DIR="$feature_dir"
    FEATURE_SPEC="$feature_dir/spec.md"
    IMPL_PLAN="$feature_dir/plan.md"
    TASKS="$feature_dir/tasks.md"
    RESEARCH="$feature_dir/research.md"
    DATA_MODEL="$feature_dir/data-model.md"
    QUICKSTART="$feature_dir/quickstart.md"
    CONTRACTS_DIR="$feature_dir/contracts"
}

check_file() { [[ -f "$1" ]] && echo "  ✓ $2" || echo "  ✗ $2"; }
check_dir() { [[ -d "$1" && -n $(ls -A "$1" 2>/dev/null) ]] && echo "  ✓ $2" || echo "  ✗ $2"; }

# jq がない環境向けの最小限のJSON文字列エスケープ
# - 文字列をJSONに埋め込めるように \, ", 改行などをエスケープして返す（囲みの " は付けない）
json_escape_string() {
    local value="$1"
    value=${value//\\/\\\\}
    value=${value//\"/\\\"}
    value=${value//$'\n'/\\n}
    value=${value//$'\r'/\\r}
    value=${value//$'\t'/\\t}
    value=${value//$'\f'/\\f}
    value=${value//$'\b'/\\b}
    printf '%s' "$value"
}
