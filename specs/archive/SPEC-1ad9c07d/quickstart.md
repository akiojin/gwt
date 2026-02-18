<!-- markdownlint-disable MD013 -->
# クイックスタートガイド: AIBranchSuggest

**仕様ID**: `SPEC-1ad9c07d` | **日付**: 2026-02-08

## セットアップ

### 前提条件

- Rust stable toolchain
- AI設定が有効（endpoint + model が設定済み）

### ビルド

```bash
cargo build --release
```

### テスト実行

```bash
cargo test
```

### Lint

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

## 開発ワークフロー

### 1. 変更対象ファイル

| ファイル | 変更内容 |
|---------|---------|
| `crates/gwt-cli/src/tui/screens/wizard.rs` | WizardStep enum追加、WizardState新フィールド、next_step/prev_step更新、render関数、入力ハンドラ |
| `crates/gwt-cli/src/tui/app.rs` | ai_branch_suggest_rx追加、Enter/Esc/charハンドラ更新、apply_updates追加 |
| `crates/gwt-core/src/agent/worktree.rs` | （変更なし、sanitize_branch_name()を使用） |
| `crates/gwt-core/src/ai/client.rs` | （変更なし、create_response()を使用） |

### 2. 実装順序

1. **WizardStep enum** に `AIBranchSuggest` バリアントを追加
2. **AIBranchSuggestPhase** enum を定義
3. **WizardState** に新フィールドを追加
4. **BranchType::from_prefix()** メソッドを追加
5. **next_step()** を更新（IssueSelect → AIBranchSuggest条件分岐）
6. **prev_step()** を更新（AIBranchSuggest → IssueSelect / BranchTypeSelect）
7. **insert_char() / delete_char()** にAIBranchSuggestケースを追加
8. **select_up() / select_down()** にAIBranchSuggestケースを追加
9. **current_step_item_count() / current_selection_index()** を更新
10. **render_ai_branch_suggest_step()** を追加
11. **app.rs** に非同期チャネルとハンドラを追加

### 3. テスト戦略

```bash
# ユニットテスト実行
cargo test --lib

# 特定テスト実行
cargo test test_ai_branch_suggest

# 全テスト + clippy
cargo test && cargo clippy --all-targets --all-features -- -D warnings
```

## よくある操作

### AI設定の確認

AI設定はプロファイル設定で管理される。`~/.gwt/profiles.json` を確認:

```json
{
  "default_ai": {
    "endpoint": "https://api.openai.com/v1",
    "api_key": "sk-...",
    "model": "gpt-4o-mini",
    "summary_enabled": true
  }
}
```

### デバッグ

- `RUST_LOG=debug cargo run` でデバッグログを有効化
- AIリクエスト/レスポンスのログは `tracing` で出力される

## トラブルシューティング

### AIBranchSuggestが表示されない

- AI設定が有効か確認（endpoint + model が必須）
- `active_ai_enabled()` が `true` を返すか確認
- 新規ブランチ作成フローであることを確認（既存ブランチ使用時は表示されない）

### API呼び出しが失敗する

- エンドポイントURLが正しいか確認
- APIキーが有効か確認
- ネットワーク接続を確認
- エラーフェーズでEnterを押すと手動入力にフォールバック

### ブランチ名が正規化されない

- `sanitize_branch_name()` は小文字化、特殊文字除去、ハイフン区切りを行う
- 64文字上限で切り詰め
- プレフィックス部分（`feature/` 等）は正規化の対象外
