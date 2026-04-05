# Plan: SPEC-1785 — SPECs画面からのAgent起動

## Summary

SPECs画面（一覧・詳細）からShift+EnterでAgent起動（+ Worktree自動作成）を可能にする。既存WizardStateを拡張し、SPEC固有のステップスキップロジックでブランチ関連ステップを省略。metadata.jsonにブランチ履歴を記録して2回目以降の起動を高速化する。

## Technical Context

### 影響ファイル

| ファイル | 変更内容 |
|---------|---------|
| `crates/gwt-tui/src/screens/specs.rs` | SpecItem拡張、LaunchAgentメッセージ、キーバインド、Phase確認UI、ブランチ選択UI |
| `crates/gwt-tui/src/screens/wizard.rs` | `open_for_spec()`、`from_spec`フラグ、next/prev_step分岐 |
| `crates/gwt-tui/src/app.rs` | LaunchAgentインターセプト、ブランチ検索、metadata書き戻し |

### 既存資産（再利用）

| 資産 | 場所 | 用途 |
|------|------|------|
| `WizardState::open_for_branch()` | `wizard.rs:402` | `open_for_spec()` の設計参考 |
| `AgentLaunchBuilder.auto_worktree()` | `gwt-core/src/agent/launch.rs` | Worktree自動作成 |
| `WorktreeManager` | `gwt-core/src/worktree/manager.rs` | ブランチ→Worktree作成 |
| `load_specs()` | `specs.rs:250` | metadata.json読み込み拡張 |
| `spawn_agent_session()` | `app.rs` | Agent起動処理の再利用 |
| `get_branch_tool_history()` | `gwt-core/src/config/ts_session.rs` | QuickStart履歴取得 |

### Constitution Check

| Rule | Status | Notes |
|------|--------|-------|
| 1. Spec Before Implementation | OK | SPEC-1785 策定済み |
| 2. Test-First Delivery | OK | 各フェーズでTDD実施 |
| 3. No Workaround-First Changes | OK | 根本対応（SPECs画面に操作を追加） |
| 4. Minimal Complexity | OK | 既存Wizard/AgentLaunchBuilderを再利用 |
| 5. Verifiable Completion | OK | ユニットテスト + 手動E2E |
| 6. SPEC vs Issue Separation | OK | SPEC-1785で管理 |

## Phased Implementation

### Phase 1: SpecItem拡張 + metadata連携

1. SpecItemに `branches: Vec<String>` 追加
2. load_specs() で metadata.json の `branches` フィールド読み込み
3. metadata.json書き戻しユーティリティ関数

### Phase 2: キーバインド + LaunchAgentメッセージ

1. SpecsMessage::LaunchAgent 追加
2. handle_key(): Shift+Enter → LaunchAgent（一覧・詳細両方）
3. ヘッダーにキーバインドヒント追加

### Phase 3: Phase確認ダイアログ

1. SpecsState に確認状態フィールド追加
2. draft/blockedでの確認ダイアログUI
3. Y/N キー処理

### Phase 4: ブランチ選択ダイアログ

1. SpecsState にブランチ選択状態フィールド追加
2. 複数候補リスト選択UI
3. 新規作成オプション

### Phase 5: WizardState拡張

1. `open_for_spec()` メソッド追加
2. `from_spec` / `spec_id` フィールド追加
3. next_step() / prev_step() 分岐更新

### Phase 6: app.rs統合

1. LaunchAgentインターセプト処理
2. ブランチ検索ロジック（metadata → git → 新規）
3. Wizard起動 → Agent起動 → metadata書き戻し
4. Agentペイン自動切替

### Phase 7: 統合検証

1. cargo build / test / clippy
2. 手動E2E（SC-001〜SC-007）
