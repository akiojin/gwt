# 実装計画: エージェント履歴の永続化

**仕様ID**: `SPEC-2ca73d7d` | **日付**: 2026-01-22 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-2ca73d7d/spec.md` からの機能仕様

## 概要

gwtは、エージェント起動時にブランチとエージェントの関連情報を永続化し、Worktreeが削除された後もブランチ一覧画面で「直近使用したエージェント」を表示できるようにする。履歴は`~/.config/gwt/agent-history.json`に保存され、リポジトリパスをキーとした構造で管理される。

## 技術コンテキスト

**言語/バージョン**: Rust 2021 Edition (stable)
**主要な依存関係**: ratatui 0.29, crossterm 0.28, serde, serde_json, chrono, dirs
**ストレージ**: ファイルシステム (`~/.config/gwt/agent-history.json`)
**テスト**: cargo test
**ターゲットプラットフォーム**: Linux, macOS, Windows
**プロジェクトタイプ**: 単一プロジェクト（Cargoワークスペース）
**パフォーマンス目標**: 履歴読み込み500ms以内（1000ブランチ時）、書き込み100ms以内
**制約**: `~/.config/`への書き込み権限が必要

## 原則チェック

- シンプルさの追求: 単一JSONファイルで永続化、複雑なDB不要
- ユーザビリティ優先: 自動記録で開発者の操作を増やさない
- 既存ファイル活用: gwt-coreに履歴モジュールを追加

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-2ca73d7d/
├── spec.md              # 機能仕様
├── plan.md              # このファイル
└── tasks.md             # 実装タスク
```

### ソースコード（リポジトリルート）

```text
crates/
├── gwt-core/
│   └── src/
│       └── ai/
│           ├── agent_history.rs   # 履歴永続化モジュール（新規）
│           └── mod.rs             # モジュール登録
├── gwt-cli/
│   └── src/
│       └── tui/
│           ├── app.rs             # エージェント起動時の履歴記録
│           └── screens/
│               └── branch_list.rs # 履歴からのエージェント表示
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存のコードパターンを理解し、履歴機能の統合ポイントを特定する

### 調査項目

1. **既存のコードベース分析**
   - エージェント情報の取得: `crates/gwt-core/src/ai/session_parser/` でClaude Code等のセッション解析
   - 設定ファイル管理: `crates/gwt-core/src/config/settings.rs` でのTOML設定読み書き
   - ブランチ一覧表示: `crates/gwt-cli/src/tui/screens/branch_list.rs`

2. **技術的決定**
   - JSON形式採用: serde_jsonで容易にシリアライズ/デシリアライズ可能
   - dirsクレート: クロスプラットフォームでホームディレクトリ取得
   - chronoクレート: ISO 8601形式の日時管理

3. **制約と依存関係**
   - 既存のserde/serde_json/chrono/dirsは依存済み
   - ファイルパーミッション: Rust標準の`fs::set_permissions`で対応

## フェーズ1: 設計（アーキテクチャと契約）

### 1.1 データモデル設計

**構造体定義**:

- `AgentHistoryEntry`: ブランチごとの履歴エントリ
  - `agent_id: String` - エージェント識別子
  - `agent_label: String` - 表示用ラベル
  - `updated_at: DateTime<Utc>` - 最終更新日時

- `AgentHistoryStore`: リポジトリごとの履歴ストア
  - `repos: HashMap<String, HashMap<String, AgentHistoryEntry>>` - リポジトリパス→ブランチ→エントリ

**JSON構造**:

```json
{
  "repos": {
    "/absolute/path/to/repo": {
      "branches": {
        "branch-name": {
          "agent_id": "claude-code",
          "agent_label": "Claude@latest",
          "updated_at": "2026-01-22T10:30:00Z"
        }
      }
    }
  }
}
```

### 1.2 APIインターフェース

**gwt-core側**:

- `AgentHistoryStore::load() -> Result<Self>` - 履歴ファイル読み込み
- `AgentHistoryStore::save(&self) -> Result<()>` - 履歴ファイル書き込み
- `AgentHistoryStore::record(repo: &Path, branch: &str, agent_id: &str, agent_label: &str) -> Result<()>` - 履歴記録
- `AgentHistoryStore::get(repo: &Path, branch: &str) -> Option<&AgentHistoryEntry>` - 履歴取得
- `AgentHistoryStore::get_all_for_repo(repo: &Path) -> HashMap<String, &AgentHistoryEntry>` - リポジトリ内全履歴取得

**gwt-cli側**:

- エージェント起動時: `AgentHistoryStore::record()` を呼び出し
- ブランチ一覧表示時: `AgentHistoryStore::get_all_for_repo()` で履歴取得、実行中エージェントがなければ履歴を表示

## フェーズ2: タスク生成

**次のステップ**: `tasks.md` を作成

**入力**: このプラン + 仕様書

**出力**: `specs/SPEC-2ca73d7d/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装：

1. **P1**: US2 - エージェント起動時の自動記録（基盤）
2. **P1**: US1 - Worktreeなしブランチでの履歴表示（メイン機能）
3. **P2**: US3 - 複数リポジトリの履歴管理（拡張機能）

### 独立したデリバリー

- US2完了 → 履歴が記録される（見えないが蓄積）
- US1完了 → Worktreeなしブランチでエージェント表示（MVP）
- US3完了 → 複数リポジトリ対応（完全版）

## テスト戦略

- **ユニットテスト**: AgentHistoryStoreの読み書き、エントリ更新ロジック
- **統合テスト**: エージェント起動→履歴記録→ブランチ一覧表示の一連フロー
- **エッジケーステスト**: ファイル破損時、ディスク容量不足時、特殊文字ブランチ名

## リスクと緩和策

### 技術的リスク

1. **履歴ファイル破損**
   - **緩和策**: 読み込み失敗時は空の履歴として扱い、ログ記録して継続

2. **ディスク書き込み失敗**
   - **緩和策**: エージェント起動は継続、書き込み失敗はログ記録のみ

### 依存関係リスク

1. **dirsクレートの非対応プラットフォーム**
   - **緩和策**: Linux/macOS/Windowsのみサポート、その他は環境変数でパス指定可能に

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定
2. ✅ フェーズ1完了: 設計とアーキテクチャ定義
3. ⏭️ `tasks.md` を作成してタスクを生成
4. ⏭️ TDDでテストを先行実装
5. ⏭️ 実装を開始
