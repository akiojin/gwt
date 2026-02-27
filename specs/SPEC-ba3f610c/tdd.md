# TDD計画: プロジェクトモード（Project Mode）

**仕様ID**: `SPEC-ba3f610c` | **日付**: 2026-02-27

## テスト戦略

- バックエンド（Rust）: 3層状態モデル、Lead GitHub Issue仕様管理、仕様4セクションゲート、Coordinator管理、Worker完了検出、CI監視ループ、セッション永続化、コンテキスト要約をユニットテストで検証
- フロントエンド（Svelte/TypeScript）: ダッシュボード、Leadチャット、Issue展開、Coordinator詳細、Branch Mode連携をコンポーネントテスト（vitest + @testing-library/svelte）で検証
- 各タスクはテストファーストで進める（RED → 実装 → GREEN）

## RED → GREEN の対象

### Phase 1: セットアップ

1. 3層状態モデル（ProjectModeSession / LeadState / CoordinatorState / WorkerState / ProjectTask）の型定義とenum遷移テスト
2. フロント型定義（ProjectModeState / LeadState / DashboardIssue / DashboardTask / CoordinatorState / WorkerState）の整合テスト
3. PTY通信スキルインターフェースの入出力テスト
4. SessionStore の AppState ワイヤリングと ProjectModeSession 永続化テスト

### Phase 2: 基盤

1. Lead gwt内蔵AI実行基盤（LLM呼び出し・対話ループ）テスト
2. PTY通信スキル化基盤（send_keys_to_pane / capture_scrollback_tail）テスト

### Phase 3: US1 — モード切り替え・基本対話

1. Project Modeタブ・モード切り替えテスト
2. Leadチャット（IME送信抑止 / 送信中スピナー / 自動スクロール）回帰テスト
3. ダッシュボード表示テスト（Issue/Task/Worker階層、ステータスバッジ、折りたたみ）
4. AI設定未構成時エラー表示テスト
5. コスト可視化テスト（APIコール数 / 推定トークン数）
6. Lead段階的委譲テスト（自律可能操作 vs 承認要求操作の分類）
7. Leadハイブリッド常駐テスト（イベント駆動トリガー + 2分間隔ポーリング）

### Phase 4: US2 — GitHub Issue仕様管理・計画承認

1. issue_specツール群（upsert_spec_issue / get_spec_issue等）の動作テスト
2. GitHub Issue仕様管理ワークフロー順次実行テスト（clarify → GitHub Issueにspec/plan/tasks/tdd記録）
3. 仕様4セクションゲートテスト（GitHub Issueのspec/plan/tasks/tdd いずれか欠損 → Coordinator起動ブロック）
4. 計画提示・承認フローテスト（承認 → Coordinator起動 / 拒否 → 再策定）
5. GitHub Issue作成・Project登録テスト
6. Coordinator→Lead ハイブリッド通信テスト（Tauriイベント + scrollback読み取り）
7. Issue展開・Coordinator詳細テスト（ステータス/CI結果/Worker一覧）

### Phase 5: US3 — Worker起動

1. Coordinator起動テスト（GUI内蔵ターミナルペイン、cwd=リポジトリルート、GitHub Issue番号渡し）
2. タスク分割テスト（独立タスク → 別Worktree / 依存タスク → 同一 or merge連携）
3. Worktree/ブランチ自動作成テスト（agent/プレフィックス、命名サニタイズ、連番付与）
4. Worker起動テスト（PTY送信、全自動モードフラグ、CLAUDE.md規約含むプロンプト）
5. TaskクリックでBranch Modeジャンプテスト（Worktree自動遷移）

### Phase 6: US4 — Worker完了検出

1. Worker完了検出テスト（Hook Stop / GWT_TASK_DONE / プロセス終了）
2. Lead途中経過報告テスト（scrollback取得→チャット報告、LLMコール不要）

### Phase 7: US5 — 成果物検証・統合

1. 成果物検証テスト（テスト実行 → パス → PR作成 / 失敗 → リトライ最大3回）
2. CI監視・自律修正ループテスト（CI失敗 → Worker修正 → 再プッシュ、最大3回 → Lead報告）
3. Worker間コンテキスト共有テスト（先行タスクmerge → コンフリクト検出）

### Phase 8: US6 — 障害・独立性

1. 層間独立性テスト（Lead API障害時のCoordinator/Worker続行）
2. Coordinator自律再起動テスト（クラッシュ検出 → 30秒以内再起動 → 状態再取得）

### Phase 9: US7 — セッション

1. セッション永続化テスト（全層状態のJSON保存 / トリガー検証）
2. セッション復元・再開テスト（gwt再起動 → 最新未完了セッション復元）
3. セッション強制中断テスト（Esc → SIGTERM → 5秒タイムアウト → Paused保存）

### Phase 10: US8 — 直接アクセス

1. Coordinator詳細パネル表示テスト（状態 / Worker一覧 / View Terminal / Chat）
2. 直接アクセステスト（Workerターミナル直操作 / Coordinatorチャット入力）

### Phase 11: US9 — コンテキスト管理

1. コンテキスト要約・圧縮テスト（Worker: LLM自動、Lead/Coordinator: gwt側80%閾値制御）

### Phase 12: 仕上げ

1. PTY通信スキル完全移行テスト（agent_tools.rs旧PTYツール呼び出しの廃止確認）
2. issue_specスキル公開テスト（ブランチモード各エージェントからissue_specツール利用可能）
3. ブランチモード連携テスト（agent/ブランチ表示 / 削除時Failed検出）
4. ClaudeプラグインHook転送マニフェスト検証（`hooks.json` に5イベント転送とPreToolUse保護Hook併存）
5. Claudeプラグイン自動登録テスト（起動時`repair_skill_registration` → `setup_gwt_plugin`経路）
6. 手動Hook登録ダイアログ非依存の回帰テスト（GUI起動時にmanual hook setupを要求しない）
7. Skill登録スコープ設定モデルテスト（default_scope + Agent別上書きの永続化）
8. Scope別登録先解決テスト（User/Project/Local + Agent別上書き）
9. Settings Scope UIテスト（選択/保存/再読込/repair-status反映）

## テスト実行コマンド

### バックエンド

- `cargo test -p gwt-core agent -- --nocapture`
- `cargo test -p gwt-tauri agent_master -- --nocapture`
- `cargo test -p gwt-tauri commands::project_mode -- --nocapture`
- `cargo test -p gwt-tauri commands::terminal -- --nocapture`
- `cargo test -p gwt-tauri agent_tools -- --nocapture`
- `cargo test -p gwt-tauri context_summarizer -- --nocapture`
- `cargo test -p gwt-core claude_plugins -- --nocapture`
- `cargo test -p gwt-core claude_hooks -- --nocapture`
- `cargo test -p gwt-core skill_registration -- --nocapture`

### フロントエンド

- `cd gwt-gui && pnpm test src/lib/components/ProjectModePanel.test.ts`
- `cd gwt-gui && pnpm test src/lib/components/LeadChat.test.ts`
- `cd gwt-gui && pnpm test src/lib/components/Dashboard.test.ts`
- `cd gwt-gui && pnpm test src/lib/components/IssueItem.test.ts`
- `cd gwt-gui && pnpm test src/lib/components/CoordinatorDetail.test.ts`
- `cd gwt-gui && pnpm test src/lib/components/SettingsPanel.test.ts`

## 実行ログ

> 以下は実装進行に伴い更新する。

- [ ] Phase 1: セットアップテスト
- [ ] Phase 2: 基盤テスト
- [ ] Phase 3: US1テスト
- [ ] Phase 4: US2テスト
- [ ] Phase 5: US3テスト
- [ ] Phase 6: US4テスト
- [ ] Phase 7: US5テスト
- [ ] Phase 8: US6テスト
- [ ] Phase 9: US7テスト
- [ ] Phase 10: US8テスト
- [ ] Phase 11: US9テスト
- [ ] Phase 12: 仕上げテスト
