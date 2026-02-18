# タスクリスト: プロジェクトチーム（Project Team）

## 完了済み（旧アーキテクチャ: MA + Agentビュー）

> 以下のT001〜T018は旧Master Agent + Agentビュー構成で完了したタスク。
> 3層アーキテクチャへのリファクタリングにあたり、成果物は再利用しつつ新タスクで拡張する。

- [x] T001〜T018: 旧アーキテクチャ実装完了（詳細は git log 参照）

## Phase A: 基盤モデル・型定義

- [ ] T101 [US1] [基盤] エンティティモデル定義（Project / Issue / Task / Coordinator / Developer） `crates/gwt-core/src/agent/mod.rs`
  - Project(1) → Issue(N) → Task(N) → Developer(N) → Worktree(1) の階層モデル
  - Lead/Coordinator/Developerの状態enum
  - 依存: なし

- [ ] T102 [US1] [基盤] フロント型定義を3層対応に拡張 `gwt-gui/src/lib/types.ts`
  - Project / Issue / LeadState / CoordinatorState / DeveloperState / TaskCard / KanbanColumn 型
  - 1 Task = N Developer = N Worktree の表現
  - 依存: T101

- [ ] T103 [US1] [基盤] PTY通信スキル化の基盤インターフェース定義 `crates/gwt-tauri/src/commands/terminal.rs`
  - send_keys_to_pane / send_keys_broadcast / capture_scrollback_tail のスキルAPI設計
  - 依存: なし

## Phase B: Lead（PM）機能

- [ ] T201 [US1] [テスト] Leadチャット（IME/スピナー/自動スクロール）の既存テスト維持・拡張 `gwt-gui/src/lib/components/ProjectTeamPanel.test.ts`
  - 旧AgentModePanel.test.tsのテストをリネーム・拡張
  - 依存: T102

- [ ] T202 [US1] [実装] LeadチャットUI（バブル表示 + 入力エリア） `gwt-gui/src/lib/components/LeadChat.svelte`
  - 旧AgentModePanelのチャット機能をLeadChat.svelteとして分離・拡張
  - 依存: T201

- [ ] T203 [US2] [テスト] Spec Kit内蔵化テスト（LLMプロンプトテンプレート呼び出し） `crates/gwt-tauri/src/agent_master.rs`
  - clarify/specify/plan/tasks/tddの各テンプレート実行テスト
  - 依存: T101

- [ ] T204 [US2] [実装] Spec Kit LLMプロンプトテンプレートのRust組み込み `crates/gwt-tauri/src/agent_master.rs`
  - include_str!マクロでテンプレート埋め込み、LLM経由実行
  - 依存: T203

- [ ] T205 [US2] [テスト] Spec Kitワークフロー実行テスト（clarify→specify→plan→tasks→tdd） `crates/gwt-tauri/src/agent_master.rs`
  - Leadが全フローを順次実行し、成果物4点が生成されるテスト
  - 依存: T204

- [ ] T206 [US2] [実装] Leadによる全Spec Kitワークフロー実行 `crates/gwt-tauri/src/agent_master.rs`
  - ユーザー入力→clarify→specify→plan→tasks→tddの一連フロー
  - 依存: T205

- [ ] T207 [US2] [テスト] 成果物4点ゲート（4点揃うまでCoordinator起動不可）テスト `crates/gwt-tauri/src/agent_master.rs`
  - spec.md/plan.md/tasks.md/tdd.mdのいずれか欠損時のブロック検証
  - 依存: T206

- [ ] T208 [US2] [実装] 成果物4点ゲートチェック `crates/gwt-tauri/src/agent_master.rs`
  - 4点揃い判定 + 欠損時のエラーメッセージ生成
  - 依存: T207

- [ ] T209 [US2] [テスト] 計画提示・承認フローテスト `crates/gwt-tauri/src/agent_master.rs`
  - 計画表示→承認/拒否→Coordinator起動 or 再策定のフロー検証
  - 依存: T208

- [ ] T210 [US2] [実装] 計画提示・承認フロー `crates/gwt-tauri/src/agent_master.rs`
  - spec.md/plan.md/tasks.mdの順で提示、承認UIメッセージ（英語）
  - 依存: T209

- [ ] T211 [US2] [テスト] GitHub Issue作成・Project登録テスト `crates/gwt-tauri/src/agent_master.rs`
  - Issue作成→Project登録→ステータス遷移の検証
  - 依存: T101

- [ ] T212 [US2] [実装] GitHub Issue作成・GitHub Project登録 `crates/gwt-tauri/src/agent_master.rs`
  - gwt_issue_spec MCPとの連携（spec_issue_upsert, spec_project_sync等）
  - 依存: T211

- [ ] T213 [US1] [テスト] Lead段階的委譲テスト（自律範囲 vs 承認要求） `crates/gwt-tauri/src/agent_master.rs`
  - 自律実行可能な操作と承認が必要な操作の分類テスト
  - 依存: T101

- [ ] T214 [US1] [実装] Lead段階的委譲ロジック `crates/gwt-tauri/src/agent_master.rs`
  - タスク順序変更/並列度調整/リトライ等の自律判定
  - 依存: T213

- [ ] T215 [US1] [テスト] Leadハイブリッド常駐テスト（イベント駆動 + ポーリング） `crates/gwt-tauri/src/agent_master.rs`
  - イベントトリガーと2分間隔ポーリングの動作検証
  - 依存: T101

- [ ] T216 [US1] [実装] Leadハイブリッド常駐（イベント駆動 + 定期ポーリング） `crates/gwt-tauri/src/agent_master.rs`
  - Developer完了/Coordinator状態変更/ユーザー入力イベント + 2分間隔チェック
  - 依存: T215

- [ ] T217 [US1] [テスト] Lead gwt内蔵AI実行基盤テスト `crates/gwt-tauri/src/agent_master.rs`
  - gwt内蔵AIとしてのLLM呼び出し・対話ループ動作検証
  - 依存: T101

- [ ] T218 [US1] [実装] Lead gwt内蔵AI実行基盤 `crates/gwt-tauri/src/agent_master.rs`
  - gwt自身がLLMを呼び出し、チャットUIで統一的UXを提供
  - 依存: T217

## Phase C: Coordinator（Orchestrator）機能

- [ ] T301 [US2] [テスト] Coordinator起動テスト（1 Issue = 1 Coordinator、並列起動） `crates/gwt-tauri/src/commands/agent_mode.rs`
  - Issue単位でのCoordinator起動 → ターミナルペイン割当 → 複数並列起動の検証
  - ファイルパス（specs/SPEC-xxx/）受け渡しの検証
  - 依存: T210

- [ ] T302 [US2] [実装] Coordinator起動・管理（Issue単位、並列対応） `crates/gwt-tauri/src/commands/agent_mode.rs`
  - GUI内蔵ターミナルペインでのCoordinator起動、ファイルパス渡し、複数並列管理
  - 依存: T301

- [ ] T303 [US3] [テスト] タスク分割・Developer割り当てテスト（1 Task = N Developer） `crates/gwt-tauri/src/commands/agent_mode.rs`
  - 1タスクに複数Developer+Worktreeを割り当てるロジック検証
  - 依存: T302

- [ ] T304 [US3] [実装] タスク分割とDeveloper割り当て（1 Task = N Developer = N Worktree） `crates/gwt-tauri/src/commands/agent_mode.rs`
  - 大タスク→複数Developer並列、独立タスク→別Worktree、依存タスク→同一Worktree or merge連携
  - 依存: T303

- [ ] T305 [US3] [テスト] Worktree/ブランチ自動作成テスト `crates/gwt-core/src/agent/`
  - agent/プレフィックス、命名規則、重複時連番付与の検証
  - 依存: T101

- [ ] T306 [US3] [実装] Worktree/ブランチ自動作成 `crates/gwt-core/src/agent/`
  - ブランチ名サニタイズ、worktreeパス生成、連番付与
  - 依存: T305

- [ ] T307 [US3] [テスト] Developer起動テスト（ターミナルペイン + プロンプト送信） `crates/gwt-tauri/src/commands/agent_mode.rs`
  - PTY直接通信でプロンプトが送信されるテスト
  - 依存: T304, T306

- [ ] T308 [US3] [実装] Developer起動とプロンプト送信 `crates/gwt-tauri/src/commands/agent_mode.rs`
  - 全自動モード起動 + アダプティブプロンプト生成 + CLAUDE.md規約含む
  - 依存: T307

- [ ] T309 [US4] [テスト] Developer完了検出テスト（Hook Stop / GWT_TASK_DONE / プロセス終了） `crates/gwt-tauri/src/commands/agent_mode.rs`
  - 各検出方式の動作検証
  - 依存: T308

- [ ] T310 [US4] [実装] Developer完了検出（複合方式） `crates/gwt-tauri/src/commands/agent_mode.rs`
  - Hook Stop最優先 → フォールバック（出力パターン / プロセス終了）
  - 依存: T309

- [ ] T311 [US5] [テスト] CI監視・自律修正ループテスト `crates/gwt-tauri/src/commands/agent_mode.rs`
  - CI失敗検出→Developer修正指示→再プッシュ→CI再実行、最大3回の検証
  - 依存: T310

- [ ] T312 [US5] [実装] CoordinatorによるCI監視・自律修正ループ `crates/gwt-tauri/src/commands/agent_mode.rs`
  - gh pr checks監視 + Developer修正指示 + 3回失敗時Lead報告
  - 依存: T311

- [ ] T313 [US5] [テスト] 成果物検証（テスト実行→PR作成）テスト `crates/gwt-tauri/src/commands/agent_mode.rs`
  - Developer完了→テスト実行→パス→PR作成のフロー検証
  - 依存: T310

- [ ] T314 [US5] [実装] 成果物検証とPR作成 `crates/gwt-tauri/src/commands/agent_mode.rs`
  - テスト実行指示、PR作成条件チェック、LLMでPRタイトル/本文生成
  - 依存: T313

- [ ] T315 [US5] [テスト] Developer間コンテキスト共有テスト（Git merge） `crates/gwt-core/src/agent/`
  - 先行タスクcommit→後続タスクにmerge→コンフリクト時Developer解決指示
  - 依存: T306

- [ ] T316 [US5] [実装] Developer間コンテキスト共有（Git merge経由） `crates/gwt-core/src/agent/`
  - 先行タスクブランチのmerge実行、コンフリクト検出・解決指示
  - 依存: T315

## Phase D: GUI（プロジェクトチームUI）

- [ ] T401 [US1] [テスト] Project Teamタブ・モード切り替えテスト `gwt-gui/src/lib/components/ProjectTeamPanel.test.ts`
  - タブ選択→Project Team画面表示、他タブ→ブランチモード復帰
  - 依存: T102

- [ ] T402 [US1] [実装] Project Teamタブ・モード切り替え `gwt-gui/src/lib/components/ProjectTeamPanel.svelte`
  - 旧AgentModePanelをProjectTeamPanelにリネーム・拡張
  - 依存: T401

- [ ] T403 [US1] [テスト] 下部パネル切替テスト（Chat / Kanban / Coordinator） `gwt-gui/src/lib/components/ProjectTeamPanel.test.ts`
  - 3つのパネルビューの切り替え動作検証
  - 依存: T402

- [ ] T404 [US1] [実装] 下部パネル切替（Chat / Kanban / Coordinator） `gwt-gui/src/lib/components/ProjectTeamPanel.svelte`
  - タブ切り替えUIとパネルコンテンツの動的表示
  - 依存: T403

- [ ] T405 [US2] [テスト] Kanbanボード表示テスト（4カラム + Issue別フィルタ + タスクカード） `gwt-gui/src/lib/components/KanbanBoard.test.ts`
  - Pending/Running/Completed/Failedの4カラム、Issue別フィルタ、1 Task = N Developer表示
  - 依存: T102

- [ ] T406 [US2] [実装] Kanbanボード `gwt-gui/src/lib/components/KanbanBoard.svelte`
  - 4カラムレイアウト、Issue別フィルタ/グルーピング、TaskCardコンポーネント、worktree相対パス表示
  - 依存: T405

- [ ] T407 [US2] [テスト] タスクカード表示テスト（タスク名/ブランチ名/worktree/ホバー） `gwt-gui/src/lib/components/TaskCard.test.ts`
  - カード内表示情報、ホバーで絶対パス表示
  - 依存: T102

- [ ] T408 [US2] [実装] タスクカード `gwt-gui/src/lib/components/TaskCard.svelte`
  - タスクID/名前、ブランチ名、worktree相対パス、ホバーtooltip
  - 依存: T407

- [ ] T409 [US8] [テスト] Coordinatorパネル表示テスト `gwt-gui/src/lib/components/CoordinatorPanel.test.ts`
  - Coordinator状態、Developer一覧、View Terminal/Chatボタン
  - 依存: T102

- [ ] T410 [US8] [実装] Coordinatorパネル `gwt-gui/src/lib/components/CoordinatorPanel.svelte`
  - 状態表示、Developer一覧、ターミナル/チャット切り替えボタン
  - 依存: T409

- [ ] T411 [US1] [テスト] コスト可視化テスト `gwt-gui/src/lib/components/ProjectTeamPanel.test.ts`
  - APIコール数・推定トークン数の表示検証
  - 依存: T102

- [ ] T412 [US1] [実装] コスト可視化 `gwt-gui/src/lib/components/ProjectTeamPanel.svelte`
  - LeadのLLM APIコール数/推定トークン数をGUI表示
  - 依存: T411

- [ ] T413 [US1] [テスト] AI設定未構成時エラー表示テスト `gwt-gui/src/lib/components/ProjectTeamPanel.test.ts`
  - AI設定無効時のエラーメッセージと設定ウィザード導線の表示検証
  - 依存: T402

- [ ] T414 [US1] [実装] AI設定未構成時エラー表示 `gwt-gui/src/lib/components/ProjectTeamPanel.svelte`
  - エラーメッセージ（英語）+ AI設定ウィザード遷移リンク
  - 依存: T413

## Phase E: セッション・障害・連携

- [ ] T501 [US7] [テスト] セッション永続化テスト（全層状態保存） `crates/gwt-tauri/src/commands/agent_mode.rs`
  - Lead会話/Coordinator状態/Developer状態/タスク一覧の保存検証
  - 依存: T101

- [ ] T502 [US7] [実装] セッション永続化（JSON形式） `crates/gwt-tauri/src/commands/agent_mode.rs`
  - 状態変更トリガーで~/.gwt/sessions/に保存
  - 依存: T501

- [ ] T503 [US7] [テスト] セッション復元・再開テスト `crates/gwt-tauri/src/commands/agent_mode.rs`
  - gwt再起動→最新未完了セッション復元→続行の検証
  - 依存: T502

- [ ] T504 [US7] [実装] セッション復元・再開 `crates/gwt-tauri/src/commands/agent_mode.rs`
  - JSON読み込み→状態復元→Coordinator/Developer再接続
  - 依存: T503

- [ ] T505 [US6] [テスト] 層間独立性テスト（上位障害時の下位続行） `crates/gwt-tauri/src/commands/agent_mode.rs`
  - Lead API障害時にCoordinator/Developer続行の検証
  - 依存: T101

- [ ] T506 [US6] [実装] 層間独立性保証 `crates/gwt-tauri/src/commands/agent_mode.rs`
  - 各層のプロセス独立性、障害検出・通知
  - 依存: T505

- [ ] T507 [US6] [テスト] Coordinator自律再起動テスト `crates/gwt-tauri/src/agent_master.rs`
  - Coordinatorクラッシュ→Lead検出→自律再起動→状態再取得
  - 依存: T506

- [ ] T508 [US6] [実装] Coordinator自律再起動 `crates/gwt-tauri/src/agent_master.rs`
  - クラッシュ検出→自律再起動→Developer状態再取得
  - 依存: T507

- [ ] T509 [US1] [テスト] セッション強制中断テスト（Esc→SIGTERM→Paused） `crates/gwt-tauri/src/commands/agent_mode.rs`
  - Esc→全ペインSIGTERM→5秒タイムアウト→Paused永続化
  - 依存: T502

- [ ] T510 [US1] [実装] セッション強制中断 `crates/gwt-tauri/src/commands/agent_mode.rs`
  - Escキーイベント→SIGTERM送信→Paused状態保存→チャット表示
  - 依存: T509

- [ ] T511 [US1] [テスト] ブランチモード連携テスト（agent/ブランチ表示・削除検出） `gwt-gui/src/lib/components/`
  - agent/ブランチのブランチモード完全表示、削除時Failed/Paused検出
  - 依存: T306

- [ ] T512 [US1] [実装] ブランチモード連携 `gwt-gui/src/lib/components/`
  - agent/ブランチ表示、削除検出→該当タスクFailed/Paused
  - 依存: T511

- [ ] T513 [US9] [テスト] コンテキスト要約・圧縮テスト `crates/gwt-tauri/src/agent_master.rs`
  - 完了タスク/古い会話の要約対象判定テスト
  - 依存: T101

- [ ] T514 [US9] [実装] コンテキスト要約・圧縮 `crates/gwt-tauri/src/agent_master.rs`
  - 直近メッセージ/未完了タスク保持、完了分の要約圧縮
  - 依存: T513

## Phase F: スキル化・統合

- [ ] T601 [US8] [テスト] PTY通信スキル化テスト `crates/gwt-tauri/src/commands/terminal.rs`
  - send_keys_to_pane/send_keys_broadcast/capture_scrollback_tailのスキルAPI検証
  - 依存: T103

- [ ] T602 [US8] [実装] PTY通信のClaude Codeスキル化 `crates/gwt-tauri/src/commands/terminal.rs`
  - agent_tools.rs→Claude Codeプラグインスキルへの完全移行
  - 依存: T601

- [ ] T603 [US8] [テスト] 直接アクセステスト（Developerターミナル / Coordinatorチャット） `gwt-gui/src/lib/components/`
  - Developerターミナル直操作、Coordinatorチャット入力の検証
  - 依存: T410

- [ ] T604 [US8] [実装] 直接アクセス `gwt-gui/src/lib/components/`
  - Developerターミナルペイン直接キー入力、Coordinatorチャット
  - 依存: T603

## Phase G: 仕上げ

- [ ] T701 [P] [共通] ログ記録実装（agent.lead.llm / agent.coordinator / agent.developer） `crates/gwt-tauri/src/`
  - 全カテゴリのJSON Linesログ出力
  - 依存: T310

- [ ] T702 [P] [共通] 仕様・計画・タスクの最終同期確認 `specs/SPEC-ba3f610c/`
  - spec.md/plan.md/tasks.md/tdd.mdの整合性最終確認
  - 依存: 全タスク
