---
description: tasks.md に基づいて実装を進めます（ブランチは作成しません）。
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

2. **前提確認（tasks.md 必須）**:
   - リポジトリルートで以下を実行し、JSON をパースして `FEATURE_DIR` / `AVAILABLE_DOCS` / `SPEC_ID` を取得します。
     - `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks --spec-id <SPEC_ID>`

3. **チェックリスト確認（任意）**:
   - `FEATURE_DIR/checklists/` が存在する場合、各ファイルの `- [ ]` / `- [x]` / `- [X]` を集計します。
   - 未完了がある場合は一覧を提示し、実装を続行するかユーザーに確認します（yes 以外は中断）。

4. **コンテキスト読込**:
   - 必須: `tasks.md`, `plan.md`
   - 任意: `spec.md`, `data-model.md`, `contracts/`, `research.md`, `quickstart.md`

5. **実行ポリシー**:
   - フェーズ順にタスクを処理し、完了したら `tasks.md` を `- [x]` に更新します。
   - `[P]` が付いたタスクのみ並列の候補です（同一ファイルを触るタスクは直列）。
   - TDD タスクがある場合は「テスト → 実装」の順を厳守します。
   - 不明点や仕様変更が発生したら `/speckit.clarify` / `/speckit.plan` / `/speckit.tasks` の再実行を提案してください。

6. **完了判定**:
   - 必須タスクが完了していること
   - 実装が `spec.md` の要件と一致すること
   - 計画されたテストが通ること

7. **レポート**:
   - 変更点の要約
   - 残タスク/未完了チェックリスト（あれば）
   - 次の推奨（例: `/speckit.analyze`）
