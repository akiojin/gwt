# タスクリスト: プロジェクトモード（Project Mode）

**仕様ID**: `SPEC-ba3f610c` | **日付**: 2026-02-19

## 完了済み（旧アーキテクチャ: MA + Agentビュー）

> 以下のT001〜T018は旧Master Agent + Agentビュー構成で完了したタスク。
> 3層アーキテクチャへのリファクタリングにあたり、成果物は再利用しつつ新タスクで拡張する。

- [x] T001〜T018: 旧アーキテクチャ実装完了（詳細は git log 参照）

## ストーリー間依存関係

```text
US1 (基本対話)     ← 全ストーリーの前提
US2 (GitHub Issue)  ← US1
US3 (Developer起動) ← US2
US4 (完了検出)      ← US3
US5 (成果物検証)    ← US4
US6 (障害)         ← US3, US4
US7 (セッション)    ← US1
US8 (直接アクセス)  ← US3, US4
US9 (コンテキスト)  ← US1
```

## Phase 1: セットアップ

- [ ] T101 [P] [US1] エンティティモデル定義（ProjectModeSession / LeadState / LeadStatus / LeadMessage） `crates/gwt-core/src/agent/mod.rs` `crates/gwt-core/src/agent/lead.rs`
  - data-model.md の LeadState / LeadStatus / LeadMessage を実装
  - 既存 AgentSession を ProjectModeSession に拡張
  - 依存: なし

- [ ] T102 [P] [US1] エンティティモデル定義（ProjectIssue / IssueStatus / CoordinatorState / CoordinatorStatus） `crates/gwt-core/src/agent/issue.rs` `crates/gwt-core/src/agent/coordinator.rs`
  - data-model.md の ProjectIssue / CoordinatorState を実装
  - 依存: なし

- [ ] T103 [P] [US1] エンティティモデル定義（ProjectTask拡張 / DeveloperState / DeveloperStatus） `crates/gwt-core/src/agent/task.rs` `crates/gwt-core/src/agent/developer.rs`
  - 既存 Task に developers Vec を追加、DeveloperState 新規
  - 依存: なし

- [ ] T104 [P] [US1] フロント型定義を3層対応に拡張 `gwt-gui/src/lib/types.ts`
  - ProjectModeState / LeadState / DashboardIssue / DashboardTask / CoordinatorState / DeveloperState
  - 依存: なし

- [ ] T105 [US1] SessionStore の AppState ワイヤリング `crates/gwt-tauri/src/state.rs` `crates/gwt-core/src/agent/session_store.rs`
  - 既存 SessionStore を ProjectModeSession 対応に拡張
  - AppState に SessionStore を追加し永続化パイプラインを接続
  - 依存: T101, T102, T103

## Phase 2: 基盤（全ストーリー共通）

- [ ] T201 [US1] [テスト] Lead gwt内蔵AI実行基盤テスト `crates/gwt-tauri/src/agent_master.rs`
  - gwt内蔵AIとしてのLLM呼び出し・対話ループ動作検証
  - 依存: T101

- [ ] T202 [US1] Lead gwt内蔵AI実行基盤 `crates/gwt-tauri/src/agent_master.rs`
  - 既存 ReAct ループを拡張、チャットUIで統一的UX
  - 依存: T201

- [ ] T203 [US1] [テスト] PTY通信スキル化基盤テスト `crates/gwt-tauri/src/commands/terminal.rs`
  - send_keys_to_pane / send_keys_broadcast / capture_scrollback_tail のスキルAPI検証
  - 依存: なし

- [ ] T204 [P] [US1] PTY通信スキル化基盤インターフェース定義 `crates/gwt-tauri/src/commands/terminal.rs`
  - スキルAPIの入出力インターフェース定義（既存コマンドのラッパー）
  - 依存: T203

## Phase 3: US1 — モード切り替えと基本対話

> 独立テスト条件: Project Modeタブを開き、テキスト入力してLead応答を受け取れること

- [ ] T301 [US1] [テスト] Project Modeタブ・モード切り替えテスト `gwt-gui/src/lib/components/ProjectModePanel.test.ts`
  - タブ選択→PT画面表示、他タブ→BM復帰、タスク実行中のバックグラウンド継続
  - 依存: T104

- [ ] T302 [US1] Project Modeタブ・モード切り替え実装 `gwt-gui/src/lib/components/ProjectModePanel.svelte`
  - 既存ProjectModePanelを2カラム構成へ拡張し、Project Mode導線を統一
  - 依存: T301

- [ ] T303 [US1] [テスト] Leadチャット（IME/スピナー/自動スクロール）テスト `gwt-gui/src/lib/components/LeadChat.test.ts`
  - IME送信抑止、送信中スピナー、自動スクロール、バブル表示
  - 依存: T104

- [ ] T304 [US1] LeadチャットUI実装 `gwt-gui/src/lib/components/LeadChat.svelte`
  - ProjectModePanelからチャット部分を分離、バブル表示、進捗インライン
  - 依存: T303

- [ ] T305 [US1] [テスト] ダッシュボード表示テスト `gwt-gui/src/lib/components/Dashboard.test.ts`
  - Issue/Task/Developer階層表示、ステータスバッジ、折りたたみ、タスク数表示
  - 依存: T104

- [ ] T306 [US1] ダッシュボード実装 `gwt-gui/src/lib/components/Dashboard.svelte`
  - 左カラム、Issue→Task→Developer階層、ステータスバッジ、折りたたみ/展開
  - 依存: T305

- [ ] T307 [US1] [テスト] AI設定未構成時エラー表示テスト `gwt-gui/src/lib/components/ProjectModePanel.test.ts`
  - AI設定無効時のエラーメッセージと設定ウィザード導線
  - 依存: T302

- [ ] T308 [US1] AI設定未構成時エラー表示実装 `gwt-gui/src/lib/components/ProjectModePanel.svelte`
  - エラーメッセージ（英語）+ AI設定ウィザード遷移リンク
  - 依存: T307

- [ ] T309 [US1] [テスト] コスト可視化テスト `gwt-gui/src/lib/components/ProjectModePanel.test.ts`
  - APIコール数・推定トークン数の表示検証
  - 依存: T302

- [ ] T310 [US1] コスト可視化実装 `gwt-gui/src/lib/components/ProjectModePanel.svelte`
  - LeadのLLM APIコール数/推定トークン数をGUI表示
  - 依存: T309

- [ ] T311 [US1] [テスト] Lead段階的委譲テスト `crates/gwt-tauri/src/agent_master.rs`
  - 自律実行可能操作（順序変更/リトライ等）と承認要求操作の分類テスト
  - 依存: T202

- [ ] T312 [US1] Lead段階的委譲ロジック実装 `crates/gwt-tauri/src/agent_master.rs`
  - タスク順序変更/並列度調整/リトライの自律判定、方針変更時の承認要求
  - 依存: T311

- [ ] T313 [US1] [テスト] Leadハイブリッド常駐テスト `crates/gwt-tauri/src/agent_master.rs`
  - イベントトリガー（完了検出/ユーザー入力/CI変更等）と2分間隔ポーリングの動作検証
  - 依存: T202

- [ ] T314 [US1] Leadハイブリッド常駐実装 `crates/gwt-tauri/src/agent_master.rs`
  - イベント駆動 + 2分間隔ポーリング、イベント間はLLMコール不要
  - 依存: T313

## Phase 4: US2 — GitHub Issue仕様管理と計画承認

> 独立テスト条件: テキスト入力からGitHub Issueにspec/plan/tasks/tdd記録、承認後にCoordinator起動指示が出ること

- [ ] T401 [US2] [テスト] issue_specツール群動作テスト `crates/gwt-tauri/src/agent_tools.rs`
  - upsert_spec_issue / get_spec_issue / upsert_spec_issue_artifact等の動作検証
  - 依存: T202

- [ ] T402 [US2] issue_specツール群のLead統合 `crates/gwt-tauri/src/agent_master.rs`
  - LeadがLLMツールとしてissue_specを呼び出し、GitHub Issueに仕様を記録する基盤
  - 依存: T401

- [ ] T403 [US2] [テスト] GitHub Issue仕様管理ワークフロー実行テスト `crates/gwt-tauri/src/agent_master.rs`
  - Leadが clarify→GitHub Issueにspec/plan/tasks/tdd記録を順次実行の検証
  - 依存: T402

- [ ] T404 [US2] GitHub Issue仕様管理ワークフロー実装 `crates/gwt-tauri/src/agent_master.rs`
  - ユーザー入力→clarify→GitHub Issue作成→spec/plan/tasks/tdd記録の一連フロー
  - 依存: T403

- [ ] T405 [US2] [テスト] 仕様4セクションゲートテスト `crates/gwt-tauri/src/agent_master.rs`
  - GitHub Issueのspec/plan/tasks/tdd のいずれか欠損時のブロック検証
  - 依存: T404

- [ ] T406 [US2] 仕様4セクションゲートチェック実装 `crates/gwt-tauri/src/agent_master.rs`
  - GitHub Issueの4セクション揃い判定（list_spec_issue_artifacts使用）+ 欠損時のエラーメッセージ生成
  - 依存: T405

- [ ] T407 [US2] [テスト] 計画提示・承認フローテスト `crates/gwt-tauri/src/agent_master.rs`
  - 計画表示→承認/拒否→Coordinator起動 or 再策定のフロー検証
  - 依存: T406

- [ ] T408 [US2] 計画提示・承認フロー実装 `crates/gwt-tauri/src/agent_master.rs`
  - GitHub Issueのspec/plan/tasksの順で提示、承認UIメッセージ（英語）
  - 依存: T407

- [ ] T409 [US2] [テスト] GitHub Issue作成・Project登録テスト `crates/gwt-tauri/src/agent_master.rs`
  - Issue作成→Project登録→ステータス遷移の検証
  - 依存: T101

- [ ] T410 [US2] GitHub Issue作成・GitHub Project登録実装 `crates/gwt-tauri/src/agent_master.rs`
  - 既存 issue_spec ツールとの連携
  - 依存: T409

- [ ] T411 [US2] [テスト] Coordinator→Lead ハイブリッド通信テスト `crates/gwt-tauri/src/agent_master.rs`
  - 重要イベント（Tauriイベント）と途中経過（scrollback読み取り）のテスト
  - 依存: T202

- [ ] T412 [US2] Coordinator→Lead ハイブリッド通信実装 `crates/gwt-tauri/src/agent_master.rs`
  - agent-status-changed Tauriイベント受信 + capture_scrollback_tail定期取得
  - 依存: T411

- [ ] T413 [US2] [テスト] Issue展開・Coordinator詳細テスト `gwt-gui/src/lib/components/IssueItem.test.ts`
  - Issue展開時のCoordinator詳細表示（ステータス/CI結果/Developer一覧）
  - 依存: T104

- [ ] T414 [US2] Issue階層コンポーネント + Coordinator詳細展開実装 `gwt-gui/src/lib/components/IssueItem.svelte`
  - Issue情報、展開時Coordinator詳細、View Terminal/Chatリンク
  - 依存: T413

## Phase 5: US3 — Developer起動と実装

> 独立テスト条件: Coordinatorからの指示でDeveloperがGUI内蔵ターミナルペインで起動しプロンプト受信すること

- [ ] T501 [US3] [テスト] Coordinator起動テスト `crates/gwt-tauri/src/commands/project_mode.rs`
  - Issue単位でのCoordinator起動→ターミナルペイン割当→cwd=リポジトリルート→GitHub Issue番号渡し
  - 依存: T408

- [ ] T502 [US3] Coordinator起動・管理実装 `crates/gwt-tauri/src/commands/project_mode.rs`
  - GUI内蔵ターミナルペインでCoordinator起動、GitHub Issue番号渡し、複数並列管理
  - 依存: T501

- [ ] T503 [US3] [テスト] タスク分割・Developer割り当てテスト `crates/gwt-tauri/src/commands/project_mode.rs`
  - 1タスクに複数Developer+Worktreeを割り当てるロジック検証
  - 依存: T502

- [ ] T504 [US3] タスク分割とDeveloper割り当て実装 `crates/gwt-tauri/src/commands/project_mode.rs`
  - 大タスク→複数Developer並列、独立→別Worktree、依存→同一/merge連携
  - 依存: T503

- [ ] T505 [US3] [テスト] Worktree/ブランチ自動作成テスト `crates/gwt-core/src/agent/worktree.rs`
  - agent/プレフィックス、命名サニタイズ、重複時連番付与の検証
  - 依存: T101

- [ ] T506 [US3] Worktree/ブランチ自動作成実装 `crates/gwt-core/src/agent/worktree.rs`
  - 既存 sanitize_branch_name() 拡張、worktreeパス生成、連番付与
  - 依存: T505

- [ ] T507 [US3] [テスト] Developer起動テスト `crates/gwt-tauri/src/commands/project_mode.rs`
  - PTY直接通信でプロンプトが送信され、全自動モードで起動されるテスト
  - 依存: T504, T506

- [ ] T508 [US3] Developer起動とプロンプト送信実装 `crates/gwt-tauri/src/commands/project_mode.rs`
  - ユーザー指定エージェント種別の自動モードフラグ + アダプティブプロンプト + CLAUDE.md規約
  - 依存: T507

- [ ] T509 [US3] [テスト] TaskクリックでBranch Modeジャンプテスト `gwt-gui/src/lib/components/Dashboard.test.ts`
  - Taskクリック→Branch Mode切替→該当Worktree選択の検証
  - 依存: T306

- [ ] T510 [US3] Dashboard→Branch Mode連携実装 `gwt-gui/src/lib/components/Dashboard.svelte`
  - Taskクリック→Branch Modeタブ切替→該当Worktree自動遷移
  - 依存: T509

## Phase 6: US4 — Developer完了検出

> 独立テスト条件: DeveloperがタスクをComplete→Coordinatorが検出して次アクションに移行すること

- [ ] T601 [US4] [テスト] Developer完了検出テスト（Hook Stop / GWT_TASK_DONE / プロセス終了） `crates/gwt-tauri/src/commands/project_mode.rs`
  - 各検出方式の動作検証、フォールバック順序の検証
  - 依存: T508

- [ ] T602 [US4] Developer完了検出実装（複合方式） `crates/gwt-tauri/src/commands/project_mode.rs`
  - Hook Stop最優先 → フォールバック（出力パターン / プロセス終了）
  - 依存: T601

- [ ] T603 [US4] [テスト] Lead途中経過報告テスト `crates/gwt-tauri/src/agent_master.rs`
  - scrollback読み取り→チャット報告（LLMコール不要）の検証
  - 依存: T412

- [ ] T604 [US4] Lead途中経過報告実装 `crates/gwt-tauri/src/agent_master.rs`
  - 2分間隔でscrollback取得、進捗フォーマット（英語、10行以内）でチャット表示
  - 依存: T603

## Phase 7: US5 — 成果物検証と統合（PR経由）

> 独立テスト条件: Developer完了→テスト実行→PR作成→CI監視→修正ループが動作すること

- [ ] T701 [US5] [テスト] 成果物検証（テスト実行→PR作成）テスト `crates/gwt-tauri/src/commands/project_mode.rs`
  - Developer完了→テスト実行→パス→PR作成のフロー検証
  - 依存: T602

- [ ] T702 [US5] 成果物検証とPR作成実装 `crates/gwt-tauri/src/commands/project_mode.rs`
  - テスト実行指示、テスト失敗時最大3回リトライ、PR作成条件チェック、LLMでPRタイトル/本文生成
  - 依存: T701

- [ ] T703 [US5] [テスト] CI監視・自律修正ループテスト `crates/gwt-tauri/src/commands/project_mode.rs`
  - CI失敗検出→Developer修正指示→再プッシュ→CI再実行、最大3回→Lead報告の検証
  - 依存: T702

- [ ] T704 [US5] CI監視・自律修正ループ実装 `crates/gwt-tauri/src/commands/project_mode.rs`
  - gh pr checks監視 + Developer修正指示 + 3回失敗時Lead報告→ユーザー通知
  - 依存: T703

- [ ] T705 [US5] [テスト] Developer間コンテキスト共有テスト `crates/gwt-core/src/agent/worktree.rs`
  - 先行タスクcommit→後続タスクにmerge→コンフリクト時Developer解決指示
  - 依存: T506

- [ ] T706 [US5] Developer間コンテキスト共有実装 `crates/gwt-core/src/agent/worktree.rs`
  - 先行タスクブランチのmerge実行、コンフリクト検出・解決指示
  - 依存: T705

## Phase 8: US6 — 障害ハンドリングと層間独立性

> 独立テスト条件: 各層の障害シナリオで他層が独立して動作し続けること

- [ ] T801 [US6] [テスト] 層間独立性テスト `crates/gwt-tauri/src/commands/project_mode.rs`
  - Lead API障害時にCoordinator/Developer続行の検証
  - 依存: T602

- [ ] T802 [US6] 層間独立性保証実装 `crates/gwt-tauri/src/commands/project_mode.rs`
  - 各層のプロセス独立性、障害検出・通知
  - 依存: T801

- [ ] T803 [US6] [テスト] Coordinator自律再起動テスト `crates/gwt-tauri/src/agent_master.rs`
  - Coordinatorクラッシュ→Lead検出→自律再起動→状態再取得の検証
  - 依存: T802

- [ ] T804 [US6] Coordinator自律再起動実装 `crates/gwt-tauri/src/agent_master.rs`
  - クラッシュ検出→30秒以内に再起動→Developer状態再取得
  - 依存: T803

## Phase 9: US7 — セッション永続化と再開

> 独立テスト条件: gwt終了→再起動→前回セッション復元で中断前の状態から継続できること

- [ ] T901 [US7] [テスト] セッション永続化テスト `crates/gwt-tauri/src/commands/project_mode.rs`
  - Lead会話/Coordinator状態/Developer状態/タスク一覧のJSON保存検証
  - 依存: T105

- [ ] T902 [US7] セッション永続化実装 `crates/gwt-tauri/src/commands/project_mode.rs`
  - 状態変更トリガーで~/.gwt/sessions/に保存（アトミック書き込み）
  - 依存: T901

- [ ] T903 [US7] [テスト] セッション復元・再開テスト `crates/gwt-tauri/src/commands/project_mode.rs`
  - gwt再起動→最新未完了セッション復元→Coordinator/Developer再接続の検証
  - 依存: T902

- [ ] T904 [US7] セッション復元・再開実装 `crates/gwt-tauri/src/commands/project_mode.rs`
  - JSON読み込み→状態復元→ペイン再接続→worktree消失時Failed化
  - 依存: T903

- [ ] T905 [US7] [テスト] セッション強制中断テスト `crates/gwt-tauri/src/commands/project_mode.rs`
  - Esc→全ペインSIGTERM→5秒タイムアウト→Paused永続化の検証
  - 依存: T902

- [ ] T906 [US7] セッション強制中断実装 `crates/gwt-tauri/src/commands/project_mode.rs`
  - Escキーイベント→SIGTERM送信→Paused状態保存→チャット表示
  - 依存: T905

## Phase 10: US8 — 直接アクセスと層間対話

> 独立テスト条件: Developerターミナルに直接キー入力でき、CoordinatorにチャットでIssue展開から指示できること

- [ ] T1001 [US8] [テスト] Coordinator詳細パネル表示テスト `gwt-gui/src/lib/components/CoordinatorDetail.test.ts`
  - Coordinator状態、Developer一覧、View Terminal/Chatリンクの表示検証
  - 依存: T104

- [ ] T1002 [US8] Coordinator詳細パネル実装 `gwt-gui/src/lib/components/CoordinatorDetail.svelte`
  - 状態表示、CI結果、Developer一覧、ターミナル/チャットリンク
  - 依存: T1001

- [ ] T1003 [US8] [テスト] 直接アクセステスト `gwt-gui/src/lib/components/`
  - Developerターミナル直操作、Coordinatorチャット入力の検証
  - 依存: T1002

- [ ] T1004 [US8] 直接アクセス実装 `gwt-gui/src/lib/components/`
  - Developerターミナルペイン直接キー入力、Coordinatorチャット送信
  - 依存: T1003

## Phase 11: US9 — コンテキスト管理

> 独立テスト条件: 長時間対話でコンテキスト閾値を超えた場合に要約圧縮が実行されること

- [ ] T1101 [US9] [テスト] コンテキスト要約・圧縮テスト `crates/gwt-tauri/src/context_summarizer.rs`
  - Developer: LLM自動（gwt不介入）、Lead/Coordinator: 80%閾値で圧縮実行の検証
  - 依存: T202

- [ ] T1102 [US9] コンテキスト要約・圧縮実装 `crates/gwt-tauri/src/context_summarizer.rs`
  - 推定トークン数チェック→閾値超過時に完了タスク/古い会話を要約→LLMコール
  - 依存: T1101

## Phase 12: 仕上げ・横断

- [ ] T1201 [P] [共通] PTY通信スキル完全移行 `crates/gwt-tauri/src/agent_tools.rs` `crates/gwt-tauri/src/commands/terminal.rs`
  - agent_tools.rs の send_keys系3ツール → Claude Code プラグインスキルへ完全移行
  - 旧ツール呼び出しの廃止確認
  - 依存: T204

- [ ] T1202 [P] [共通] ブランチモード連携 `gwt-gui/src/lib/components/`
  - agent/ブランチのブランチモード完全表示、削除時Failed/Paused検出
  - 依存: T506

- [ ] T1203 [P] [共通] issue_specスキル公開 `crates/gwt-tauri/src/agent_tools.rs`
  - issue_specツール群をClaude Codeプラグインスキルとして公開、ブランチモード各エージェントから利用可能に
  - 依存: T402

- [ ] T1206 [P] [共通] ブランチモード GitHub Issueボタン `gwt-gui/src/lib/components/`
  - GUI上にGitHub Issueボタン設置、gwt内蔵AIがissue_specツールでGitHub Issue管理を実行
  - 依存: T1203

- [ ] T1204 [P] [共通] ログ記録実装 `crates/gwt-tauri/src/`
  - agent.lead.llm / agent.coordinator / agent.developer カテゴリのJSON Linesログ出力
  - 依存: T602

- [ ] T1205 [共通] 仕様・計画・タスクの最終同期確認 `specs/SPEC-ba3f610c/`
  - spec.md/plan.md/tasks.md/tdd.md/data-model.mdの整合性最終確認
  - 依存: 全タスク
