# 実装計画: Claude Code プラグインマーケットプレイス自動登録

**仕様ID**: `SPEC-f8dab6e2` | **日付**: 2026-01-30 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-f8dab6e2/spec.md` からの機能仕様

## 概要

gwtでClaude Codeを起動する際、worktree-protection-hooksプラグインのマーケットプレイス登録を自動化する機能を実装する。未登録の場合は確認ダイアログを表示し、ユーザー同意後に`known_marketplaces.json`と`enabledPlugins`を更新する。

## 技術コンテキスト

**言語/バージョン**: Rust 2021 Edition (stable)
**主要な依存関係**: ratatui 0.29, crossterm 0.28, serde, serde_json, dirs
**ストレージ**: ファイル（`~/.claude/plugins/known_marketplaces.json`, `~/.claude/settings.json`, `.claude/settings.json`）
**テスト**: cargo test
**ターゲットプラットフォーム**: Linux, macOS, Windows
**プロジェクトタイプ**: 単一（Rustワークスペース）
**制約**: サイレントエラーハンドリング、既存UI統合

## 原則チェック

- [x] シンプルさの追求: 既存の`claude_hooks.rs`パターンに従う
- [x] ユーザビリティ優先: 確認ダイアログで明確な選択肢を提示
- [x] 既存ファイル優先: `claude_hooks.rs`を拡張、新規ファイル最小化

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-f8dab6e2/
├── spec.md              # 機能仕様
├── plan.md              # このファイル
└── tasks.md             # 実装タスク
```

### ソースコード（リポジトリルート）

```text
crates/gwt-core/src/config/
├── mod.rs
├── claude_hooks.rs      # 既存（gwt hookコマンド登録）
└── claude_plugins.rs    # 新規（プラグイン/マーケットプレイス登録）

crates/gwt-cli/src/tui/screens/
└── confirm.rs           # 既存（確認ダイアログ追加）
```

## フェーズ0: 調査（技術スタック選定）

### 調査項目

1. **既存のコードベース分析**
   - `claude_hooks.rs`: 既存のhook登録パターンを参照
   - `confirm.rs`: 確認ダイアログのパターンを参照
   - `~/.claude/plugins/known_marketplaces.json`: JSON形式の構造を確認済み

2. **技術的決定**
   - **ファイル形式**: JSON（既存形式に準拠）
   - **エラーハンドリング**: サイレント（ユーザー要件）
   - **ダイアログ**: 既存の`ConfirmState`パターンを使用

3. **依存関係**
   - `serde_json`: JSON読み書き（既存依存）
   - `dirs`: ホームディレクトリ取得（既存依存）

## フェーズ1: 設計（アーキテクチャと契約）

### 1.1 データモデル設計

#### KnownMarketplaces構造

```rust
/// マーケットプレイスソース情報
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceSource {
    pub source: String,  // "github"
    pub repo: String,    // "akiojin/gwt"
}

/// マーケットプレイスエントリ
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceEntry {
    pub source: MarketplaceSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
}

/// known_marketplaces.json全体
pub type KnownMarketplaces = HashMap<String, MarketplaceEntry>;
```

#### EnabledPlugins構造（既存）

```rust
/// settings.jsonのenabledPluginsセクション
pub type EnabledPlugins = HashMap<String, bool>;
```

### 1.2 API設計

```rust
// claude_plugins.rs

/// gwt-pluginsマーケットプレイスが登録済みかチェック
pub fn is_gwt_marketplace_registered() -> bool;

/// gwt-pluginsマーケットプレイスを登録
pub fn register_gwt_marketplace() -> Result<(), GwtError>;

/// worktree-protection-hooksプラグインを有効化
pub fn enable_worktree_protection_plugin() -> Result<(), GwtError>;

/// マーケットプレイス登録とプラグイン有効化を一括実行
pub fn setup_gwt_plugin() -> Result<(), GwtError>;
```

### 1.3 確認ダイアログ設計

```rust
// confirm.rs に追加

/// Claude Codeプラグインセットアップ確認ダイアログ
pub fn plugin_setup() -> Self {
    Self {
        title: "Setup Worktree Protection Plugin".to_string(),
        message: "Enable worktree-protection-hooks plugin for Claude Code?".to_string(),
        details: vec![
            "This will register gwt-plugins marketplace.".to_string(),
            "Plugin protects against dangerous operations.".to_string(),
        ],
        confirm_label: "Setup".to_string(),
        cancel_label: "Skip".to_string(),
        selected_confirm: true,
        is_dangerous: false,
        ..Default::default()
    }
}
```

## 実装戦略

### 優先順位付け

1. **P1**: マーケットプレイス登録チェック機能
2. **P1**: マーケットプレイス登録機能
3. **P1**: プラグイン有効化機能
4. **P1**: 確認ダイアログ追加
5. **P2**: Codex判定でのスキップ
6. **P2**: ファイル/ディレクトリ自動作成

### 独立したデリバリー

- ストーリー1完了 → 基本的なプラグイン自動登録が動作
- ストーリー2完了 → 登録済み時のスキップが動作
- ストーリー3完了 → Codex起動時のスキップが動作
- ストーリー4完了 → 新規ユーザー対応が完了

## テスト戦略

### ユニットテスト

- `claude_plugins.rs`の各関数のテスト
- JSON読み書きのテスト
- エラーハンドリングのテスト

### 統合テスト

- 確認ダイアログの表示・操作テスト
- Claude Code起動フローとの統合テスト

### TDD対応

各機能要件（FR-001〜FR-010）に対応するテストを先に作成し、実装を後から行う。

```rust
#[cfg(test)]
mod tests {
    // FR-001: マーケットプレイス登録状態の確認
    #[test]
    fn test_is_gwt_marketplace_registered_when_not_exists();

    #[test]
    fn test_is_gwt_marketplace_registered_when_exists();

    // FR-003: マーケットプレイス登録
    #[test]
    fn test_register_gwt_marketplace_creates_correct_entry();

    // FR-004: プラグイン有効化
    #[test]
    fn test_enable_worktree_protection_plugin_adds_to_enabled_plugins();

    // FR-006: ディレクトリ自動作成
    #[test]
    fn test_register_creates_plugins_directory_if_not_exists();

    // FR-009: サイレントエラー
    #[test]
    fn test_register_silently_continues_on_write_error();

    // FR-010: 無効化されたプラグインの再有効化禁止
    #[test]
    fn test_does_not_reenable_disabled_plugin();
}
```

## リスクと緩和策

### 技術的リスク

1. **Claude Codeのプラグイン仕様変更**
   - **緩和策**: 最小限の設定のみ書き込み、Claude Code側の処理に依存

2. **ファイル競合（Claude Codeが同時に書き込む可能性）**
   - **緩和策**: Claude Code起動前に書き込み完了、追記のみ（既存エントリを削除しない）

### 依存関係リスク

1. **known_marketplaces.jsonの形式変更**
   - **緩和策**: 必須フィールドのみ使用、オプションフィールドはスキップ

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定
2. ✅ フェーズ1完了: 設計とアーキテクチャ定義
3. ⏭️ TDDテストを作成
4. ⏭️ 実装を開始
