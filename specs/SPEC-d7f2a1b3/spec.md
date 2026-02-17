# バグ修正仕様: Cleanup「Select All Safe」が機能しない

**仕様ID**: `SPEC-d7f2a1b3`
**作成日**: 2026-02-13
**ステータス**: 実装完了
**カテゴリ**: GUI / Worktree Management
**依存仕様**:

- SPEC-c4e8f210（Worktree Cleanup GUI）

**入力**: ユーザー説明: "CleanupモーダルのSelect All Safeボタンを押しても安全なブランチが選択されない"

## 背景

- SPEC-c4e8f210 で実装された Cleanup 機能において、Rust バックエンドの `WorktreeInfo` 構造体に `#[serde(rename_all = "camelCase")]` が付与されていた
- これにより JSON シリアライズ時にフィールド名が camelCase（`safetyLevel`, `hasChanges` 等）で出力される
- フロントエンド（TypeScript 型定義・全コンポーネント）は snake_case（`safety_level`, `has_changes` 等）でアクセスしている
- ランタイムで全ての複合語フィールドが `undefined` となり、以下の機能が全て壊れていた:
  1. Select All Safe ボタン（`safety_level === "safe"` が常に false）
  2. 安全性ドットの色（全てデフォルトの灰色）
  3. 保護/現在ブランチの無効化
  4. M/U 変更マーカー
  5. Gone バッジ
  6. ツール使用ラベル
  7. 安全性によるソート

## 根本原因

`crates/gwt-tauri/src/commands/cleanup.rs`:

- `WorktreeInfo`（L24）: `#[serde(rename_all = "camelCase")]` → JSON キーが camelCase
- `WorktreesChangedPayload`（L67）: 同上 → `project_path` が `projectPath` に

TypeScript 側は全て snake_case で定義（`gwt-gui/src/lib/types.ts:265-281`）。

テストが通っていた理由: フロントエンドのテストフィクスチャが snake_case で手書きされており、Rust のシリアライズ結果を経由していないため。

## ユーザーシナリオとテスト

### ユーザーストーリー 1 - Select All Safe が正常に動作する（優先度: P0）

開発者として、Cleanup モーダルで Select All Safe を押したとき、安全なブランチが正しく選択されることを期待する。

**受け入れシナリオ**:

1. **前提条件** `WorktreeInfo` が Rust からシリアライズされる、**操作** JSON を検査する、**期待結果** `safety_level`, `has_changes`, `has_unpushed`, `is_gone`, `last_tool_usage` 等のフィールドが snake_case で出力される
2. **前提条件** `WorktreesChangedPayload` が Rust からシリアライズされる、**操作** JSON を検査する、**期待結果** `project_path` が snake_case で出力される
3. **前提条件** 安全な Worktree が存在する、**操作** Select All Safe をクリック、**期待結果** `safety_level === "safe"` のブランチにチェックが入る

## 要件

### 機能要件

- **FR-600**: `WorktreeInfo` の JSON シリアライズはフィールド名を snake_case で出力しなければ**ならない**（TypeScript 型定義との整合性）
- **FR-601**: `WorktreesChangedPayload` の JSON シリアライズはフィールド名を snake_case で出力しなければ**ならない**
- **FR-602**: `SafetyLevel` enum のシリアライズは lowercase（`"safe"`, `"warning"`, `"danger"`, `"disabled"`）を維持しなければ**ならない**

## 修正内容

- `WorktreeInfo` から `#[serde(rename_all = "camelCase")]` を削除
- `WorktreesChangedPayload` から `#[serde(rename_all = "camelCase")]` を削除
- serde のデフォルト動作（Rust フィールド名をそのまま使用 = snake_case）により TypeScript 側と一致

## 成功基準

- **SC-001**: `WorktreeInfo` のシリアライズ結果が snake_case フィールド名を含むことをユニットテストで検証
- **SC-002**: `WorktreesChangedPayload` のシリアライズ結果が snake_case フィールド名を含むことをユニットテストで検証
- **SC-003**: `cargo test`, `cargo clippy`, `npx vitest run` が全てパス
