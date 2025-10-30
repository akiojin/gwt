#!/usr/bin/env bash

set -e

JSON_MODE=false
NO_BRANCH=false
SPEC_ID=""
ARGS=()
i=1
while [ $i -le $# ]; do
    arg="${!i}"
    case "$arg" in
        --json)
            JSON_MODE=true
            ;;
        --no-branch)
            NO_BRANCH=true
            ;;
        --spec-id)
            if [ $((i + 1)) -gt $# ]; then
                echo 'エラー: --spec-id には値が必要です' >&2
                exit 1
            fi
            i=$((i + 1))
            next_arg="${!i}"
            # Check if the next argument is another option (starts with --)
            if [[ "$next_arg" == --* ]]; then
                echo 'エラー: --spec-id には値が必要です' >&2
                exit 1
            fi
            SPEC_ID="$next_arg"
            ;;
        --help|-h)
            echo "使い方: $0 [--json] [--no-branch] [--spec-id <id>] <機能の説明>"
            echo ""
            echo "オプション:"
            echo "  --json          JSON形式で出力"
            echo "  --no-branch     ブランチを作成せず、現在のブランチで作業"
            echo "  --spec-id <id>  カスタムSPEC IDを指定（例: SPEC-12345678）"
            echo "  --help, -h      このヘルプメッセージを表示"
            echo ""
            echo "例:"
            echo "  $0 'ユーザー認証システムを追加'"
            echo "  $0 --no-branch 'API用のOAuth2統合を実装'"
            echo "  $0 'API用のOAuth2統合を実装' --spec-id SPEC-abcd1234"
            exit 0
            ;;
        *)
            ARGS+=("$arg")
            ;;
    esac
    i=$((i + 1))
done

FEATURE_DESCRIPTION="${ARGS[*]}"
if [ -z "$FEATURE_DESCRIPTION" ]; then
    echo "使い方: $0 [--json] [--spec-id <id>] <機能の説明>" >&2
    exit 1
fi

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

# Function to generate UUID-based SPEC ID
generate_spec_id() {
    # Generate UUID and take first 8 characters (excluding hyphens)
    local uuid=$(uuidgen 2>/dev/null || cat /proc/sys/kernel/random/uuid 2>/dev/null || echo "$(date +%s)$(( RANDOM % 10000 ))")
    local uuid_clean=$(echo "$uuid" | tr -d '-' | tr '[:upper:]' '[:lower:]')
    local spec_suffix=$(echo "$uuid_clean" | cut -c1-8)
    echo "SPEC-${spec_suffix}"
}

# Function to check if SPEC ID already exists
check_spec_id_exists() {
    local spec_id="$1"

    # Fetch all remotes to get latest branch info (suppress errors if no remotes)
    git fetch --all --prune 2>/dev/null || true

    # Check remote branches
    local remote_exists=$(git ls-remote --heads origin 2>/dev/null | grep -c "refs/heads/${spec_id}$" || echo "0")

    # Check local branches
    local local_exists=$(git branch 2>/dev/null | grep -c "^[* ]*${spec_id}$" || echo "0")

    # Check specs directory
    local spec_dir_exists=0
    if [ -d "$SPECS_DIR/${spec_id}" ]; then
        spec_dir_exists=1
    fi

    # Return 0 if exists anywhere, 1 if not exists
    if [ "$remote_exists" -gt 0 ] || [ "$local_exists" -gt 0 ] || [ "$spec_dir_exists" -gt 0 ]; then
        return 0
    else
        return 1
    fi
}

# Resolve repository root. Prefer git information when available, but fall back
# to searching for repository markers so the workflow still functions in repositories that
# were initialised with --no-git.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if git rev-parse --show-toplevel >/dev/null 2>&1; then
    REPO_ROOT=$(git rev-parse --show-toplevel)
    HAS_GIT=true
else
    REPO_ROOT="$(find_repo_root "$SCRIPT_DIR")"
    if [ -z "$REPO_ROOT" ]; then
        echo "エラー: リポジトリルートを特定できませんでした。リポジトリ内でこのスクリプトを実行してください。" >&2
        exit 1
    fi
    HAS_GIT=false
fi

cd "$REPO_ROOT"

SPECS_DIR="$REPO_ROOT/specs"
mkdir -p "$SPECS_DIR"

# Determine SPEC ID
if [ -z "$SPEC_ID" ]; then
    # Generate new SPEC ID and ensure it's unique
    SPEC_ID=$(generate_spec_id)

    # Retry if ID already exists (very unlikely but possible)
    retry_count=0
    max_retries=10
    while check_spec_id_exists "$SPEC_ID" && [ $retry_count -lt $max_retries ]; do
        >&2 echo "[specify] 警告: SPEC ID $SPEC_ID は既に存在します。新しいIDを生成中..."
        SPEC_ID=$(generate_spec_id)
        retry_count=$((retry_count + 1))
    done

    if [ $retry_count -eq $max_retries ]; then
        echo "エラー: 一意のSPEC IDの生成に失敗しました。" >&2
        exit 1
    fi
else
    # Validate provided SPEC ID format
    if ! echo "$SPEC_ID" | grep -qE '^SPEC-[a-f0-9]{8}$'; then
        echo "エラー: 無効なSPEC IDフォーマットです。SPEC-xxxxxxxx の形式で指定してください（xは小文字の16進数8桁）。" >&2
        exit 1
    fi

    # Check if provided SPEC ID already exists
    if check_spec_id_exists "$SPEC_ID"; then
        echo "エラー: SPEC ID $SPEC_ID は既に存在します。" >&2
        exit 1
    fi
fi

BRANCH_NAME="$SPEC_ID"

if [ "$NO_BRANCH" = true ]; then
    >&2 echo "[specify] ブランチ作成をスキップ: --no-branch が指定されました。現在のブランチで作業を続けます。"
elif [ "$HAS_GIT" = true ]; then
    git checkout -b "$BRANCH_NAME"
else
    >&2 echo "[specify] 警告: Gitリポジトリが検出されませんでした。ブランチ $BRANCH_NAME の作成をスキップしました。"
fi

FEATURE_DIR="$SPECS_DIR/$BRANCH_NAME"
mkdir -p "$FEATURE_DIR"

TEMPLATE="$REPO_ROOT/.specify/templates/spec-template.md"
SPEC_FILE="$FEATURE_DIR/spec.md"
if [ -f "$TEMPLATE" ]; then cp "$TEMPLATE" "$SPEC_FILE"; else touch "$SPEC_FILE"; fi

# Set the SPECIFY_FEATURE environment variable for the current session
export SPECIFY_FEATURE="$BRANCH_NAME"

if $JSON_MODE; then
    printf '{"BRANCH_NAME":"%s","SPEC_FILE":"%s","SPEC_ID":"%s"}\n' "$BRANCH_NAME" "$SPEC_FILE" "$SPEC_ID"
else
    echo "ブランチ名: $BRANCH_NAME"
    echo "仕様ファイル: $SPEC_FILE"
    echo "SPEC ID: $SPEC_ID"
    echo "SPECIFY_FEATURE 環境変数を設定: $BRANCH_NAME"
fi
