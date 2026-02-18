# 実装計画: プロジェクトチーム（Project Team）

**仕様ID**: `SPEC-ba3f610c`
**日付**: 2026-02-19
**仕様書**: [spec.md](./spec.md)

## 概要

3層エージェントアーキテクチャ（Lead / Coordinator / Developer）によるプロジェクト統括機能を実装する。

- **Lead**（gwt内蔵AI）がプロジェクト全体を管理し、要件定義・Spec Kitワークフロー・GitHub Issue/Project管理を担う
- **Coordinator** が Issue単位（1 Issue = 1 Coordinator）でタスク管理・CI監視・Developer管理を行う（複数並列起動可）
- **Developer** が各Worktreeで実装を実行する（1 Task = N Developer = N Worktree）

## 技術コンテキスト

- 言語: Rust 2021 Edition / TypeScript
- GUI: Tauri v2 + Svelte 5 + xterm.js
- バックエンド: gwt-tauri + gwt-core
- ストレージ: ファイルシステム（`~/.gwt/sessions/`）
- テスト: cargo test / pnpm test / vitest
- CI/CD: GitHub Actions

## 実装スコープ

### Phase A: 基盤モデル・型定義

1. エンティティモデル定義（Project / Issue / Task / Coordinator / Developer）
2. フロント型定義の3層対応（Lead / Coordinator / Developer）
3. PTY通信のスキル化基盤（agent_tools.rs → Claude Code plugin skills）

### Phase B: Lead（PM）機能

4. Lead実行基盤（gwt内蔵AIとして実装）
5. Leadチャット（ユーザー対話、IME/スピナー/自動スクロール）
6. Spec Kit内蔵化（LLMプロンプトテンプレートのRust組み込み）
7. Spec Kitワークフロー実行（clarify → specify → plan → tasks → tdd）
8. 成果物4点ゲート（spec.md/plan.md/tasks.md/tdd.md揃うまでCoordinator起動不可）
9. 計画提示・承認フロー
10. GitHub Issue作成・GitHub Project登録
11. 段階的委譲（自律範囲の判定ロジック）
12. ハイブリッド常駐（イベント駆動 + 2分間隔ポーリング）
13. 途中経過報告（軽量な状態チェック + チャット報告）

### Phase C: Coordinator（Orchestrator）機能

14. Coordinator起動・管理（GUI内蔵ターミナルペインで起動）
15. Issue単位のCoordinator起動（1 Issue = 1 Coordinator、複数並列可）
16. タスク分割・Developer割り当て（1 Task = N Developer = N Worktree対応）
17. Worktree/ブランチ自動作成（`agent/`プレフィックス、命名規則）
18. Developer起動プロンプト生成（アダプティブ、CLAUDE.md規約含む）
19. Developer完了検出（Hook Stop / GWT_TASK_DONE / プロセス終了）
20. 並列実行制御（同時実行数のLLM判断）
21. CI監視と自律修正ループ（CI失敗 → Developer修正 → 再プッシュ、最大3回）
22. 成果物検証（テスト実行 → PR作成）
23. Developer間コンテキスト共有（Git merge経由）

### Phase D: GUI（プロジェクトチームUI）

24. Project Teamタブ・モード切り替え
25. ダッシュボード（左カラム：Issue/Task/Developer階層表示、ステータスバッジ）
26. Leadチャット画面（右カラム：バブル表示、入力エリア、進捗インライン表示）
27. ダッシュボード内Coordinator詳細展開（ステータス、ターミナル/チャットリンク）
28. Developer表示のBranch Mode連携（TaskクリックでWorktreeジャンプ）
29. コスト可視化（APIコール数/推定トークン数）
30. AI設定未構成時のエラー表示

### Phase E: セッション・障害・連携

31. セッション永続化（JSON形式、~/.gwt/sessions/）
32. セッション復元・再開
33. 層間独立性（Lead障害 / Coordinator障害 / Developer障害の各ハンドリング）
34. Coordinator自律再起動
35. セッション強制中断（Esc → SIGTERM → Paused永続化）
36. ブランチモード連携（agent/ブランチ表示・削除検出）
37. ログ記録（agent.lead.llm / agent.coordinator / agent.developer）
38. コンテキスト要約・圧縮

### Phase F: スキル化・統合

39. PTY通信のClaude Codeスキル化（send_keys_to_pane / send_keys_broadcast / capture_scrollback_tail）
40. agent_tools.rsからの完全移行
41. 直接アクセス（Developerターミナル直操作 / Coordinatorチャット）

## 主要コード構成

```text
crates/gwt-core/src/
├── agent/
│   ├── mod.rs                    # 3層エージェントモデル
│   ├── lead.rs                   # Lead状態・ロジック
│   ├── coordinator.rs            # Coordinator状態・ロジック
│   └── developer.rs              # Developer状態・ロジック

crates/gwt-tauri/src/
├── agent_master.rs               # Lead実行ループ（旧MA）
├── state.rs                      # ウィンドウごとのProject Team状態
├── commands/
│   ├── agent_mode.rs             # Project Team用Tauriコマンド
│   └── terminal.rs               # PTY通信（スキル化元）
└── session/                      # セッション永続化

gwt-gui/src/
├── lib/components/
│   ├── ProjectTeamPanel.svelte   # メインパネル（旧AgentModePanel）
│   ├── LeadChat.svelte           # Leadチャット（右カラム）
│   ├── Dashboard.svelte          # ダッシュボード（左カラム）
│   ├── IssueItem.svelte          # Issue階層表示コンポーネント
│   └── CoordinatorDetail.svelte  # Coordinator詳細展開
└── lib/types.ts                  # 3層対応型定義
```

## 実装方針

- 既存のAgentModePanel/AgentSidebarをリネーム・拡張して3層対応する
- Lead実行ループは既存のagent_master.rsを拡張する
- Coordinator/Developerは新規の状態管理モジュールとして追加する
- GUI UIは「Leadチャット + 下部切替パネル」構成で実装する
- PTY通信は既存のsend_keys系を維持しつつ、スキルとしてのインターフェースを追加する
- セッション永続化は既存の~/.gwt/sessions/を拡張する
- テストは各フェーズごとにTDD（テストファースト）で進める

## 受け入れ条件

- Leadチャットでユーザーと対話し、プロジェクト全体の要件定義・Spec Kitワークフローを実行できる
- Leadが要件をIssue単位に分割し、各Issue分の成果物4点を生成できる
- 承認後、各IssueにCoordinatorを並列起動できる（ファイルパス受け渡し）
- 1 Task = N Developer = N Worktreeの並列割り当てが可能
- ダッシュボードでIssue/Task/Developerの階層をステータス付きで常時俯瞰できる
- ダッシュボードのTaskクリックでBranch Modeの該当Worktreeにジャンプできる
- CoordinatorがCI結果を監視し、失敗時に自律修正ループを実行できる
- 各層が独立して動作し、上位層の障害が下位層に影響しない
- セッションを永続化し、gwt再起動後に復元・再開できる
- IME/スピナー/自動スクロールの既存チャット要件を維持する
- PTY通信がClaude Codeスキルとして利用可能

## 検証方針

- フロントエンド: ダッシュボード、チャット、Branch Mode連携をコンポーネントテスト（vitest）で検証
- バックエンド: Lead/Coordinator/Developer状態管理、成果物ゲート、CI監視をユニットテスト（cargo test）で検証
- 回帰: 既存AgentModePanel/AgentSidebarの機能テストを維持
- 統合: Coordinator→Developer起動→完了検出→PR作成のE2Eフローを検証
