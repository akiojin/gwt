# タスク: セッション変換一覧の内容表示とSpaceプレビュー

**入力**: `/specs/SPEC-330430b9/` からの設計ドキュメント
**前提条件**: plan.md（必須）、spec.md、research.md、data-model.md、quickstart.md

## フェーズ1: セットアップ（共有インフラストラクチャ）

- [ ] T001 [P] [共通] プレビュー状態・一覧表示用の共通ヘルパーを追加 `crates/gwt-cli/src/tui/screens/wizard.rs`

## フェーズ2: ユーザーストーリー1 - 一覧で開始メッセージを識別できる (優先度: P1)

**ストーリー**: セッション変換一覧にセッション名・開始ユーザーメッセージ・更新日時を表示

**価値**: セッションIDなしでも対象を識別できる

### テスト（TDD）

- [ ] T101 [P] [US1] 一覧表示フォーマット（セッション名+メッセージ+更新日時）のユニットテストを追加 `crates/gwt-cli/src/tui/screens/wizard.rs`
- [ ] T102 [P] [US1] Worktree名抽出ロジックのユニットテストを追加 `crates/gwt-cli/src/tui/screens/wizard.rs`

### 実装

- [ ] T103 [US1] 開始ユーザーメッセージ/Worktree名抽出ロジックを追加 `crates/gwt-cli/src/tui/screens/wizard.rs`
- [ ] T104 [US1] `load_sessions_for_agent` で一覧表示用の `display` を生成（セッション名を含む） `crates/gwt-cli/src/tui/screens/wizard.rs`
- [ ] T105 [US1] `render_convert_session_select_step` と幅計算を更新 `crates/gwt-cli/src/tui/screens/wizard.rs`

**✅ MVP1チェックポイント**: 一覧でセッション名・開始ユーザーメッセージ・更新日時が表示される

## フェーズ3: ユーザーストーリー2 - Spaceでセッション内容をプレビューできる (優先度: P1)

**ストーリー**: Spaceでプレビューを開閉し、先頭10メッセージを確認できる

**価値**: 変換前に内容確認ができる

### テスト（TDD）

- [ ] T201 [P] [US2] プレビュー生成/スクロールのユニットテストを追加 `crates/gwt-cli/src/tui/screens/wizard.rs`
- [ ] T202 [P] [US2] プレビューにセッション名が含まれるテストを追加 `crates/gwt-cli/src/tui/screens/wizard.rs`
- [ ] T203 [P] [US2] 変換中スピナー表示のユニットテストを追加 `crates/gwt-cli/src/tui/screens/wizard.rs`

### 実装

- [ ] T204 [US2] プレビュー状態（open/scroll/lines/error）を追加 `crates/gwt-cli/src/tui/screens/wizard.rs`
- [ ] T205 [US2] プレビュー描画（モーダル/フッター）を追加 `crates/gwt-cli/src/tui/screens/wizard.rs`
- [ ] T206 [US2] 変換中スピナーの状態管理と描画を追加 `crates/gwt-cli/src/tui/screens/wizard.rs`
- [ ] T207 [US2] Space/Esc/スクロールのキー処理とMessage追加 `crates/gwt-cli/src/tui/app.rs`

**✅ MVP2チェックポイント**: Spaceでプレビューが開閉し、先頭10メッセージが表示される

## フェーズ4: ユーザーストーリー3 - 失敗時でも一覧と変換が継続できる (優先度: P2)

**ストーリー**: 読み込み失敗時のプレースホルダ表示

**価値**: セッション欠落/破損でもフロー継続

### テスト（TDD）

- [ ] T301 [P] [US3] 失敗時プレースホルダのテストを追加 `crates/gwt-cli/src/tui/screens/wizard.rs`

### 実装

- [ ] T302 [US3] 一覧/プレビューのエラー表示を統一 `crates/gwt-cli/src/tui/screens/wizard.rs`

**✅ 完全な機能**: 失敗時も一覧と変換が継続する

## フェーズ5: ユーザーストーリー4 - セッション内容の欠落を最小化できる (優先度: P1)

**ストーリー**: 変換/プレビューで `tool_use` / `tool_result` / `thinking` / `parts` を欠落させない

**価値**: 変換結果の内容連続性を維持できる

### テスト（TDD）

- [ ] T501 [P] [US4] `tool_result` / `thinking` / `parts` の解析テストを追加 `crates/gwt-core/src/ai/session_parser/mod.rs`
- [ ] T502 [P] [US4] 画像/バイナリのプレースホルダ抽出テストを追加 `crates/gwt-core/src/ai/session_parser/mod.rs`

### 実装

- [ ] T503 [US4] セッションパーサーの内容抽出を拡張 `crates/gwt-core/src/ai/session_parser/mod.rs`

**✅ MVP3チェックポイント**: 主要なセッション要素が欠落せず抽出される

## フェーズ6: 統合とポリッシュ

- [ ] T601 [統合] `cargo test` を実行して失敗を修正 `crates/gwt-cli/src/tui/`
- [ ] T602 [統合] `bun run format:check` / `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` / `bun run lint` を実行して失敗を修正 `.`
