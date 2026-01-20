# タスク: ブランチサマリーパネル

**仕様ID**: `SPEC-4b893dae` | **日付**: 2026-01-19
**入力**: `specs/SPEC-4b893dae/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、data-model.md、contracts/openai-api.md、research.md、quickstart.md

## ストーリー依存関係

```text
US5 (パネル表示) ──┬──> US1 (コミットログ)
                  │
                  ├──> US2 (変更統計)
                  │
                  └──> US3 (メタデータ)
                            │
US6 (AI設定) ─────────────> US4 (AIサマリー)
```

- US5はすべてのセクションの基盤
- US1, US2, US3は並列実装可能（US5完了後）
- US4はUS6に依存

## フェーズ1: セットアップ（共有基盤）

**目的**: データ構造とモジュール構成の準備

### データモデル

- [x] **T001** [P] [共通] `crates/gwt-core/src/git/commit.rs` を新規作成し、CommitEntry構造体を定義
- [x] **T002** [P] [共通] `crates/gwt-core/src/git/commit.rs` にChangeStats構造体を追加
- [x] **T003** [P] [共通] `crates/gwt-core/src/git/commit.rs` にBranchMeta構造体を追加
- [x] **T004** [共通] `crates/gwt-core/src/git.rs` にcommitモジュールをエクスポート追加

### UIコンポーネント基盤

- [x] **T005** [共通] `crates/gwt-cli/src/tui/components.rs` にSummaryPanelを追加（既存パターンに合わせてcomponents.rsに統合）
- [x] **T006** [共通] SummaryPanelコンポーネントの骨格を定義（render, build_sections等）

## フェーズ2: US5 - パネルの常時表示 (優先度: P0)

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

**チェックポイント**: パネル枠とタイトルが表示される（内容は空） ✅ 完了

## フェーズ3: US1 - コミットログの確認 (優先度: P0)

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

**チェックポイント**: コミットログがパネルに表示される ✅ 完了

## フェーズ4: US2 - 変更統計の確認 (優先度: P0)

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

**チェックポイント**: 変更統計がパネルに表示される（既存の安全性判定と一致） ✅ 完了

## フェーズ5: US3 - ブランチメタデータの確認 (優先度: P1)

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

**チェックポイント**: メタデータがパネルに表示される ✅ 完了

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

**チェックポイント**: プロファイルYAMLにai設定が保存・読み込みできる

## フェーズ7: US4 - AIサマリーの確認 (優先度: P2)

**ストーリー**: AIがコミット履歴を分析して2-3行の箇条書きサマリーを表示

**価値**: コミットメッセージを読まなくても作業内容の概要を把握できる

### AIモジュール基盤

- [ ] **T601** [US4] `crates/gwt-core/src/ai/mod.rs` を新規作成
- [ ] **T602** [US4] T601の後に `crates/gwt-core/src/ai/client.rs` を新規作成し、OpenAI互換APIクライアントを実装
- [ ] **T603** [US4] T602の後に `crates/gwt-core/src/ai/client.rs` にエラーハンドリング（AIError enum）を追加
- [ ] **T604** [US4] T603の後に `crates/gwt-core/src/ai/client.rs` にタイムアウトとリトライロジックを追加

### サマリー生成

- [ ] **T605** [US4] T604の後に `crates/gwt-core/src/ai/summary.rs` を新規作成し、サマリー生成ロジックを実装
- [ ] **T606** [US4] T605の後に `crates/gwt-core/src/ai/summary.rs` にプロンプトテンプレートを追加
- [ ] **T607** [US4] T606の後に `crates/gwt-core/src/ai/summary.rs` にAISummaryCacheを実装

### バッチプリフェッチ

- [ ] **T608** [US4] T607の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` に起動時のバッチプリフェッチ（Worktreeありブランチのみ）を実装

### UI表示

- [ ] **T609** [US4] T608の後に `crates/gwt-cli/src/tui/components/summary_panel.rs` に`Summary:`セクション描画を実装
- [ ] **T610** [US4] T609の後に `crates/gwt-cli/src/tui/components/summary_panel.rs` にAI設定無効時のセクション非表示を実装
- [ ] **T611** [US4] T610の後に `crates/gwt-cli/src/tui/components/summary_panel.rs` にAPIエラー時のグレースフルデグラデーションを実装

### 依存関係追加

- [ ] **T612** [US4] `crates/gwt-core/Cargo.toml` にreqwest依存を追加

**チェックポイント**: AIサマリーがパネルに表示される（設定有効時）

## フェーズ8: 統合と仕上げ

**目的**: エッジケース対応、テスト、ドキュメント

### エッジケース対応

- [x] **T701** [統合] `crates/gwt-cli/src/tui/components/summary_panel.rs` にWorktreeなしブランチの表示制御を追加 ✓ "No worktree" 表示実装済み
- [x] **T702** [統合] `crates/gwt-cli/src/tui/components/summary_panel.rs` にdetached HEAD時の表示を追加 ✓ ブランチ名がそのまま表示される
- [x] **T703** [統合] `crates/gwt-cli/src/tui/components/summary_panel.rs` にリモートブランチ時の表示を追加 ✓ upstream情報で対応
- [x] **T704** [統合] `crates/gwt-cli/src/tui/components/summary_panel.rs` にデータ取得失敗時の`(Failed to load)`表示を追加 ✓ errors フィールドでエラー表示対応
- [x] **T705** [統合] `crates/gwt-cli/src/tui/components/summary_panel.rs` に表示幅が狭い場合の末尾省略を追加 ✓ メッセージ50文字で省略実装済み

### 品質チェック

- [x] **T706** [統合] `cargo clippy --all-targets --all-features -- -D warnings` をローカルで完走させ、失敗時は修正 ✓
- [x] **T707** [統合] `cargo fmt --check` をローカルで完走させ、失敗時は修正 ✓
- [x] **T708** [統合] `cargo test` をローカルで完走させ、失敗時は修正 ✓
- [x] **T709** [統合] `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` をローカルで完走させ、失敗時は修正 ✓

### ドキュメント

- [x] **T710** [P] [ドキュメント] `README.md` にブランチサマリーパネル機能の説明を追加 ✓
- [x] **T711** [P] [ドキュメント] `README.ja.md` にブランチサマリーパネル機能の説明を追加（日本語） ✓

## タスク凡例

**優先度**:

- **P0**: 最重要 - 基本機能に必要（US1, US2, US5）
- **P1**: 重要 - 完全な機能に必要（US3）
- **P2**: 補完的 - 付加価値機能（US4, US6）

**タグ**:

- **[P]**: 並列実行可能
- **[US1]**: コミットログ
- **[US2]**: 変更統計
- **[US3]**: メタデータ
- **[US4]**: AIサマリー
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

## 推奨MVP範囲

**MVP1**: フェーズ1〜4（セットアップ + US5 + US1 + US2）

- パネル枠とタイトル表示
- コミットログ表示
- 変更統計表示

これにより、AIなしでも実用的なブランチサマリー機能を提供できる。
