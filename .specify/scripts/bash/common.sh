#!/usr/bin/env bash

set -euo pipefail

# リポジトリルートを取得（git が無い場合は .specify / .git を探索）
get_repo_root() {
    if git rev-parse --show-toplevel >/dev/null 2>&1; then
        git rev-parse --show-toplevel
        return
    fi

    local dir
    dir="$(CDPATH="" cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    while [ "$dir" != "/" ]; do
        if [ -d "$dir/.git" ] || [ -d "$dir/.specify" ]; then
            echo "$dir"
            return
        fi
        dir="$(dirname "$dir")"
    done

    echo "" 
}

normalize_spec_id() {
    local raw="$1"
    if [ -z "$raw" ]; then
        echo ""
        return
    fi

    local upper
    upper="${raw^^}"
    if [[ "$upper" == SPEC-* ]]; then
        local suffix
        suffix="${upper#SPEC-}"
        echo "SPEC-${suffix,,}"
        return
    fi

    echo "SPEC-${upper,,}"
}

is_valid_spec_id() {
    local spec_id="$1"
    [[ "$spec_id" =~ ^SPEC-[a-f0-9]{8}$ ]]
}

require_spec_id() {
    local spec_id="$1"
    if [ -z "$spec_id" ]; then
        echo "ERROR: SPEC_ID が指定されていません" >&2
        return 1
    fi
    if ! is_valid_spec_id "$spec_id"; then
        echo "ERROR: SPEC_ID 形式が不正です: $spec_id" >&2
        echo "期待形式: SPEC-[a-f0-9]{8}" >&2
        return 1
    fi
}

get_feature_paths() {
    local spec_id="$1"
    local repo_root
    repo_root="$(get_repo_root)"
    if [ -z "$repo_root" ]; then
        echo "ERROR: リポジトリルートを特定できません" >&2
        return 1
    fi

    local feature_dir="$repo_root/specs/$spec_id"

    cat <<EOF_PATHS
REPO_ROOT='$repo_root'
SPEC_ID='$spec_id'
FEATURE_DIR='$feature_dir'
FEATURE_SPEC='$feature_dir/spec.md'
IMPL_PLAN='$feature_dir/plan.md'
TASKS='$feature_dir/tasks.md'
RESEARCH='$feature_dir/research.md'
DATA_MODEL='$feature_dir/data-model.md'
QUICKSTART='$feature_dir/quickstart.md'
CONTRACTS_DIR='$feature_dir/contracts'
EOF_PATHS
}
