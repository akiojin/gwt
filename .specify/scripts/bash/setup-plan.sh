#!/usr/bin/env bash

set -e

# コマンドライン引数を解析
JSON_MODE=false
SPEC_ID=""
ARGS=()
i=1

while [ $i -le $# ]; do
    arg="${!i}"
    case "$arg" in
        --json)
            JSON_MODE=true
            ;;
        --spec-id)
            if [ $((i + 1)) -gt $# ]; then
                echo 'エラー: --spec-id には値が必要です' >&2
                exit 1
            fi
            i=$((i + 1))
            next_arg="${!i}"
            if [[ "$next_arg" == --* ]]; then
                echo 'エラー: --spec-id には値が必要です' >&2
                exit 1
            fi
            SPEC_ID="$next_arg"
            ;;
        --help|-h)
            echo "使い方: $0 [--json] [--spec-id <id>]"
            echo "  --json           結果をJSON形式で出力"
            echo "  --spec-id <id>   SPEC IDを明示的に指定（例: SPEC-12345678）"
            echo "  --help           このヘルプメッセージを表示"
            exit 0
            ;;
        *)
            ARGS+=("$arg")
            ;;
    esac
    i=$((i + 1))
done

# SPEC_IDが指定された場合はSPECIFY_FEATURE環境変数に設定
if [[ -n "$SPEC_ID" ]]; then
    export SPECIFY_FEATURE="$SPEC_ID"
fi

# スクリプトディレクトリを取得し、共通関数を読み込む
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

# 共通関数からすべてのパスと変数を取得
eval $(get_feature_paths)

# 適切な機能ブランチ上にいるかチェック（gitリポジトリのみ）
check_feature_branch "$CURRENT_BRANCH" "$HAS_GIT" || exit 1

# 機能ディレクトリが存在することを確認
mkdir -p "$FEATURE_DIR"

# planテンプレートが存在する場合はコピー
TEMPLATE="$REPO_ROOT/.specify/templates/plan-template.md"
if [[ -f "$TEMPLATE" ]]; then
    cp "$TEMPLATE" "$IMPL_PLAN"
    echo "planテンプレートを $IMPL_PLAN にコピーしました"
else
    echo "警告: $TEMPLATE にplanテンプレートが見つかりません"
    # テンプレートが存在しない場合は基本的なplanファイルを作成
    touch "$IMPL_PLAN"
fi

# 結果を出力
if $JSON_MODE; then
    printf '{"FEATURE_SPEC":"%s","IMPL_PLAN":"%s","SPECS_DIR":"%s","BRANCH":"%s","HAS_GIT":"%s"}\n' \
        "$FEATURE_SPEC" "$IMPL_PLAN" "$FEATURE_DIR" "$CURRENT_BRANCH" "$HAS_GIT"
else
    echo "機能仕様: $FEATURE_SPEC"
    echo "実装計画: $IMPL_PLAN"
    echo "仕様ディレクトリ: $FEATURE_DIR"
    echo "ブランチ: $CURRENT_BRANCH"
    echo "Git使用: $HAS_GIT"
fi
