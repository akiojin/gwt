# タスク: ブランチサマリーパネル（セッション要約対応）

**仕様ID**: `SPEC-4b893dae` | **日付**: 2026-01-19 | **更新日**: 2026-01-21
**入力**: `specs/SPEC-4b893dae/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、data-model.md、contracts/openai-api.md、research.md、quickstart.md

## 追加作業: サマリーパネルの内側余白 (2026-01-20)

- [x] **T8801** [P] [共通] `specs/SPEC-4b893dae/spec.md` / `specs/SPEC-4b893dae/plan.md` にパネル内左右余白の要件を追記
- [x] **T8802** [Test] `crates/gwt-cli/src/tui/components.rs` のSummaryPanel描画テストを追加し、枠内余白を検証
- [x] **T8803** [Impl] SummaryPanelとセッション要約パネルの枠内に左右余白を追加する
- [ ] **T8804** `cargo test -p gwt-cli` を実行し、失敗がないことを確認する

## 追加作業: セッション要約の途中切れ対策 (2026-01-21)

- [x] **T8901** [P] [共通] `specs/SPEC-4b893dae/spec.md` にポーリング再入防止/静止期間/不完全要約保持の要件と受け入れシナリオを追記
- [x] **T8902** [P] [共通] `specs/SPEC-4b893dae/plan.md` に途中切れ対策の実装方針とリスク緩和を追記
- [ ] **T8903** [US4] `crates/gwt-core/src/ai/summary.rs` に要約Markdownの完全性バリデーションを追加（目的/要約/ハイライト/箇条書きの検証）
- [ ] **T8904** [US4] T8903の後に 不完全要約をエラー扱いにして既存要約を保持できるよう調整（`crates/gwt-core/src/ai/summary.rs`）
- [ ] **T8905** [US4] `crates/gwt-cli/src/tui/app.rs` にポーリング再入防止（生成完了後に次回を再スケジュール）と静止期間チェックを追加
- [ ] **T8906** [US4] `crates/gwt-cli/src/tui/screens/branch_list.rs` に不完全要約時の警告表示（例: `Incomplete summary; keeping previous`）を追加
- [ ] **T8907** [Test] `crates/gwt-core/src/ai/summary.rs` に完全性バリデーションのユニットテストを追加
- [ ] **T8908** [Test] `crates/gwt-cli/src/tui/app.rs` にポーリング再入防止/静止期間のユニットテストを追加（モック時間）
- [ ] **T8909** `cargo test -p gwt-core` と `cargo test -p gwt-cli` を実行し、失敗がないことを確認する

## ストーリー依存関係

```text
US5 (パネル表示) ──┬──> US1 (コミットログ)     ✅ 完了
                  │
                  ├──> US2 (変更統計)         ✅ 完了
                  │
                  ├──> US3 (メタデータ)        ✅ 完了
                  │
                  └──> US4a (タブ切り替え) ──> US4b (パーサー) ──> US4 (セッション要約)
                                                                       │
US6 (AI設定) ─────────────────────────────────────────────────────────┘
```

**並列実行可能グループ**:

- US4bのパーサー（Claude, Codex, Gemini, OpenCode）は並列実装可能

## フェーズ1: セットアップ（共有基盤） ✅ 完了

**目的**: データ構造とモジュール構成の準備

### データモデル

- [x] **T001** [P] [共通] `crates/gwt-core/src/git/commit.rs` を新規作成し、CommitEntry構造体を定義
- [x] **T002** [P] [共通] `crates/gwt-core/src/git/commit.rs` にChangeStats構造体を追加
- [x] **T003** [P] [共通] `crates/gwt-core/src/git/commit.rs` にBranchMeta構造体を追加
- [x] **T004** [共通] `crates/gwt-core/src/git.rs` にcommitモジュールをエクスポート追加

### UIコンポーネント基盤

- [x] **T005** [共通] `crates/gwt-cli/src/tui/components.rs` にSummaryPanelを追加（既存パターンに合わせてcomponents.rsに統合）
- [x] **T006** [共通] SummaryPanelコンポーネントの骨格を定義（render, build_sections等）

## フェーズ2: US5 - パネルの常時表示 (優先度: P0) ✅ 完了

**ストーリー**: ブランチ選択画面のフッター領域に詳細パネルを常時表示

**価値**: ユーザーがブランチの詳細情報を常に確認できる基盤UI

### レイアウト変更

- [x] **T101** [US5] `crates/gwt-cli/src/tui/screens/branch_list.rs` のLayout制約を変更（Length(1)→Length(12)）
- [x] **T102** [US5] `crates/gwt-cli/src/tui/screens/branch_list.rs` にBranchSummaryフィールドを追加
- [x] **T103** [US5] LoadingState構造体は`commit.rs`で定義済み（Phase 1）

### パネル描画

- [x] **T104** [US5] `crates/gwt-cli/src/tui/components.rs` のSummaryPanelにパネル枠線描画を実装
- [x] **T105** [US5] SummaryPanelにタイトル`[branch_name] Details`表示を実装
- [x] **T106** [US5] `crates/gwt-cli/src/tui/screens/branch_list.rs` にrender_summary_panel関数を追加（render_worktree_pathを置き換え）

### 状態管理

- [x] **T107** [US5] render_summary_panel関数でブランチ選択に応じたサマリー生成を実装

**✅ チェックポイント**: パネル枠とタイトルが表示される

## フェーズ3: US1 - コミットログの確認 (優先度: P0) ✅ 完了

**ストーリー**: 直近3-5件のコミット履歴をハッシュ+メッセージ形式で表示

**価値**: ブランチで何が行われたかを素早く把握できる

### Git操作

- [x] **T201** [US1] `crates/gwt-core/src/git/repository.rs` に`get_commit_log(branch: &str, limit: usize) -> Vec<CommitEntry>`関数を追加
- [x] **T202** [US1] T201の後に `crates/gwt-core/src/git/commit.rs` にCommitEntry::from_oneline()パーサーを実装

### UI表示

- [x] **T203** [US1] T202の後に `crates/gwt-cli/src/tui/components.rs` に`Commits:`セクション描画を実装
- [x] **T204** [US1] T203の後に `crates/gwt-cli/src/tui/components.rs` にコミットメッセージの末尾省略(`...`)処理を追加
- [x] **T205** [US1] T204の後に `crates/gwt-cli/src/tui/components.rs` に`No commits yet`表示を追加

### 非同期取得

- [x] **T206** [US1] T205の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` にコミットログの取得を実装（選択変更時に同期取得）
- [x] **T207** [US1] T206の後に `crates/gwt-cli/src/tui/components.rs` にローディング表示（ASCIIスピナー）を追加

**✅ チェックポイント**: コミットログがパネルに表示される

## フェーズ4: US2 - 変更統計の確認 (優先度: P0) ✅ 完了

**ストーリー**: 変更ファイル数、行数、未コミット/未プッシュ状態を表示

**価値**: ブランチの作業量と安全性を定量的に把握できる

### Git操作

- [x] **T301** [P] [US2] `crates/gwt-core/src/git/repository.rs` に`get_diff_stats(worktree_path: &Path) -> ChangeStats`関数を追加
- [x] **T302** [US2] T301の後に `crates/gwt-core/src/git/commit.rs` にChangeStats::from_shortstat()パーサーを実装

### データ統合

- [x] **T303** [US2] T302の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` で既存のhas_changes/has_unpushedをChangeStatsに統合

### UI表示

- [x] **T304** [US2] T303の後に `crates/gwt-cli/src/tui/components.rs` に`Stats:`セクション描画を実装
- [x] **T305** [US2] T304の後に `crates/gwt-cli/src/tui/components.rs` に`Uncommitted changes`/`Unpushed commits`表示を追加

**✅ チェックポイント**: 変更統計がパネルに表示される

## フェーズ5: US3 - ブランチメタデータの確認 (優先度: P1) ✅ 完了

**ストーリー**: ahead/behind、ベースブランチ、最終コミット日時を表示

**価値**: upstreamとの同期状態や最後の作業日時を把握できる

### データ変換

- [x] **T401** [US3] `crates/gwt-core/src/git/commit.rs` にBranchMeta::from_branch()変換関数を実装
- [x] **T402** [US3] T401の後に `crates/gwt-core/src/git/commit.rs` にBranchMeta::relative_time()関数を実装（std::time使用）

### UI表示

- [x] **T403** [US3] T402の後に `crates/gwt-cli/src/tui/components.rs` に`Meta:`セクション描画を実装
- [x] **T404** [US3] T403の後に `crates/gwt-cli/src/tui/components.rs` にahead/behind表示（`+N -M from <upstream>`）を追加
- [x] **T405** [US3] T404の後に `crates/gwt-cli/src/tui/components.rs` に最終コミット日時の相対表示を追加
- [x] **T406** [US3] T405の後に `crates/gwt-cli/src/tui/components.rs` にupstream未設定時の表示制御を追加

**✅ チェックポイント**: メタデータがパネルに表示される

## フェーズ6: US6 - プロファイルへのAI設定追加 (優先度: P2)

**ストーリー**: プロファイルにAI設定（エンドポイント、APIキー、モデル）を追加

**価値**: プロジェクトごとに異なるAI設定を使用できる

### データモデル

- [ ] **T501** [US6] `crates/gwt-core/src/config/profile.rs` にAISettings構造体を追加
- [ ] **T502** [US6] T501の後に `crates/gwt-core/src/config/profile.rs` のProfile構造体に`ai: Option<AISettings>`フィールドを追加

### デフォルト値と環境変数

- [ ] **T503** [US6] T502の後に `crates/gwt-core/src/config/profile.rs` にdefault_endpoint()、default_model()関数を追加
- [ ] **T504** [US6] T503の後に `crates/gwt-core/src/config/profile.rs` に環境変数フォールバック（OPENAI_API_KEY等）を実装

### シリアライズ

- [ ] **T505** [US6] T504の後に `crates/gwt-core/src/config/profile.rs` のYAMLシリアライズにai設定を追加
- [x] **T506** [US6] T505の後に `crates/gwt-core/src/config/profile.rs` のAI有効判定を更新（エンドポイント/モデル必須、APIキー任意）
- [x] **T507** [US6] T506の後に `crates/gwt-cli/src/tui/screens/environment.rs` のAIプレースホルダーを必須/任意表記に更新

**チェックポイント**: プロファイルYAMLにai設定が保存・読み込みできる

## フェーズ7: US4a - Tabキーによるタブ切り替え (優先度: P0)

**ストーリー**: TabキーでブランチとセッションSummaryタブを切り替えられる

**価値**: ブランチ詳細とセッション要約を両方確認できる

### 状態管理

- [ ] **T551** [US4a] `crates/gwt-cli/src/tui/screens/branch_list.rs` に `DetailPanelTab` enum を追加（Details, Session）
- [ ] **T552** [US4a] T551の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` の `BranchListScreen` に `detail_panel_tab` フィールドを追加

### キーハンドリング

- [ ] **T553** [US4a] T552の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` に Tabキーハンドラを追加
- [ ] **T554** [US4a] T553の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` で `detail_panel_tab` を切り替え

### タブ表示

- [ ] **T555** [US4a] T554の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` でタブ状態に応じてタイトルを切り替え（`Details` / `Session`）
- [ ] **T556** [US4a] T555の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` でタブ状態に応じて描画メソッドを切り替え
- [x] **T557** [US4a] T556の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` でブランチ切り替え時にタブ状態を維持（グローバル）
- [x] **T558** [US4a] T557の後に ブランチ詳細/セッション要約パネル枠へTab切り替えヒントを追加

### ブランチ単位のタブ記憶 (FR-074シリーズ)

- [x] **T561** [US4a] `crates/gwt-cli/src/tui/screens/branch_list.rs` に `branch_tab_cache: HashMap<String, DetailPanelTab>` フィールドを追加
- [x] **T562** [US4a] T561の後に `get_tab_for_branch(&self, branch_name: &str) -> DetailPanelTab` メソッドを実装（キャッシュヒット時は記憶値、ミス時はDetails）
- [x] **T563** [US4a] T562の後に `set_tab_for_branch(&mut self, branch_name: &str, tab: DetailPanelTab)` メソッドを実装
- [x] **T564** [US4a] T563の後に Tab切り替え時（Tabキー押下時）に即座に `set_tab_for_branch` を呼び出すよう修正
- [x] **T565** [US4a] T564の後に ブランチ選択変更時に `get_tab_for_branch` でタブ状態を復元するよう修正
- [x] **T566** [US4a] T565の後に rキーリフレッシュ・自動更新時に `branch_tab_cache` を維持するよう修正
- [x] **T567** [US4a] T566の後に ブランチリスト更新時に削除されたブランチのタブ記憶を破棄する `cleanup_branch_tab_cache` メソッドを実装
- [x] **T568** [US4a] T567の後に 選択中ブランチが削除された場合、先頭ブランチに選択を移動するよう修正
- [x] **T569** [Test] T568の後に ブランチ単位タブ記憶のユニットテストを追加（branch_list.rs内）

**チェックポイント**: Tabキーでタブ切り替えが動作する＋ブランチ単位でタブ状態が記憶される

## フェーズ8: US4b - セッションパーサー (優先度: P0)

**ストーリー**: gwtのsession_idを使用してエージェントセッションを特定・解析する

**価値**: 4種類のエージェントセッション内容を抽出できる

### モジュール構成

- [ ] **T601** [US4b] `crates/gwt-core/src/ai/mod.rs` に `session_parser` モジュールを追加
- [ ] **T602** [US4b] T601の後に `crates/gwt-core/src/ai/session_parser/mod.rs` を新規作成

### トレイト定義

- [ ] **T603** [US4b] T602の後に `crates/gwt-core/src/ai/session_parser/mod.rs` に `SessionParser` トレイトを定義
- [ ] **T604** [US4b] T603の後に `crates/gwt-core/src/ai/session_parser/mod.rs` に `ParsedSession` 構造体を定義
- [ ] **T605** [US4b] T604の後に `crates/gwt-core/src/ai/session_parser/mod.rs` に `AgentType` enum を定義
- [ ] **T606** [US4b] T605の後に `crates/gwt-core/src/ai/session_parser/mod.rs` に `SessionParseError` enum を定義

### Claude Code パーサー

- [ ] **T611** [P] [US4b] T606の後に `crates/gwt-core/src/ai/session_parser/claude.rs` に `ClaudeSessionParser` を実装
- [ ] **T612** [US4b] T611の後に `crates/gwt-core/src/ai/session_parser/claude.rs` で JSONL 形式を解析
- [ ] **T613** [US4b] T612の後に `crates/gwt-core/src/ai/session_parser/claude.rs` で会話履歴とツール実行履歴を抽出

### Codex CLI パーサー

- [ ] **T621** [P] [US4b] T606の後に `crates/gwt-core/src/ai/session_parser/codex.rs` に `CodexSessionParser` を実装
- [ ] **T622** [US4b] T621の後に `crates/gwt-core/src/ai/session_parser/codex.rs` で JSONL 形式を解析

### Gemini CLI パーサー

- [ ] **T631** [P] [US4b] T606の後に `crates/gwt-core/src/ai/session_parser/gemini.rs` に `GeminiSessionParser` を実装
- [ ] **T632** [US4b] T631の後に `crates/gwt-core/src/ai/session_parser/gemini.rs` で JSON 形式を解析

### OpenCode パーサー

- [ ] **T641** [P] [US4b] T606の後に `crates/gwt-core/src/ai/session_parser/opencode.rs` に `OpenCodeSessionParser` を実装
- [ ] **T642** [US4b] T641の後に `crates/gwt-core/src/ai/session_parser/opencode.rs` で JSON 形式を解析

### 動的サンプリング

- [ ] **T651** [US4b] T613, T622, T632, T642の後に `crates/gwt-core/src/ai/session_parser/mod.rs` に動的サンプリング関数を実装（1000ターン以上対応）

**チェックポイント**: 4エージェントのセッションを解析できる

## フェーズ9: US4 - セッション要約の確認 (優先度: P0)

**ストーリー**: セッション要約タブに切り替えると、エージェントセッションのAI要約が表示される

**価値**: エージェントの作業状況をリアルタイムで把握できる

### データ構造

- [ ] **T701** [US4] `crates/gwt-core/src/ai/summary.rs` に `SessionSummary` 構造体を追加（task_overview, short_summary, bullet_points, metrics）
- [ ] **T702** [US4] T701の後に `crates/gwt-core/src/ai/summary.rs` に `SessionMetrics` 構造体を追加（token_count, tool_calls, elapsed_time）
- [ ] **T703** [US4] T702の後に `crates/gwt-core/src/ai/summary.rs` に `SessionSummaryCache` 構造体を追加

### AI要約生成

- [ ] **T711** [US4] T703の後に `crates/gwt-core/src/ai/summary.rs` にセッション要約用システムプロンプトを追加
- [ ] **T712** [US4] T711の後に `crates/gwt-core/src/ai/summary.rs` に `summarize_session(parsed: &ParsedSession) -> SessionSummary` を実装
- [ ] **T713** [US4] T712の後に `crates/gwt-core/src/ai/client.rs` で MAX_TOKENS を調整（150 → 300-500）
- [x] **T714** [US4] T711の後に システムプロンプトへ言語推定の指示を追加
- [x] **T715** [US4] T712の後に `build_session_prompt` の入力サイズ上限を追加
- [x] **T716** [US4] T715の後に 入力サイズ上限のテストを追加

### UI表示

- [ ] **T721** [US4] T713の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` にセッション要約タブの描画メソッドを実装
- [ ] **T722** [US4] T721の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` でタスク概要・短文要約・バレットポイント・メトリクスを表示
- [ ] **T723** [US4] T722の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` でローディングスピナーを表示
- [x] **T724** [US4] T723の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` で要約テキストの折り返し/スクロールを追加
- [x] **T725** [US4] T724の後に 既存要約がある場合は再生成中も表示し続けるように調整

### エラーハンドリング

- [ ] **T731** [US4] T723の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` で AI設定無効時に設定促進メッセージを表示
- [ ] **T732** [US4] T731の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` で セッションなし時に `No session` を表示

### ポーリング更新

- [ ] **T741** [US4] T732の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` に30秒ポーリングスレッドを実装
- [ ] **T742** [US4] T741の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` でファイル変更検出時に自動再生成
- [ ] **T743** [US4] T742の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` でセッションタブ非表示時にポーリング停止
- [ ] **T744** [US4] T743の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` でポーリングエラー時にキャッシュ表示継続

### キャッシュ

- [ ] **T751** [US4] T744の後に `crates/gwt-core/src/ai/summary.rs` に `SessionSummaryCache` の get/set メソッドを実装
- [ ] **T752** [US4] T751の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` でキャッシュを活用

**チェックポイント**: セッション要約がタブに表示される

## フェーズ10: 統合と仕上げ

**目的**: エッジケース対応、テスト、ドキュメント

### エッジケース対応（完了済み）

- [x] **T801** [統合] `crates/gwt-cli/src/tui/components.rs` にWorktreeなしブランチの表示制御を追加
- [x] **T802** [統合] `crates/gwt-cli/src/tui/components.rs` にdetached HEAD時の表示を追加
- [x] **T803** [統合] `crates/gwt-cli/src/tui/components.rs` にリモートブランチ時の表示を追加
- [x] **T804** [統合] `crates/gwt-cli/src/tui/components.rs` にデータ取得失敗時の`(Failed to load)`表示を追加
- [x] **T805** [統合] `crates/gwt-cli/src/tui/components.rs` に表示幅が狭い場合の末尾省略を追加

### セッション要約エッジケース（新規）

- [ ] **T811** [統合] `crates/gwt-core/src/ai/session_parser/mod.rs` でセッションファイル破損時のエラーハンドリングを実装
- [ ] **T812** [統合] `crates/gwt-core/src/ai/session_parser/mod.rs` でファイルロック時のリトライを実装
- [ ] **T813** [統合] `crates/gwt-cli/src/tui/screens/branch_list.rs` で長いセッション読み込み中のUI応答性を確保

### 品質チェック（完了済み - 再確認必要）

- [x] **T821** [統合] `cargo clippy --all-targets --all-features -- -D warnings` をローカルで完走させ、失敗時は修正
- [x] **T822** [統合] `cargo fmt --check` をローカルで完走させ、失敗時は修正
- [x] **T823** [統合] `cargo test` をローカルで完走させ、失敗時は修正
- [x] **T824** [統合] `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` をローカルで完走させ、失敗時は修正

### ドキュメント

- [x] **T831** [P] [ドキュメント] `README.md` にブランチサマリーパネル機能の説明を追加
- [x] **T832** [P] [ドキュメント] `README.ja.md` にブランチサマリーパネル機能の説明を追加（日本語）
- [ ] **T833** [P] [ドキュメント] `README.md` にセッション要約機能の説明を追加
- [ ] **T834** [P] [ドキュメント] `README.ja.md` にセッション要約機能の説明を追加（日本語）

## タスク凡例

**優先度**:

- **P0**: 最重要 - 基本機能に必要（US1, US2, US4, US4a, US4b, US5）
- **P1**: 重要 - 完全な機能に必要（US3）
- **P2**: 補完的 - 付加価値機能（US6）

**タグ**:

- **[P]**: 並列実行可能
- **[US1]**: コミットログ
- **[US2]**: 変更統計
- **[US3]**: メタデータ
- **[US4]**: セッション要約
- **[US4a]**: タブ切り替え
- **[US4b]**: セッションパーサー
- **[US5]**: パネル表示
- **[US6]**: AI設定
- **[共通]**: 全ストーリー共通
- **[統合]**: 複数ストーリーにまたがる
- **[ドキュメント]**: ドキュメント専用

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## 要約

| 項目 | 値 |
|------|-----|
| 総タスク数 | 83 |
| 完了済み | 40 |
| 未完了 | 43 |
| フェーズ1-5（完了済み） | 26 |
| フェーズ6（US6: AI設定） | 5 |
| フェーズ7（US4a: タブ切り替え） | 16 (含むブランチ単位タブ記憶9件) |
| フェーズ8（US4b: パーサー） | 17 |
| フェーズ9（US4: セッション要約） | 16 |
| フェーズ10（統合） | 9 |
| 並列実行可能タスク | 8 |

## 推奨実装順序

1. **US4a** (タブ切り替え) - UIの切り替え基盤
2. **US4b** (セッションパーサー) - 4エージェント対応
3. **US4** (セッション要約) - AI要約とポーリング
4. **US6** (AI設定) - プロファイル連携

MVP範囲: US4a → US4b（Claude Codeのみ先行） → US4（基本機能）
