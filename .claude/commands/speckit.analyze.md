---
description: spec.md / plan.md / tasks.md の整合性と品質を分析（読み取り専用）します。
---

## ユーザー入力

```text
$ARGUMENTS
```

追加の指示があれば必ず反映します。

## 前提

- このコマンドは **読み取り専用** です（ファイルを編集しません）。
- `tasks.md` が作成済みであること（通常は `/speckit.tasks` の後）。

## 手順概要

1. **対象SPEC IDの決定（必須）**:
   - ユーザー入力に `SPEC-[a-f0-9]{8}` が含まれる場合、それを対象SPEC IDとします。
   - 含まれない場合は `specs/specs.md` を提示し、ユーザーに対象SPEC IDを選んでもらってください（このコマンドはそこで停止）。

2. **前提確認（JSON）**:
   - リポジトリルートで以下を 1 回だけ実行し、`FEATURE_DIR` と各ファイルパスを確定します。
     - `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks --spec-id <SPEC_ID>`

3. **読込対象**:
   - `spec.md`（要件・ストーリー）
   - `plan.md`（設計・方針）
   - `tasks.md`（実行計画）
   - `.specify/memory/constitution.md`（原則）

4. **分析観点（例）**:
   - 要件 → タスクのカバレッジ（要件が tasks.md に落ちているか）
   - タスク → 要件の逆引き（要件/ストーリーに紐づかないタスクがないか）
   - 矛盾（spec と plan の整合、用語揺れ、順序/依存の破綻）
   - テスト可能性（受け入れ条件が観測可能か）
   - 原則違反（憲章 MUST に反していないか）

5. **レポート出力**:
   - 重要度（CRITICAL/HIGH/MEDIUM/LOW）付きで指摘を列挙します。
   - 修正案は提案しますが、編集は行いません（ユーザーが明示的に許可した後に別コマンドで実施）。
