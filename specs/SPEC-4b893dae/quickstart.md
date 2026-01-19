# クイックスタートガイド: ブランチサマリーパネル

**仕様ID**: `SPEC-4b893dae` | **日付**: 2026-01-19

## 概要

ブランチサマリーパネル機能の開発を開始するためのガイド。

## 前提条件

- Rust (Stable) がインストール済み
- Git がインストール済み
- OpenAI互換APIへのアクセス（AI機能を使用する場合）

## 開発環境セットアップ

```bash
# リポジトリをクローン（既存の場合はスキップ）
git clone https://github.com/akiojin/gwt.git
cd gwt

# ビルド
cargo build

# テスト実行
cargo test

# 開発実行
cargo run
```

## 関連ファイル

### 変更対象

| ファイル | 変更内容 |
|---------|---------|
| `crates/gwt-cli/src/tui/screens/branch_list.rs` | パネルUI追加 |
| `crates/gwt-core/src/git/repository.rs` | コミットログ・diff統計追加 |
| `crates/gwt-core/src/config/profile.rs` | AI設定追加 |

### 新規作成

| ファイル | 内容 |
|---------|------|
| `crates/gwt-core/src/git/commit.rs` | CommitEntry, ChangeStats, BranchMeta |
| `crates/gwt-core/src/ai/mod.rs` | AIモジュール |
| `crates/gwt-core/src/ai/client.rs` | OpenAI互換APIクライアント |
| `crates/gwt-core/src/ai/summary.rs` | サマリー生成・キャッシュ |
| `crates/gwt-cli/src/tui/components/summary_panel.rs` | パネルコンポーネント |

## AI機能の設定

### 方法1: プロファイル設定

`~/.gwt/profiles.yaml` に追加:

```yaml
profiles:
  default:
    name: default
    env: {}
    ai:
      endpoint: "https://api.openai.com/v1"
      api_key: "sk-..."
      model: "gpt-4o-mini"
```

### 方法2: 環境変数

```bash
export OPENAI_API_KEY="sk-..."
export OPENAI_API_BASE="https://api.openai.com/v1"  # オプション
export OPENAI_MODEL="gpt-4o-mini"  # オプション
```

### ローカルLLM（Ollama等）

```yaml
profiles:
  local:
    name: local
    env: {}
    ai:
      endpoint: "http://localhost:11434/v1"
      api_key: ""
      model: "llama3.2"
```

## テスト実行

```bash
# 全テスト
cargo test

# 特定モジュールのテスト
cargo test --package gwt-core commit
cargo test --package gwt-core ai
cargo test --package gwt-cli summary_panel

# 統合テスト
cargo test --test integration
```

## 開発フロー

1. **Phase 1**: パネル枠 + コミットログ
   - `branch_list.rs` のレイアウト変更（12行固定パネル）
   - `git log --oneline -n 5` のラッパー実装
   - CommitEntry構造体とパーサー

2. **Phase 2**: 変更統計
   - `git diff --shortstat` のラッパー実装
   - ChangeStats構造体とパーサー
   - 既存のhas_changes/has_unpushedと統合

3. **Phase 3**: メタデータ
   - BranchMeta構造体（既存Branch構造体から変換）
   - 相対日時計算

4. **Phase 4**: AI機能
   - AISettings構造体とプロファイル連携
   - OpenAI互換APIクライアント
   - サマリー生成とキャッシュ

## デバッグ

### ログ出力

```bash
RUST_LOG=debug cargo run
```

### TUIデバッグ

`branch_list.rs` でデバッグ情報を一時的にパネルに表示:

```rust
// デバッグ用: パネル内容を確認
let debug_text = format!("commits: {:?}", self.branch_summary);
```

## トラブルシューティング

### AI機能が動作しない

1. APIキーが設定されているか確認
2. エンドポイントが正しいか確認
3. ネットワーク接続を確認
4. `RUST_LOG=debug` でエラーメッセージを確認

### パネルが表示されない

1. ターミナルの高さが十分か確認（最低15行以上推奨）
2. ブランチが選択されているか確認

### コミットログが空

1. リポジトリにコミットが存在するか確認
2. Worktreeパスが正しいか確認
