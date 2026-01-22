# タスク: エージェント履歴の永続化

**仕様ID**: `SPEC-2ca73d7d`
**入力**: `/specs/SPEC-2ca73d7d/` からの設計ドキュメント
**前提条件**: plan.md（必須）、spec.md（必須）
**ポリシー**: CLAUDE.md の TDD ルールに基づき、必ず RED→GREEN→リグレッションチェックの順に進める

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（US1、US2、US3）

## フェーズ1: セットアップ（共有インフラストラクチャ）

**目的**: 履歴永続化の基盤モジュールを作成

### セットアップタスク

- [ ] **T001** [P] [共通] `crates/gwt-core/src/ai/agent_history.rs` に空のモジュールファイルを作成
- [ ] **T002** [P] [共通] `crates/gwt-core/src/ai/mod.rs` に `agent_history` モジュールを登録

## フェーズ2: ユーザーストーリー2 - エージェント起動時の自動記録 (優先度: P1)

**ストーリー**: 開発者がgwtからエージェントを起動すると、ブランチ名とエージェント情報が自動的に永続化される

**価値**: 明示的な操作なしに履歴が蓄積される基盤機能

### データ層

- [ ] **T101** [Test] [US2] `crates/gwt-core/src/ai/agent_history.rs` に `AgentHistoryEntry` 構造体のシリアライズ/デシリアライズテストを追加
- [ ] **T102** [Test] [US2] `crates/gwt-core/src/ai/agent_history.rs` に `AgentHistoryStore` のJSON読み書きテストを追加
- [ ] **T103** [Impl] [US2] T101の後に `crates/gwt-core/src/ai/agent_history.rs` に `AgentHistoryEntry` 構造体を実装
- [ ] **T104** [Impl] [US2] T102の後に `crates/gwt-core/src/ai/agent_history.rs` に `AgentHistoryStore` 構造体を実装

### ファイル操作

- [ ] **T105** [Test] [US2] `crates/gwt-core/src/ai/agent_history.rs` に履歴ファイルパス取得（`~/.config/gwt/agent-history.json`）のテストを追加
- [ ] **T106** [Test] [US2] `crates/gwt-core/src/ai/agent_history.rs` にディレクトリ自動作成のテストを追加
- [ ] **T107** [Test] [US2] `crates/gwt-core/src/ai/agent_history.rs` にファイルパーミッション（600）設定のテストを追加
- [ ] **T108** [Impl] [US2] T105-T107の後に `crates/gwt-core/src/ai/agent_history.rs` に `get_history_path()` とファイル操作を実装

### 履歴記録API

- [ ] **T109** [Test] [US2] `crates/gwt-core/src/ai/agent_history.rs` に `record()` メソッドのテストを追加（新規エントリ作成）
- [ ] **T110** [Test] [US2] `crates/gwt-core/src/ai/agent_history.rs` に `record()` メソッドのテストを追加（既存エントリ上書き）
- [ ] **T111** [Impl] [US2] T109-T110の後に `crates/gwt-core/src/ai/agent_history.rs` に `record()` メソッドを実装

### CLI統合

- [ ] **T112** [Test] [US2] `crates/gwt-cli/src/tui/app.rs` にエージェント起動時の履歴記録呼び出しテストを追加
- [ ] **T113** [Impl] [US2] T112の後に `crates/gwt-cli/src/tui/app.rs` のエージェント起動処理に履歴記録を追加

### 耐障害性

- [ ] **T114** [Test] [US2] `crates/gwt-core/src/ai/agent_history.rs` に破損ファイル読み込み時の空履歴フォールバックテストを追加
- [ ] **T115** [Test] [US2] `crates/gwt-core/src/ai/agent_history.rs` に書き込み失敗時のエラーログ記録テストを追加
- [ ] **T116** [Impl] [US2] T114-T115の後に `crates/gwt-core/src/ai/agent_history.rs` に耐障害性ロジックを実装

**✅ MVP1チェックポイント**: US2完了後、エージェント起動時に履歴が蓄積される

## フェーズ3: ユーザーストーリー1 - Worktreeなしブランチでの履歴表示 (優先度: P1)

**ストーリー**: Worktreeを削除した後も、そのブランチで最後に使用したエージェント名がブランチ一覧に表示される

**価値**: 過去の作業履歴を確認でき、作業再開時の判断に有用

### 履歴取得API

- [ ] **T201** [Test] [US1] `crates/gwt-core/src/ai/agent_history.rs` に `get()` メソッドのテストを追加（存在するエントリ）
- [ ] **T202** [Test] [US1] `crates/gwt-core/src/ai/agent_history.rs` に `get()` メソッドのテストを追加（存在しないエントリ）
- [ ] **T203** [Test] [US1] `crates/gwt-core/src/ai/agent_history.rs` に `get_all_for_repo()` メソッドのテストを追加
- [ ] **T204** [Impl] [US1] T201-T203の後に `crates/gwt-core/src/ai/agent_history.rs` に `get()` と `get_all_for_repo()` を実装

### ブランチ一覧表示統合

- [ ] **T205** [Test] [US1] `crates/gwt-cli/src/tui/screens/branch_list.rs` に履歴からエージェント情報を表示するテストを追加（Worktreeなし）
- [ ] **T206** [Test] [US1] `crates/gwt-cli/src/tui/screens/branch_list.rs` に実行中エージェントが履歴より優先されるテストを追加
- [ ] **T207** [Test] [US1] `crates/gwt-cli/src/tui/screens/branch_list.rs` に履歴なしブランチでエージェント情報が空白になるテストを追加
- [ ] **T208** [Impl] [US1] T205-T207の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` に履歴表示ロジックを実装
- [ ] **T209** [Impl] [US1] `crates/gwt-cli/src/tui/app.rs` に起動時の履歴読み込みを追加

**✅ MVP2チェックポイント**: US1完了後、Worktreeなしブランチでもエージェント表示可能

## フェーズ4: ユーザーストーリー3 - 複数リポジトリの履歴管理 (優先度: P2)

**ストーリー**: 複数のリポジトリでgwtを使用する場合、各リポジトリの履歴は独立して管理される

**価値**: 異なるリポジトリの履歴が混在しない

### リポジトリ分離

- [ ] **T301** [Test] [US3] `crates/gwt-core/src/ai/agent_history.rs` に異なるリポジトリの履歴が分離されることを確認するテストを追加
- [ ] **T302** [Test] [US3] `crates/gwt-core/src/ai/agent_history.rs` にリポジトリAの履歴がリポジトリBに影響しないテストを追加
- [ ] **T303** [Impl] [US3] T301-T302の後にリポジトリパスをキーとした分離ロジックを検証・調整

**✅ 完全な機能**: US3完了後、複数リポジトリ対応完了

## フェーズ5: 統合とポリッシュ

**目的**: 全ストーリーを統合し、プロダクション準備を整える

### 統合

- [ ] **T401** [統合] エンドツーエンドの統合テストを実行（エージェント起動→履歴記録→Worktree削除→履歴表示）
- [ ] **T402** [統合] エッジケース処理を検証（特殊文字ブランチ名、パス変更など）
- [ ] **T403** [統合] `cargo test` と `cargo clippy --all-targets --all-features -- -D warnings` を実行し、失敗があれば修正
- [ ] **T404** [統合] `cargo fmt` でフォーマットを確認

### 検証

- [ ] **T405** [検証] パフォーマンス検証: 1000ブランチの履歴読み込みが500ms以内であることを確認
- [ ] **T406** [検証] 履歴ファイル破損時もブランチ一覧画面が正常に表示されることを確認

## タスク凡例

**優先度**:

- **P1**: 最も重要 - MVP1/MVP2に必要
- **P2**: 重要 - 完全な機能に必要

**依存関係**:

- **[P]**: 並列実行可能
- **[Test]**: テスト先行（TDD）
- **[Impl]**: 実装

**ストーリータグ**:

- **[US1]**: ユーザーストーリー1（Worktreeなしブランチでの履歴表示）
- **[US2]**: ユーザーストーリー2（エージェント起動時の自動記録）
- **[US3]**: ユーザーストーリー3（複数リポジトリの履歴管理）
- **[共通]**: すべてのストーリーで共有
- **[統合]**: 複数ストーリーにまたがる
- **[検証]**: 検証専用

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## 注記

- 各タスクは1時間から1日で完了可能であるべき
- TDDを厳守: テスト先行で実装
- ファイルパスは正確で、プロジェクト構造と一致させる
