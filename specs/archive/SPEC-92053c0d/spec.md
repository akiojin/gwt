# 機能仕様: commitlint を npm ci 無しで実行可能にする

**仕様ID**: `SPEC-92053c0d`
**作成日**: 2026-02-03
**ステータス**: ドラフト
**カテゴリ**: GUI

**入力**: ユーザー説明: "commitlintがnpm ciでなくてもインストールされるようにして下さい。"

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - node_modules が無くても commitlint が実行できる (優先度: P1)

開発者がクリーンなチェックアウトで `bunx commitlint --from HEAD~1 --to HEAD` を実行した際に、`npm ci` を行わなくても commitlint が起動し、コミットメッセージの検証ができる。

**この優先度の理由**: CI/ローカルの検証フローに必須で、依存の事前インストールが不要になるため。

**独立したテスト**: commitlint 設定読み込み時に `@commitlint/config-conventional` と `conventional-changelog-conventionalcommits` が存在しない状況を模擬し、設定が例外なく読み込めることを確認する。

**受け入れシナリオ**:

1. **前提条件** node_modules が無い、**操作** `bunx commitlint --from HEAD~1 --to HEAD` を実行、**期待結果** 設定読み込みエラーなく検証が完了する
2. **前提条件** `@commitlint/config-conventional` が見つからない、**操作** 設定読み込みを実行、**期待結果** フォールバック設定で読み込みが継続する

---

### ユーザーストーリー 2 - 既存の commitlint ルールは維持される (優先度: P2)

開発者が commitlint を実行した際に、従来のルール（type/subject/length など）が維持される。

**この優先度の理由**: リリース判定に依存するため、ルール逸脱を避ける必要がある。

**独立したテスト**: 既存ルール（type-enum、subject-empty、header-max-length など）が設定に含まれていることを検証する。

**受け入れシナリオ**:

1. **前提条件** 既存の commitlint 設定がある、**操作** 設定読み込みを実行、**期待結果** 既存ルールが上書きされずに保持される

---

### エッジケース

- `@commitlint/config-conventional` が無い場合でも設定読み込みが失敗しない
- `conventional-changelog-conventionalcommits` が無い場合、parserPreset が必須にならない

## 要件 *(必須)*

### 機能要件

- **FR-001**: システムは `@commitlint/config-conventional` が無い場合でも commitlint 設定を読み込め**なければならない**
- **FR-002**: `@commitlint/config-conventional` が利用可能な場合は既存設定を優先して読み込め**なければならない**
- **FR-003**: フォールバック設定は従来の commitlint ルール（type/subject/length 等）を維持**しなければならない**
- **FR-004**: `bunx commitlint --from HEAD~1 --to HEAD` が npm ci を前提とせずに動作**しなければならない**
- **FR-005**: `@commitlint/config-conventional` 未導入を模擬する自動テストを追加**しなければならない**

### 主要エンティティ *(機能がデータを含む場合は含める)*

- なし（設定ファイルのみ）

## 成功基準 *(必須)*

### 測定可能な成果

- **SC-001**: node_modules が無い状態でも commitlint 設定が例外なく読み込まれる
- **SC-002**: フォールバック時も type-enum/subject-empty/header-max-length が有効
- **SC-003**: `bunx commitlint --from HEAD~1 --to HEAD` が 1 回で完了する

## 制約と仮定 *(該当する場合)*

### 制約

- 追加のランタイム依存は増やさない
- 既存の commitlint ルールは変更しない

### 仮定

- Node.js 18+ が利用可能

## 範囲外 *(必須)*

次の項目は、この機能の範囲外です：

- commitlint ルール自体の仕様変更
- husky のフック挙動変更

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

- 追加の機密情報は扱わない

## 依存関係 *(該当する場合)*

- commitlint 設定ファイル（commitlint.config.cjs）

## 参考資料 *(該当する場合)*

- [commitlint config conventional](https://github.com/conventional-changelog/commitlint/tree/master/@commitlint/config-conventional)
