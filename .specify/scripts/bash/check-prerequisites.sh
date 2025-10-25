#!/usr/bin/env bash

# 統合された前提条件チェックスクリプト
#
# このスクリプトは、仕様駆動開発ワークフローのための統一された前提条件チェックを提供します。
# 以前は複数のスクリプトに分散していた機能を置き換えます。
#
# 使い方: ./check-prerequisites.sh [オプション]
#
# オプション:
#   --json              JSON形式で出力
#   --require-tasks     tasks.mdの存在を要求（実装フェーズ用）
#   --include-tasks     AVAILABLE_DOCSリストにtasks.mdを含める
#   --paths-only        パス変数のみ出力（検証なし）
#   --help, -h          ヘルプメッセージを表示
#
# 出力:
#   JSONモード: {"FEATURE_DIR":"...", "AVAILABLE_DOCS":["..."]}
#   テキストモード: FEATURE_DIR:... \n AVAILABLE_DOCS: \n ✓/✗ file.md
#   パスのみ: REPO_ROOT: ... \n BRANCH: ... \n FEATURE_DIR: ... など

set -e

# コマンドライン引数を解析
JSON_MODE=false
REQUIRE_TASKS=false
INCLUDE_TASKS=false
PATHS_ONLY=false

for arg in "$@"; do
    case "$arg" in
        --json)
            JSON_MODE=true
            ;;
        --require-tasks)
            REQUIRE_TASKS=true
            ;;
        --include-tasks)
            INCLUDE_TASKS=true
            ;;
        --paths-only)
            PATHS_ONLY=true
            ;;
        --help|-h)
            cat << 'EOF'
使い方: check-prerequisites.sh [オプション]

仕様駆動開発ワークフローのための統合された前提条件チェック。

オプション:
  --json              JSON形式で出力
  --require-tasks     tasks.mdの存在を要求（実装フェーズ用）
  --include-tasks     AVAILABLE_DOCSリストにtasks.mdを含める
  --paths-only        パス変数のみ出力（前提条件検証なし）
  --help, -h          このヘルプメッセージを表示

例:
  # タスク前提条件をチェック（plan.md必須）
  ./check-prerequisites.sh --json

  # 実装前提条件をチェック（plan.md + tasks.md必須）
  ./check-prerequisites.sh --json --require-tasks --include-tasks

  # 機能パスのみ取得（検証なし）
  ./check-prerequisites.sh --paths-only

EOF
            exit 0
            ;;
        *)
            echo "エラー: 未知のオプション '$arg'。使用方法については --help を参照してください。" >&2
            exit 1
            ;;
    esac
done

# 共通関数を読み込む
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

# 機能パスを取得してブランチを検証
eval $(get_feature_paths)
check_feature_branch "$CURRENT_BRANCH" "$HAS_GIT" || exit 1

# パスのみモードの場合、パスを出力して終了（JSON + paths-only を組み合わせ可能）
if $PATHS_ONLY; then
    if $JSON_MODE; then
        # 最小限のJSONパスペイロード（検証は実行されない）
        printf '{"REPO_ROOT":"%s","BRANCH":"%s","FEATURE_DIR":"%s","FEATURE_SPEC":"%s","IMPL_PLAN":"%s","TASKS":"%s"}\n' \
            "$REPO_ROOT" "$CURRENT_BRANCH" "$FEATURE_DIR" "$FEATURE_SPEC" "$IMPL_PLAN" "$TASKS"
    else
        echo "リポジトリルート: $REPO_ROOT"
        echo "ブランチ: $CURRENT_BRANCH"
        echo "機能ディレクトリ: $FEATURE_DIR"
        echo "機能仕様: $FEATURE_SPEC"
        echo "実装計画: $IMPL_PLAN"
        echo "タスク: $TASKS"
    fi
    exit 0
fi

# 必要なディレクトリとファイルを検証
if [[ ! -d "$FEATURE_DIR" ]]; then
    echo "エラー: 機能ディレクトリが見つかりません: $FEATURE_DIR" >&2
    echo "最初に /speckit.specify を実行して機能構造を作成してください。" >&2
    exit 1
fi

if [[ ! -f "$IMPL_PLAN" ]]; then
    echo "エラー: $FEATURE_DIR に plan.md が見つかりません" >&2
    echo "最初に /speckit.plan を実行して実装計画を作成してください。" >&2
    exit 1
fi

# 必要な場合はtasks.mdをチェック
if $REQUIRE_TASKS && [[ ! -f "$TASKS" ]]; then
    echo "エラー: $FEATURE_DIR に tasks.md が見つかりません" >&2
    echo "最初に /speckit.tasks を実行してタスクリストを作成してください。" >&2
    exit 1
fi

# 利用可能なドキュメントのリストを構築
docs=()

# これらのオプションドキュメントを常にチェック
[[ -f "$RESEARCH" ]] && docs+=("research.md")
[[ -f "$DATA_MODEL" ]] && docs+=("data-model.md")

# contractsディレクトリをチェック（存在し、ファイルがある場合のみ）
if [[ -d "$CONTRACTS_DIR" ]] && [[ -n "$(ls -A "$CONTRACTS_DIR" 2>/dev/null)" ]]; then
    docs+=("contracts/")
fi

[[ -f "$QUICKSTART" ]] && docs+=("quickstart.md")

# 要求された場合、tasks.mdが存在すれば含める
if $INCLUDE_TASKS && [[ -f "$TASKS" ]]; then
    docs+=("tasks.md")
fi

# 結果を出力
if $JSON_MODE; then
    # ドキュメントのJSON配列を構築
    if [[ ${#docs[@]} -eq 0 ]]; then
        json_docs="[]"
    else
        json_docs=$(printf '"%s",' "${docs[@]}")
        json_docs="[${json_docs%,}]"
    fi

    printf '{"FEATURE_DIR":"%s","AVAILABLE_DOCS":%s}\n' "$FEATURE_DIR" "$json_docs"
else
    # テキスト出力
    echo "機能ディレクトリ: $FEATURE_DIR"
    echo "利用可能なドキュメント:"

    # 各潜在的ドキュメントのステータスを表示
    check_file "$RESEARCH" "research.md"
    check_file "$DATA_MODEL" "data-model.md"
    check_dir "$CONTRACTS_DIR" "contracts/"
    check_file "$QUICKSTART" "quickstart.md"

    if $INCLUDE_TASKS; then
        check_file "$TASKS" "tasks.md"
    fi
fi
