# タスク: GitView画面

**入力**: `/specs/SPEC-1ea18899/` からの設計ドキュメント
**前提条件**: plan.md（必須）、spec.md（必須）、research.md、data-model.md、quickstart.md

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（US1〜US4）
- 説明に正確なファイルパスを含める

## ストーリー依存関係

```text
US4 (Detailsパネル削除) ─┐
                         ├─> US1 (GitView基本) ─> US2 (PRリンク)
US3 (ワークツリーなし)  ─┘
```

- **US1 + US4**: P1（同時実装が必要、GitView画面の骨格 + Detailsパネル削除）
- **US2**: P2（US1完了後、PRリンクのブラウザ起動）
- **US3**: P2（US1完了後、ワークツリーなしブランチ対応）

## フェーズ1: セットアップ（共有インフラストラクチャ）

**目的**: Screen/Message enum の拡張、データ構造の定義

### セットアップタスク

- [x] **T001** [共通] `crates/gwt-cli/src/tui/app.rs` の Screen enum に `GitView` バリアントを追加
- [x] **T002** [共通] T001の後に `crates/gwt-cli/src/tui/screens/mod.rs` に `git_view` モジュールを追加
- [x] **T003** [共通] T002の後に `crates/gwt-cli/src/tui/screens/git_view.rs` を新規作成し、FileStatus/FileEntry/CommitEntry/GitViewState/GitViewCache/GitViewData 構造体を定義

## フェーズ2: ユーザーストーリー1+4 - GitView画面基本 + Detailsパネル削除 (優先度: P1)

**ストーリー**: ユーザーはブランチ一覧で`v`キーを押すとGitView画面を表示し、詳細情報を確認できる。同時にDetailsパネルを削除し2ペイン構成に変更する。

**価値**: ブランチのgit状態把握という中核機能と、UIの簡素化を同時に実現

### テスト（TDD）

- [x] **T101** [P] [US1] `crates/gwt-cli/src/tui/screens/git_view.rs` に GitViewState の select_next/select_prev/toggle_expand のユニットテストを作成
- [x] **T102** [P] [US1] `crates/gwt-cli/src/tui/screens/git_view.rs` に GitViewCache の get/insert/clear のユニットテストを作成

### 状態管理

- [x] **T103** [US1] T003の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に GitViewState::new() コンストラクタを実装
- [x] **T104** [US1] T103の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に select_next/select_prev/toggle_expand メソッドを実装
- [x] **T105** [US1] T103の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に total_item_count() メソッドを実装（シームレスナビゲーション用）

### Model拡張

- [x] **T106** [US1] T003の後に `crates/gwt-cli/src/tui/app.rs` の Model に git_view: GitViewState フィールドを追加
- [x] **T107** [US1] T106の後に `crates/gwt-cli/src/tui/app.rs` の Model に git_view_cache: GitViewCache フィールドを追加
- [x] **T108** [US1] T107の後に `crates/gwt-cli/src/tui/app.rs` の Model::new() で git_view と git_view_cache を初期化

### キーバインド

- [x] **T109** [US1] T108の後に `crates/gwt-cli/src/tui/app.rs` の handle_key_event で `v` キー処理を追加（BranchList → GitView 遷移）
- [x] **T110** [US1] T109の後に `crates/gwt-cli/src/tui/app.rs` の handle_key_event で GitView 画面の `v`/`Esc` キー処理を追加（NavigateBack）
- [x] **T111** [US1] T110の後に `crates/gwt-cli/src/tui/app.rs` の handle_key_event で GitView 画面の上下キー処理を追加
- [x] **T112** [US1] T111の後に `crates/gwt-cli/src/tui/app.rs` の handle_key_event で GitView 画面の Space キー処理を追加（展開トグル）

### レンダリング

- [x] **T113** [US1] T003の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に render_git_view() 関数の骨格を作成
- [x] **T114** [US1] T113の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に render_header() 関数を実装（ブランチ名、ahead/behind表示）
- [x] **T115** [US1] T114の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に render_files_section() 関数を実装（ファイル一覧、ステータスアイコン）
- [x] **T116** [US1] T115の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に render_file_diff() 関数を実装（展開時のdiff表示、50行制限）
- [x] **T117** [US1] T116の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に render_commits_section() 関数を実装（コミット一覧）
- [x] **T118** [US1] T117の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に render_commit_detail() 関数を実装（展開時のコミット詳細）
- [x] **T119** [US1] T118の後に `crates/gwt-cli/src/tui/app.rs` の view() で Screen::GitView の分岐を追加し render_git_view() を呼び出す

### Detailsパネル削除（US4）

- [x] **T120** [US4] `crates/gwt-cli/src/tui/screens/branch_list.rs` の render_branch_list() から Detailsパネルのレイアウト分割を削除
- [x] **T121** [US4] T120の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` の render_summary_panels() を修正し Sessionパネルのみ表示
- [x] **T122** [US4] T121の後に `crates/gwt-cli/src/tui/screens/branch_list.rs` から render_details_panel() 関数を削除またはコメントアウト

### データ取得

- [x] **T123** [US1] T003の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に build_git_view_data() 関数を実装（git status/diff/log 取得）
- [x] **T124** [US1] T123の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に parse_git_status() 関数を実装（porcelain形式パース）
- [x] **T125** [US1] T124の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に parse_git_diff() 関数を実装（diff内容パース、バイナリ検出）
- [x] **T126** [US1] T125の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に parse_git_log() 関数を実装（コミット履歴パース）

### キャッシュ

- [x] **T127** [US1] T107の後に `crates/gwt-cli/src/tui/app.rs` に git_view_cache_tx/rx チャネルを追加
- [x] **T128** [US1] T127の後に `crates/gwt-cli/src/tui/app.rs` に spawn_git_view_cache_update() 関数を実装（バックグラウンド取得）
- [x] **T129** [US1] T128の後に `crates/gwt-cli/src/tui/app.rs` に apply_git_view_cache_update() 関数を実装（Tick時に呼び出し）
- [x] **T130** [US1] T129の後に `crates/gwt-cli/src/tui/app.rs` の RefreshData 処理で git_view_cache.clear() を追加

**✅ MVP1チェックポイント**: US1+US4完了後、`v`キーでGitView画面が開き、ブランチ一覧は2ペイン構成で表示

## フェーズ3: ユーザーストーリー2 - PRリンクをブラウザで開く (優先度: P2)

**ストーリー**: ユーザーはGitView画面のヘッダーのPRリンクを選択し、`Enter`キーまたはマウスクリックでブラウザを起動できる。

**価値**: PRとの連携によるワークフロー効率化

### テスト（TDD）

- [ ] **T201** [P] [US2] `crates/gwt-cli/src/tui/screens/git_view.rs` に PRリンクフォーカス状態のユニットテストを作成

### PRリンク機能

- [x] **T202** [US2] T119の後に `crates/gwt-cli/src/tui/screens/git_view.rs` の render_header() にPRリンク表示とLinkRegion設定を追加
- [x] **T203** [US2] T202の後に `crates/gwt-cli/src/tui/app.rs` の handle_key_event で GitView 画面の Enter キー処理を追加（リンク開く）
- [x] **T204** [US2] T203の後に `crates/gwt-cli/src/tui/app.rs` に handle_gitview_mouse() 関数を追加（マウスクリックでリンク開く）
- [x] **T205** [US2] T204の後に `crates/gwt-cli/src/tui/app.rs` の handle_mouse_event で Screen::GitView の分岐を追加

**✅ MVP2チェックポイント**: US2完了後、PRリンクをクリックまたはEnterでブラウザで開ける

## フェーズ4: ユーザーストーリー3 - ワークツリーなしブランチの情報表示 (優先度: P2)

**ストーリー**: ユーザーはワークツリーを持たないブランチでも、git show/rev-parseで取得可能な情報を確認できる。

**価値**: 全ブランチに対してGitViewの価値を提供

### テスト（TDD）

- [ ] **T301** [P] [US3] `crates/gwt-cli/src/tui/screens/git_view.rs` に ワークツリーなしブランチの表示テストを作成

### ワークツリーなし対応

- [x] **T302** [US3] T126の後に `crates/gwt-cli/src/tui/screens/git_view.rs` に build_git_view_data_no_worktree() 関数を実装（git show/rev-parse のみ）
- [x] **T303** [US3] T302の後に `crates/gwt-cli/src/tui/screens/git_view.rs` の render_files_section() で ワークツリーなし時の「No worktree」表示を追加
- [x] **T304** [US3] T303の後に `crates/gwt-cli/src/tui/app.rs` の `v` キー処理で ワークツリーの有無に応じた GitViewState 初期化を分岐

**✅ MVP3チェックポイント**: US3完了後、ワークツリーなしブランチでも最低限の情報が表示される

## フェーズ5: 統合とポリッシュ

**目的**: すべてのストーリーを統合し、品質ゲートを通過

### 統合

- [ ] **T401** [統合] 全画面遷移パス（BranchList ↔ GitView）の動作確認
- [ ] **T402** [統合] エッジケース対応確認（50+ファイル、50+行diff、バイナリファイル）
- [ ] **T403** [統合] `cargo clippy --all-targets --all-features -- -D warnings` をローカルで完走させ、失敗時は修正
- [ ] **T404** [統合] `cargo fmt --check` をローカルで完走させ、失敗時は修正
- [ ] **T405** [統合] `cargo test` をローカルで完走させ、失敗時は修正

### ドキュメント

- [ ] **T406** [P] [ドキュメント] `README.md` にGitView画面の使い方を追加（`v`キーでの起動、ナビゲーション）

## タスク凡例

**優先度**:
- **P1**: 最も重要 - US1+US4に必要（GitView基本 + Detailsパネル削除）
- **P2**: 重要 - US2, US3に必要（PRリンク、ワークツリーなし対応）

**依存関係**:
- **[P]**: 並列実行可能
- **Tnnnの後に**: 明示的な依存関係あり

**ストーリータグ**:
- **[US1]**: GitView画面基本
- **[US2]**: PRリンクをブラウザで開く
- **[US3]**: ワークツリーなしブランチ対応
- **[US4]**: Detailsパネル削除
- **[共通]**: すべてのストーリーで共有
- **[統合]**: 複数ストーリーにまたがる
- **[ドキュメント]**: ドキュメント専用

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化

## 並列実行候補

以下のタスクは並列実行可能:
- T101, T102（テスト作成）
- T201, T301（各ストーリーのテスト作成）
- T406（ドキュメント）
