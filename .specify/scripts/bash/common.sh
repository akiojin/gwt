#!/usr/bin/env bash
# すべてのスクリプトで使用する共通関数と変数

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

# 現在のブランチを取得（非gitリポジトリのフォールバック付き）
get_current_branch() {
    # まずSPECIFY_FEATURE環境変数をチェック
    if [[ -n "${SPECIFY_FEATURE:-}" ]]; then
        echo "$SPECIFY_FEATURE"
        return
    fi

    # 次にgitが利用可能ならチェック
    if git rev-parse --abbrev-ref HEAD >/dev/null 2>&1; then
        git rev-parse --abbrev-ref HEAD
        return
    fi

    # 非gitリポジトリの場合、最新の機能ディレクトリを見つける
    local repo_root=$(get_repo_root)
    local specs_dir="$repo_root/specs"

    if [[ -d "$specs_dir" ]]; then
        local latest_feature=""
        local latest_time=0

        for dir in "$specs_dir"/SPEC-*; do
            if [[ -d "$dir" ]]; then
                local mod_time=$(stat -c %Y "$dir" 2>/dev/null || stat -f %m "$dir" 2>/dev/null)
                if [[ "$mod_time" -gt "$latest_time" ]]; then
                    latest_time=$mod_time
                    latest_feature=$(basename "$dir")
                fi
            fi
        done

        if [[ -n "$latest_feature" ]]; then
            echo "$latest_feature"
            return
        fi
    fi

    echo "main"  # 最終フォールバック
}

# gitが利用可能かチェック
has_git() {
    git rev-parse --show-toplevel >/dev/null 2>&1
}

check_feature_branch() {
    local branch="$1"
    local has_git_repo="$2"

    # 非gitリポジトリの場合、ブランチ名を強制できないが出力は提供
    if [[ "$has_git_repo" != "true" ]]; then
        echo "[specify] 警告: Gitリポジトリが検出されませんでした。ブランチ検証をスキップしました" >&2
        return 0
    fi

    # SPECIFY_FEATURE環境変数が設定されている場合は優先
    if [[ -n "${SPECIFY_FEATURE:-}" ]]; then
        echo "[specify] SPECIFY_FEATURE環境変数を使用: $SPECIFY_FEATURE" >&2
        return 0
    fi

    # SPEC-UUID形式のチェック（警告のみ、エラーにはしない）
    if [[ ! "$branch" =~ ^SPEC-[a-f0-9]{8}$ ]]; then
        echo "[specify] 警告: ブランチ名がSPEC形式ではありません: $branch" >&2
        echo "[specify] Worktree設計思想に従い、任意のブランチ名で作業を続けます" >&2
        echo "[specify] 注意: specsディレクトリにSPEC-*形式のディレクトリが存在する必要があります" >&2
        # エラーではなく警告として続行
        return 0
    fi

    return 0
}

get_feature_dir() { echo "$1/specs/$2"; }

# 正確なブランチ名でディレクトリを検索
find_feature_dir() {
    local repo_root="$1"
    local branch_name="$2"
    local specs_dir="$repo_root/specs"

    # SPEC-UUID形式の検証
    if [[ "$branch_name" =~ ^SPEC-[a-f0-9]{8}$ ]]; then
        echo "$specs_dir/$branch_name"
        return
    fi

    # フォーマットが無効な場合でもパスを返す（後でより明確なエラーで失敗）
    echo "$specs_dir/$branch_name"
}

get_feature_paths() {
    local repo_root=$(get_repo_root)
    local current_branch=$(get_current_branch)
    local has_git_repo="false"

    if has_git; then
        has_git_repo="true"
    fi

    # SPEC IDベースのルックアップを使用
    local feature_dir=$(find_feature_dir "$repo_root" "$current_branch")

    cat <<EOF
REPO_ROOT='$repo_root'
CURRENT_BRANCH='$current_branch'
HAS_GIT='$has_git_repo'
FEATURE_DIR='$feature_dir'
FEATURE_SPEC='$feature_dir/spec.md'
IMPL_PLAN='$feature_dir/plan.md'
TASKS='$feature_dir/tasks.md'
RESEARCH='$feature_dir/research.md'
DATA_MODEL='$feature_dir/data-model.md'
QUICKSTART='$feature_dir/quickstart.md'
CONTRACTS_DIR='$feature_dir/contracts'
EOF
}

check_file() { [[ -f "$1" ]] && echo "  ✓ $2" || echo "  ✗ $2"; }
check_dir() { [[ -d "$1" && -n $(ls -A "$1" 2>/dev/null) ]] && echo "  ✓ $2" || echo "  ✗ $2"; }
