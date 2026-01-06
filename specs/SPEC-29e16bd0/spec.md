# 機能仕様: tools.json スキーママイグレーション

**仕様ID**: `SPEC-29e16bd0`
**作成日**: 2026-01-04
**ステータス**: 承認済み
**種別**: バグフィックス

## 背景

コミット `35212a1` で「AI Tool」から「Coding Agent」へ名称変更が行われ、
`tools.json` のスキーマが `customTools` から `customCodingAgents` に変更された。
しかし、既存ユーザーのファイルには旧フィールド名 `customTools` が残っており、
マイグレーションパスが実装されていなかったため起動時にエラーが発生する。

## エラーメッセージ

```text
workflow error: customCodingAgents field must be an array
```

## ユーザーシナリオとテスト

### ユーザーストーリー 1 - 旧フィールド名からの自動マイグレーション (優先度: P0)

既存ユーザーは `~/.gwt/tools.json` に `customTools` フィールドを持つファイルがあっても、
アプリケーションが正常に起動し、設定が読み込まれる。

**受け入れシナリオ**:

1. **前提条件** `tools.json` に `customTools: []` が存在し `customCodingAgents` が存在しない
   **操作** `loadCodingAgentsConfig()` を呼び出す
   **期待結果** `customTools` の値が `customCodingAgents` として読み込まれる

2. **前提条件** `tools.json` に `customCodingAgents` も `customTools` も存在しない
   **操作** `loadCodingAgentsConfig()` を呼び出す
   **期待結果** 空配列 `[]` がフォールバックとして使用される

3. **前提条件** `tools.json` に `customCodingAgents: [...]` が存在する（新形式）
   **操作** `loadCodingAgentsConfig()` を呼び出す
   **期待結果** `customCodingAgents` がそのまま使用される（マイグレーション不要）

## 要件

### 機能要件

- **FR-001**: `loadCodingAgentsConfig()` は `customTools` フィールドが存在し
  `customCodingAgents` が存在しない場合、`customTools` を `customCodingAgents` として扱わなければならない
- **FR-002**: マイグレーション実行時は警告ログを出力しなければならない
- **FR-003**: `customCodingAgents` が undefined/null の場合は空配列にフォールバックしなければならない
- **FR-004**: 新形式のファイル（`customCodingAgents` が存在）では何も変更しない

## 成功基準

- **SC-001**: 旧形式の `tools.json` を持つユーザーがエラーなく起動できる
- **SC-002**: マイグレーションのユニットテストが100%パスする

## 対象ファイル

- `src/config/tools.ts` - `loadCodingAgentsConfig()` 関数

## テスト要件

以下のテストケースを `tests/unit/config/tools.test.ts` に追加：

1. `customTools` のみ存在する場合、`customCodingAgents` として読み込まれる
2. どちらのフィールドも存在しない場合、空配列にフォールバック
3. `customCodingAgents` が存在する場合はマイグレーション不要
4. マイグレーション時に警告ログが出力される
