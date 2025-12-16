---
description: 自然言語の機能説明から、新しい仕様（spec.md）を作成または更新します。
---

## ユーザー入力

```text
$ARGUMENTS
```

`/speckit.specify` 実行時の文章が機能説明です。空でない限り再度尋ねないでください。

## 手順概要

1. **入力検証**:
   - 機能説明が空の場合はエラーで停止します。

2. **SPEC ID 生成（ブランチは作成しない）**:
   - リポジトリルートで以下を 1 回だけ実行し、JSON をパースして `SPEC_ID` と `SPEC_FILE` を取得します。
     - `.specify/scripts/bash/create-new-feature.sh --json "$ARGUMENTS"`
   - 仕様ディレクトリは必ず `specs/SPEC-[a-f0-9]{8}/` 形式です。
   - 仕様一覧 `specs/specs.md` は自動更新されます。

3. **テンプレート読込**:
   - `.specify/templates/spec-template.md` を読み、セクション順序を維持したまま内容を埋めます。

4. **仕様作成ガイドライン**:
   - 「何を、なぜ実現するか」に集中し、実装手段（言語/フレームワーク/具体API等）は書きません。
   - 要件は必ずテスト可能な表現にします（入力・条件・期待結果が観測できる）。
   - 不確定事項は `【要確認: ...】` を最大 3 件まで残します（影響度の高いものを優先）。

5. **spec.md の書き込み**:
   - 生成した仕様を `SPEC_FILE` に保存します。

6. **品質チェックリスト（任意だが推奨）**:
   - `specs/<SPEC_ID>/checklists/requirements.md` を作成し、仕様の品質チェック項目を列挙します。
   - 雛形は `.specify/templates/checklist-template.md` を利用し、サンプル項目は残さず実項目に置き換えます。

7. **出力**:
   - `SPEC_ID`（例: `SPEC-1defd8fd`）
   - `SPEC_FILE`（例: `specs/SPEC-1defd8fd/spec.md`）
   - 次の推奨ステップ（通常は `/speckit.clarify` → `/speckit.plan`）
