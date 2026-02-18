# TDD計画: プロジェクトチーム（Project Team）

**仕様ID**: `SPEC-ba3f610c` | **日付**: 2026-02-19

## テスト戦略

- バックエンド（Rust）: 3層状態モデル、Lead Spec Kitワークフロー、成果物ゲート、Coordinator管理、Developer完了検出、CI監視ループ、セッション永続化をユニットテストで検証
- フロントエンド（Svelte/TypeScript）: Kanbanボード、Leadチャット、パネル切替、タスクカード、Coordinatorパネルをコンポーネントテスト（vitest + @testing-library/svelte）で検証
- 各タスクはテストファーストで進める（RED → 実装 → GREEN）

## RED → GREEN の対象

### Phase A: 基盤モデル

1. 3層状態モデル（Session / Lead / Coordinator / Developer / Task）の型定義とenum遷移テスト
2. フロント型定義（LeadState / CoordinatorState / DeveloperState / TaskCard / KanbanColumn）の整合テスト
3. PTY通信スキルインターフェースの入出力テスト

### Phase B: Lead機能

4. Leadチャット（IME送信抑止 / 送信中スピナー / 自動スクロール）回帰テスト
5. Spec Kit LLMプロンプトテンプレートの読み込み・実行テスト
6. Spec Kitワークフロー順次実行テスト（clarify → specify → plan → tasks → tdd）
7. 成果物4点ゲートテスト（spec.md/plan.md/tasks.md/tdd.md いずれか欠損 → Coordinator起動ブロック）
8. 計画提示・承認フローテスト（承認 → Coordinator起動 / 拒否 → 再策定）
9. GitHub Issue作成・Project登録テスト
10. 段階的委譲テスト（自律可能操作 vs 承認要求操作の分類）
11. ハイブリッド常駐テスト（イベント駆動トリガー + 2分間隔ポーリング）
12. Lead実行基盤切り替えテスト（gwt内蔵AI / Claude Code）

### Phase C: Coordinator機能

13. Coordinator起動テスト（GUI内蔵ターミナルペイン割当）
14. タスク分割テスト（独立タスク → 別Worktree / 依存タスク → 同一 or merge連携）
15. Worktree/ブランチ自動作成テスト（agent/プレフィックス、命名サニタイズ、連番付与）
16. Developer起動テスト（PTY送信、全自動モードフラグ、CLAUDE.md規約含むプロンプト）
17. Developer完了検出テスト（Hook Stop / GWT_TASK_DONE / プロセス終了）
18. CI監視・自律修正ループテスト（CI失敗 → Developer修正 → 再プッシュ、最大3回 → Lead報告）
19. 成果物検証テスト（テスト実行 → パス → PR作成 / 失敗 → リトライ最大3回）
20. Developer間コンテキスト共有テスト（先行タスクmerge → コンフリクト検出）

### Phase D: GUI

21. Project Teamタブ・モード切り替えテスト
22. ダッシュボード表示テスト（Issue/Task/Developer階層、ステータスバッジ、折りたたみ）
23. Issue展開・Coordinator詳細テスト（CI結果、Developer一覧、ターミナル/チャットリンク）
24. TaskクリックでBranch Modeジャンプテスト（Worktree自動遷移）
25. Coordinator詳細パネル表示テスト（状態 / Developer一覧 / View Terminal / Chat）
26. コスト可視化テスト（APIコール数 / 推定トークン数）
27. AI設定未構成時エラー表示テスト

### Phase E: セッション・障害

28. セッション永続化テスト（全層状態のJSON保存 / トリガー検証）
29. セッション復元・再開テスト（gwt再起動 → 最新未完了セッション復元）
30. 層間独立性テスト（Lead API障害時のCoordinator/Developer続行）
31. Coordinator自律再起動テスト（クラッシュ検出 → 30秒以内再起動 → 状態再取得）
32. セッション強制中断テスト（Esc → SIGTERM → 5秒タイムアウト → Paused保存）
33. ブランチモード連携テスト（agent/ブランチ表示 / 削除時Failed検出）
34. コンテキスト要約・圧縮テスト

### Phase F: スキル化

35. PTY通信スキル化テスト（send_keys_to_pane / send_keys_broadcast / capture_scrollback_tail）
36. agent_tools.rs完全移行テスト（旧ツール呼び出しの廃止確認）
37. 直接アクセステスト（Developerターミナル直操作 / Coordinatorチャット入力）

## テスト実行コマンド

### バックエンド

- `cargo test -p gwt-core agent -- --nocapture`
- `cargo test -p gwt-tauri agent_master -- --nocapture`
- `cargo test -p gwt-tauri commands::agent_mode -- --nocapture`
- `cargo test -p gwt-tauri commands::terminal -- --nocapture`

### フロントエンド

- `cd gwt-gui && pnpm test src/lib/components/ProjectTeamPanel.test.ts`
- `cd gwt-gui && pnpm test src/lib/components/LeadChat.test.ts`
- `cd gwt-gui && pnpm test src/lib/components/Dashboard.test.ts`
- `cd gwt-gui && pnpm test src/lib/components/IssueItem.test.ts`
- `cd gwt-gui && pnpm test src/lib/components/CoordinatorDetail.test.ts`

## 実行ログ

> 以下は実装進行に伴い更新する。

- [ ] Phase A: 基盤モデルテスト
- [ ] Phase B: Lead機能テスト
- [ ] Phase C: Coordinator機能テスト
- [ ] Phase D: GUIテスト
- [ ] Phase E: セッション・障害テスト
- [ ] Phase F: スキル化テスト
