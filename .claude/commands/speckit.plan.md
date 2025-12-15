---
description: 計画テンプレートを使用して実装計画ワークフローを実行し、設計成果物を生成します。
---

## ユーザー入力

```text
$ARGUMENTS
```

追加の指示があれば必ず反映します。

## 手順概要

1. **対象SPEC IDの決定（必須）**:
   - ユーザー入力に `SPEC-[a-f0-9]{8}` が含まれる場合、それを対象SPEC IDとします。
   - 含まれない場合は `specs/specs.md` を提示し、ユーザーに対象SPEC IDを選んでもらってください（このコマンドはそこで停止）。

2. **初期化**:
   - リポジトリルートで以下を実行し、JSON をパースして `FEATURE_SPEC` / `IMPL_PLAN` / `FEATURE_DIR` / `SPEC_ID` を取得します。
     - `.specify/scripts/bash/setup-plan.sh --json --spec-id <SPEC_ID>`

3. **コンテキスト読込**:
   - `FEATURE_SPEC` と `.specify/memory/constitution.md` を読み込みます。
   - `IMPL_PLAN`（テンプレートがコピー済み）を開きます。

4. **計画作成フロー**:
   - `plan.md` の **技術コンテキスト** を埋め、未確定事項は `要確認` と明記します。
   - **原則チェック** を憲章（constitution）から反映し、違反が残る場合はエラーで停止します。
   - **フェーズ0**: `research.md` を生成し、`要確認` を解消します。
   - **フェーズ1**: `data-model.md` / `contracts/` / `quickstart.md` を生成します。
   - **フェーズ1の最後**: `.specify/scripts/bash/update-agent-context.sh --spec-id <SPEC_ID> claude` を実行し、エージェントコンテキストを更新します（手動追記は保持）。
   - **フェーズ2**: リスク・依存関係・マイルストーンを整理し、原則チェックを再評価します。

5. **レポート**:
   - `SPEC_ID`、`FEATURE_DIR`、`IMPL_PLAN` のパスと、生成した成果物（`research.md` / `data-model.md` / `contracts/` / `quickstart.md`）を報告します。

## 重要ルール

- 生成したファイルは必ず `specs/<SPEC_ID>/` 配下に保存します。
- `要確認` が残ったまま次フェーズへ進めません（解消してから進みます）。
- 実装の詳細（コード・具体API等）は `plan.md` に寄せ、`spec.md` は要件中心に保ちます。
