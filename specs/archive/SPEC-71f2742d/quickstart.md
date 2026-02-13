# クイックスタート: カスタムコーディングエージェント登録機能

**仕様ID**: `SPEC-71f2742d` | **作成日**: 2026-01-26

## 1. 開発環境セットアップ

### 1.1 必要な環境

- Rust 2021 Edition (stable)
- Cargo

### 1.2 ビルド

```bash
# リリースビルド
cargo build --release

# デバッグビルド（開発時）
cargo build
```

### 1.3 テスト実行

```bash
# 全テスト
cargo test

# 特定モジュールのテスト
cargo test tools::  # tools.rs のテスト
cargo test wizard:: # wizard.rs のテスト

# 詳細出力
cargo test -- --nocapture
```

### 1.4 Lint / フォーマット

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt
```

## 2. 開発ワークフロー

### 2.1 TDD サイクル

1. `specs/SPEC-71f2742d/tasks.md` のタスクを確認
2. 対応するテストを先に作成
3. テストが失敗することを確認
4. 実装を追加
5. テストがパスすることを確認
6. リファクタリング（必要に応じて）

### 2.2 ファイル編集時の注意

- 新規ファイルより既存ファイルの修正を優先
- gwt-core に追加: `crates/gwt-core/src/config/tools.rs`
- gwt-cli に追加: `crates/gwt-cli/src/tui/screens/settings.rs`

### 2.3 コミットルール

```bash
# コミット前の検証
bunx commitlint --from HEAD~1 --to HEAD

# Conventional Commits 形式
feat(custom-agent): tools.json 読み込み機能を追加
fix(wizard): カスタムエージェント表示順序を修正
```

## 3. 主要ファイルと責務

| ファイル | 責務 |
|----------|------|
| `gwt-core/src/config/tools.rs` | ToolsConfig、CustomCodingAgent 構造体、JSON 読み込み |
| `gwt-core/src/config/mod.rs` | tools モジュール公開 |
| `gwt-cli/src/tui/screens/wizard.rs` | Wizard でのカスタムエージェント表示 |
| `gwt-cli/src/main.rs` | カスタムエージェント起動ロジック |
| `gwt-cli/src/tui/screens/settings.rs` | 設定画面 UI |
| `gwt-cli/src/tui/app.rs` | タブ切り替え、Screen 管理 |

## 4. よくある操作

### 4.1 tools.json のテスト用ファイル作成

```bash
mkdir -p ~/.gwt
cat > ~/.gwt/tools.json << 'EOF'
{
  "version": "1.0.0",
  "customCodingAgents": [
    {
      "id": "test-agent",
      "displayName": "Test Agent",
      "type": "command",
      "command": "echo",
      "defaultArgs": ["Hello from custom agent"]
    }
  ]
}
EOF
```

### 4.2 ローカル tools.json のテスト

```bash
mkdir -p .gwt
cat > .gwt/tools.json << 'EOF'
{
  "version": "1.0.0",
  "customCodingAgents": [
    {
      "id": "local-agent",
      "displayName": "Local Agent",
      "type": "command",
      "command": "ls"
    }
  ]
}
EOF
```

### 4.3 デバッグ実行

```bash
# TUI 起動
cargo run

# ログ出力付き
GWT_DEBUG=true cargo run
```

## 5. トラブルシューティング

### 5.1 カスタムエージェントが表示されない

1. tools.json の構文確認

   ```bash
   cat ~/.gwt/tools.json | jq .
   ```

2. version フィールドの存在確認
3. 必須フィールド（id, displayName, type, command）の確認

### 5.2 エージェントがグレーアウト

- command が PATH に存在するか確認

  ```bash
  which <command>
  ```

### 5.3 テストが失敗する

```bash
# 詳細なテスト出力
cargo test -- --nocapture

# 特定テストのみ実行
cargo test test_tools_config_parse
```

## 6. 参考リンク

- [仕様書](./spec.md)
- [データモデル](./data-model.md)
- [調査レポート](./research.md)
- [プロジェクト原則](../../.specify/memory/constitution.md)
