# クイックスタートガイド: ブランチサマリーパネル（セッション要約対応）

**仕様ID**: `SPEC-4b893dae` | **日付**: 2026-01-19 | **更新日**: 2026-01-19

## 概要

ブランチサマリーパネル機能の開発を開始するためのガイド。
タブ切り替えでブランチ詳細とセッション要約を表示し、4種類のエージェントセッションをAIで要約する。

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

| ファイル                                        | 変更内容                     |
| ----------------------------------------------- | ---------------------------- |
| `crates/gwt-cli/src/tui/screens/branch_list.rs` | パネルUI追加                 |
| `crates/gwt-core/src/git/repository.rs`         | コミットログ・diff統計追加   |
| `crates/gwt-core/src/config/profile.rs`         | AI設定追加                   |

### 新規作成

| ファイル                                             | 内容                                 |
| ---------------------------------------------------- | ------------------------------------ |
| `crates/gwt-core/src/git/commit.rs`                  | CommitEntry, ChangeStats, BranchMeta |
| `crates/gwt-core/src/ai/mod.rs`                      | AIモジュール                         |
| `crates/gwt-core/src/ai/client.rs`                   | OpenAI互換APIクライアント            |
| `crates/gwt-core/src/ai/summary.rs`                  | サマリー生成・キャッシュ             |
| `crates/gwt-core/src/ai/session_parser/mod.rs`       | SessionParserトレイト定義            |
| `crates/gwt-core/src/ai/session_parser/claude.rs`    | Claude Code用パーサー                |
| `crates/gwt-core/src/ai/session_parser/codex.rs`     | Codex CLI用パーサー                  |
| `crates/gwt-core/src/ai/session_parser/gemini.rs`    | Gemini CLI用パーサー                 |
| `crates/gwt-core/src/ai/session_parser/opencode.rs`  | OpenCode用パーサー                   |
| `crates/gwt-cli/src/tui/components/summary_panel.rs` | パネルコンポーネント                 |

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

5. **Phase 5**: タブ切り替え + セッション要約タブ
   - `DetailPanelTab` enum追加（Details, Session）
   - Tabキーハンドラー追加
   - セッション要約タブUI実装

6. **Phase 6**: セッションパーサー（Claude Code優先）
   - `SessionParser` トレイト定義
   - `ClaudeSessionParser` 実装
   - JSONL形式解析

7. **Phase 7**: 残り3エージェント対応
   - `CodexSessionParser` 実装
   - `GeminiSessionParser` 実装
   - `OpenCodeSessionParser` 実装

8. **Phase 8**: ポーリング更新
   - バックグラウンドスレッド実装
   - 30秒間隔チェック
   - ファイル変更検出

## セッションパーサー追加ガイド

### 新しいエージェントを追加する場合

1. `crates/gwt-core/src/ai/session_parser/` に新しいファイルを作成

2. `SessionParser` トレイトを実装:

```rust
use super::{AgentType, ParsedSession, SessionParseError, SessionParser};

pub struct NewAgentSessionParser {
    home_dir: PathBuf,
}

impl SessionParser for NewAgentSessionParser {
    fn parse(&self, session_id: &str) -> Result<ParsedSession, SessionParseError> {
        let path = self.session_file_path(session_id);
        // ファイル読み込みと解析
    }

    fn agent_type(&self) -> AgentType {
        AgentType::NewAgent
    }

    fn session_file_path(&self, session_id: &str) -> PathBuf {
        self.home_dir.join(".newagent/sessions").join(session_id)
    }
}
```

3. `mod.rs` にモジュールを追加:

```rust
mod newagent;
pub use newagent::NewAgentSessionParser;
```

4. `AgentType` enumに新しいバリアントを追加

### セッションファイル形式

| エージェント | 形式  | 場所                                    |
| ------------ | ----- | --------------------------------------- |
| Claude Code  | JSONL | `~/.claude/projects/<hash>/<id>.jsonl`  |
| Codex CLI    | JSONL | `~/.codex/sessions/<id>.jsonl`          |
| Gemini CLI   | JSON  | `~/.gemini/sessions/<id>.json`          |
| OpenCode     | JSON  | `~/.opencode/sessions/<id>.json`        |

### 動的サンプリング

長いセッション（1000ターン以上）では動的サンプリングを適用:

```rust
fn sample_turns(turns: &[Turn]) -> Vec<Turn> {
    let len = turns.len();
    match len {
        0..=100 => turns.to_vec(),           // 全件
        101..=1000 => {                       // 最初50 + 最後50
            let mut result = turns[..50].to_vec();
            result.extend_from_slice(&turns[len-50..]);
            result
        }
        _ => {                                // 最初30 + 中間20 + 最後30
            let mid_start = len / 2 - 10;
            let mut result = turns[..30].to_vec();
            result.extend_from_slice(&turns[mid_start..mid_start+20]);
            result.extend_from_slice(&turns[len-30..]);
            result
        }
    }
}
```

## タブ切り替え実装パターン

### 状態管理

```rust
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum DetailPanelTab {
    #[default]
    Details,
    Session,
}

// BranchListScreenに追加
pub struct BranchListScreen {
    // 既存フィールド...
    detail_panel_tab: DetailPanelTab,
}
```

### キーハンドラー

```rust
fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Tab => {
            self.detail_panel_tab = match self.detail_panel_tab {
                DetailPanelTab::Details => DetailPanelTab::Session,
                DetailPanelTab::Session => DetailPanelTab::Details,
            };
            None
        }
        // 他のキー処理...
    }
}
```

### 描画切り替え

```rust
fn render_detail_panel(&self, frame: &mut Frame, area: Rect) {
    match self.detail_panel_tab {
        DetailPanelTab::Details => self.render_branch_details(frame, area),
        DetailPanelTab::Session => self.render_session_summary(frame, area),
    }
}
```

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

1. APIキーが必要なエンドポイントの場合は設定されているか確認
2. エンドポイントが正しいか確認
3. ネットワーク接続を確認
4. `RUST_LOG=debug` でエラーメッセージを確認

### パネルが表示されない

1. ターミナルの高さが十分か確認（最低15行以上推奨）
2. ブランチが選択されているか確認

### コミットログが空

1. リポジトリにコミットが存在するか確認
2. Worktreeパスが正しいか確認

### セッション要約が表示されない

1. gwtでセッションIDが記録されているか確認（`~/.gwt/ts-session.yaml`）
2. エージェントのセッションファイルが存在するか確認
3. セッションIDがマッチしているか確認
4. `RUST_LOG=debug` でパーサーのエラーを確認

### セッション要約が長くて見切れる

1. セッション要約タブで `PgUp` / `PgDn` を使ってスクロール
2. 端末幅が狭い場合は折り返し表示されるため高さを確保

### タブ切り替えが動作しない

1. Tabキーが他のキーバインドと競合していないか確認
2. パネルにフォーカスがあるか確認

### ポーリングが動作しない

1. セッション要約タブが表示されているか確認（非表示時はポーリング停止）
2. バックグラウンドスレッドがパニックしていないか確認（ログ出力）
