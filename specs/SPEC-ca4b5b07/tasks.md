# タスクリスト: Issue タブ — GitHub Issue 一覧・詳細・フルフロー

## 依存関係

- US1（一覧表示）→ US2（フィルタ）、US3（open/closed）、US7（リフレッシュ）は US1 の一覧基盤に依存
- US4（詳細表示）→ US1 の一覧からの遷移に依存、GFM Markdown 基盤に依存
- US5（フルフロー）→ US4 の詳細ビューに依存
- US6（GitHub で開く）→ US4 の詳細ビューに依存

## Phase 1: セットアップ

- [x] T001 [P] [US1] TypeScript 型定義の追加（GitHubLabel, GitHubAssignee, GitHubMilestone, GitHubIssueInfo 拡張, Tab type "issues"） `gwt-gui/src/lib/types.ts`
- [x] T002 [P] [US4] GFM Markdown ライブラリの導入（marked + dompurify） `gwt-gui/package.json`

## Phase 2: 基盤（バックエンド拡張）

- [x] T003 [US1] GitHubIssueInfo / GitHubLabel 構造体の拡張（body, assignees, comments_count, milestone, state, html_url, label.color） `crates/gwt-tauri/src/commands/issue.rs`
- [x] T004 [US1] fetch_github_issues に state パラメータ（open/closed）を追加 `crates/gwt-tauri/src/commands/issue.rs`
- [x] T005 [US4] fetch_github_issue_detail コマンドの新規追加（単一 Issue 取得） `crates/gwt-tauri/src/commands/issue.rs`
- [x] T006 [US1] fetch_github_issue_detail をコマンド登録 `crates/gwt-tauri/src/app.rs`
- [x] T007 [US1] バックエンド拡張のユニットテスト（型シリアライズ・state パラメータ・detail コマンド） `crates/gwt-tauri/src/commands/issue.rs`
- [x] T008 [US1] Git メニューに「Issues」メニュー項目を追加 `crates/gwt-tauri/src/menu.rs`

## Phase 3: GFM Markdown レンダリング（US4 基盤）

- [x] T009 [US4] MarkdownRenderer テストを作成（GFM 各要素・XSS サニタイズ） `gwt-gui/src/lib/components/MarkdownRenderer.test.ts`
- [x] T010 [US4] MarkdownRenderer.svelte コンポーネントの実装（marked + DOMPurify） `gwt-gui/src/lib/components/MarkdownRenderer.svelte`

## Phase 4: US1 — Issue 一覧の表示

- [x] T011 [US1] IssueListPanel テストを作成（一覧レンダリング・無限スクロール・gh CLI エラー・空状態） `gwt-gui/src/lib/components/IssueListPanel.test.ts`
- [x] T012 [US1] IssueListPanel.svelte 一覧ビューの実装（リッチ表示・ローディング・エラー・空状態） `gwt-gui/src/lib/components/IssueListPanel.svelte`
- [x] T013 [US1] 無限スクロールの実装（IntersectionObserver） `gwt-gui/src/lib/components/IssueListPanel.svelte`
- [x] T014 [US1] worktree 紐づきインジケーターの実装（find_existing_issue_branch API） `gwt-gui/src/lib/components/IssueListPanel.svelte`
- [x] T015 [US1] App.svelte にメニューアクション・シングルトンタブ管理を追加 `gwt-gui/src/App.svelte`
- [x] T016 [US1] MainArea.svelte に IssueListPanel タブレンダリングを追加 `gwt-gui/src/lib/components/MainArea.svelte`

## Phase 5: US2 — フィルタリング + US3 — open/closed + US7 — リフレッシュ

- [x] T017 [US2] フィルタリングテストを追加（テキスト検索・ラベルフィルタ） `gwt-gui/src/lib/components/IssueListPanel.test.ts`
- [x] T018 [US2] テキスト検索バーの実装 `gwt-gui/src/lib/components/IssueListPanel.svelte`
- [x] T019 [US2] ラベルフィルタ（クリックトグル）の実装 `gwt-gui/src/lib/components/IssueListPanel.svelte`
- [x] T020 [US3] open/closed トグルテストを追加 `gwt-gui/src/lib/components/IssueListPanel.test.ts`
- [x] T021 [US3] open/closed トグルボタンの実装 `gwt-gui/src/lib/components/IssueListPanel.svelte`
- [x] T022 [US7] 手動リフレッシュボタンの実装 `gwt-gui/src/lib/components/IssueListPanel.svelte`

## Phase 6: US4 — Issue 詳細表示

- [x] T023 [US4] 詳細ビューテストを追加（遷移・戻る・フィルタ保持・メタ情報・Markdown・Spec Issue 判定） `gwt-gui/src/lib/components/IssueListPanel.test.ts`
- [x] T024 [US4] 一覧→詳細切替ナビゲーション（戻るボタン・フィルタ状態保持）の実装 `gwt-gui/src/lib/components/IssueListPanel.svelte`
- [x] T025 [US4] Issue 詳細ヘッダー（タイトル・ステータス・ラベル・アサイニー・マイルストーン・コメント数）の実装 `gwt-gui/src/lib/components/IssueListPanel.svelte`
- [x] T026 [US4] Issue 本文の MarkdownRenderer による GFM レンダリングの実装 `gwt-gui/src/lib/components/IssueListPanel.svelte`
- [x] T027 [US4] Spec Issue 判定（spec ラベル）→ IssueSpecPanel セクション解析ビュー表示の実装 `gwt-gui/src/lib/components/IssueListPanel.svelte`

## Phase 7: US5 — フルフロー + US6 — GitHub で開く

- [x] T028 [US5] フルフロー連携テストを追加（Work on this・prefix 推定・Switch to worktree） `gwt-gui/src/lib/components/IssueListPanel.test.ts`
- [x] T029 [US5] App.svelte に Issue → AgentLaunchForm 起動インターフェースを追加 `gwt-gui/src/App.svelte`
- [x] T030 [US5] AgentLaunchForm に Issue プリフィルロジック追加（prefix 推定・suffix 生成・全項目） `gwt-gui/src/lib/components/AgentLaunchForm.svelte`
- [x] T031 [US5] 「Work on this」ボタンの実装 `gwt-gui/src/lib/components/IssueListPanel.svelte`
- [x] T032 [US5] 紐づき worktree 存在時の「Switch to worktree」ボタン切替の実装 `gwt-gui/src/lib/components/IssueListPanel.svelte`
- [x] T033 [US6] 「Open in GitHub」ボタンの実装 `gwt-gui/src/lib/components/IssueListPanel.svelte`

## Phase 8: 仕上げ・横断

- [x] T034 [P] [共通] cargo clippy --all-targets --all-features -- -D warnings `crates/`
- [x] T035 [P] [共通] cargo test 全パス確認 `crates/`
- [x] T036 [P] [共通] cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json `gwt-gui/`
- [x] T037 [P] [共通] cd gwt-gui && pnpm test 全パス確認 `gwt-gui/`
