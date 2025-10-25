#!/usr/bin/env bash

set -e

# コマンドライン引数を解析
JSON_MODE=false
ARGS=()

for arg in "$@"; do
    case "$arg" in
        --json)
            JSON_MODE=true
            ;;
        --help|-h)
            echo "使い方: $0 [--json]"
            echo "  --json    結果をJSON形式で出力"
            echo "  --help    このヘルプメッセージを表示"
            exit 0
            ;;
        *)
            ARGS+=("$arg")
            ;;
    esac
done

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
