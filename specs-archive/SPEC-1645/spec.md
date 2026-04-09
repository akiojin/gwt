# 設定画面と設定カテゴリ構成

> **Canonical Boundary**: 本 SPEC は Settings 画面のカテゴリ構成と編集導線の正本である。Voice は `SPEC-1778`、Custom Agent は `SPEC-1779`、Docker launch target は `SPEC-1642` が担当する。

## Background

- gwt-tui の Settings タブは、General / Worktree / Agent / Custom / Env / AI のカテゴリを持つ。
- 既存の SPEC-1645 は Studio 時代の設定項目をそのまま引き継いでおり、現在のカテゴリ構成とズレている。
- 本 SPEC は Settings 画面の情報設計と編集フローに限定し、個別機能の詳細仕様は子 SPEC に委譲する。

## User Stories

### US-1: 設定カテゴリを横断する

開発者として、Settings タブでカテゴリを切り替え、必要な設定群へすぐ到達したい。

### US-2: 設定値を編集する

開発者として、General / Worktree / Agent / AI などの設定を画面内で編集し、その場で反映を確認したい。

### US-3: 専用編集フローへ遷移する

開発者として、Custom Agent や Environment Profile のような専用フォームを Settings 内で完結して操作したい。

## Acceptance Scenarios

1. Settings タブでカテゴリタブを移動すると、該当カテゴリの項目だけが表示される。
2. General / Worktree / Agent の設定値は一覧形式で編集できる。
3. Custom / Env / AI では専用フォームまたは専用ペインで編集できる。
4. 変更は設定ストレージへ保存され、再起動後も復元される。
5. キーボードショートカット一覧や操作ヒントはヘルプ導線と矛盾しない。

## Edge Cases

- 保存に失敗しても編集内容が失われず、再試行できる。
- 不正な AI endpoint や profile 定義を保存しようとした場合に validation error を返す。
- カテゴリ数が増えてもタブ移動と選択状態が壊れない。

## Functional Requirements

- FR-001: Settings 画面は General / Worktree / Agent / Custom / Env / AI のカテゴリを提供する。
- FR-002: 一覧編集とフォーム編集をカテゴリごとに切り替えられる。
- FR-003: 設定変更は gwt-core の設定ストレージへ永続化する。
- FR-004: 子 SPEC が持つ詳細項目は、本 SPEC ではカテゴリ導線だけを定義する。
- FR-005: キーボード操作だけでカテゴリ移動・項目選択・保存まで完結できる。

## Success Criteria

- 現行 Settings タブのカテゴリ構成をこの SPEC だけで説明できる。
- 個別機能仕様との重複が解消される。
- 設定編集導線が README / Help / 実装と一致する。
