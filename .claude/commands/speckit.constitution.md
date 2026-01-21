---
description: プロジェクト原則（.specify/memory/constitution.md）を作成・更新します。
---

## ユーザー入力

```text
$ARGUMENTS
```

追加の指示があれば必ず反映します。

## 概要

`.specify/memory/constitution.md` はテンプレートです。
角括弧 `[PLACEHOLDER]` を具体値に置き換え、必要に応じて関連テンプレートとの整合も取ります。

## 実行フロー

1. **現行憲章の読込**:
   - `.specify/memory/constitution.md` を開き、`[ALL_CAPS_IDENTIFIER]` 形式のプレースホルダーを洗い出します。
   - 原則（Principles）の数が増減する必要がある場合は、テンプレート構造を保ちつつ柔軟に調整します。

2. **値の収集**:
   - ユーザー入力があれば最優先で採用します。
   - ない場合は README や既存ドキュメントから推定します。
   - 日付:
     - `RATIFICATION_DATE`: 初回制定日（不明なら `TODO(RATIFICATION_DATE): 理由`）
     - `LAST_AMENDED_DATE`: 変更があれば本日（YYYY-MM-DD）
   - `CONSTITUTION_VERSION` はセマンティックバージョンで増分します。
     - MAJOR: 原則の撤廃/破壊的変更
     - MINOR: 原則やセクションの追加/大幅拡張
     - PATCH: 文言の明確化など軽微な修正

3. **憲章の更新**:
   - すべてのプレースホルダーを実値で置換します（意図的に残す場合は理由をコメント）。
   - MUST/SHOULD を使い分け、曖昧語を避けてテスト可能な表現にします。

4. **整合チェック**（必要に応じて更新）:
   - `.specify/templates/spec-template.md`
   - `.specify/templates/plan-template.md`
   - `.specify/templates/tasks-template.md`

5. **Sync Impact Report**:
   - 憲章の先頭に HTML コメントで以下を記載します。
     - 旧→新バージョン
     - 変更した原則/追加・削除セクション
     - 更新したテンプレート一覧（✅/⚠）
     - TODO/保留事項

6. **書き込み**:
   - 更新内容で `.specify/memory/constitution.md` を上書きします。

## 出力

- 新しい `CONSTITUTION_VERSION` と増分理由
- 更新したファイル一覧
- 次の推奨（例: `/speckit.specify`）
