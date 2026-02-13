# タスク: GitView in Session Summary

**入力**: `/specs/SPEC-735cbc5d/` からの設計ドキュメント
**前提条件**: spec.md（必須）

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2）

## Lint 最小要件

- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo fmt --check`
- `cargo test`
- `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`

## フェーズ 1: セットアップ（TDD テスト作成）

**目的**: TDD に従い、バックエンド側のテストを先に作成

### テストタスク

- [ ] **T001** [P] [共通] `crates/gwt-core/src/git/diff.rs` に新規ファイルを
  作成し、テストモジュールを追加
- [ ] **T002** [P] [共通] `crates/gwt-core/src/git/stash.rs` に新規ファイルを
  作成し、テストモジュールを追加
- [ ] **T003** [共通] `crates/gwt-core/src/git/mod.rs` に `diff` と `stash`
  モジュールを追加
- [ ] **T004** [共通] T001 の後に以下の FR に対応するテストケースを作成
  （全テストは失敗状態）:
  - FR-005: ブランチ差分ファイル一覧取得
  - FR-006: ファイル変更統計（additions/deletions）
  - FR-007: ファイル diff テキスト取得
  - FR-009: Staged/Unstaged 分離
  - FR-014: バイナリファイル判定
  - FR-015: 1000 行超 diff の切り詰め
- [ ] **T005** [共通] T002 の後に以下の FR に対応するテストケースを作成
  （全テストは失敗状態）:
  - FR-012: stash 一覧取得
  - stash 0 件時の空リスト返却

## フェーズ 2: バックエンド - データ型定義

**目的**: Rust 側のデータ型を定義

### データ型（全タスク並列可能）

- [ ] **T101** [P] [US1] `crates/gwt-core/src/git/diff.rs` に
  `FileChangeKind` enum を定義（Added / Modified / Deleted / Renamed）
- [ ] **T102** [P] [US1] 同ファイルに `FileChange` struct を定義
  （path: String, kind: FileChangeKind, additions: usize, deletions: usize,
  is_binary: bool）
- [ ] **T103** [P] [US1] 同ファイルに `FileDiff` struct を定義
  （content: String, truncated: bool）
- [ ] **T104** [P] [US3] 同ファイルに `CommitEntry` struct を定義
  （sha: String, message: String, timestamp: i64, author: String）
- [ ] **T105** [P] [US2] 同ファイルに `WorkingTreeEntry` struct を定義
  （path: String, status: FileChangeKind, is_staged: bool）
- [ ] **T106** [P] [US4] `crates/gwt-core/src/git/stash.rs` に
  `StashEntry` struct を定義
  （index: usize, message: String, file_count: usize）
- [ ] **T107** [US5] `crates/gwt-core/src/git/diff.rs` に
  `GitChangeSummary` struct を定義
  （file_count: usize, commit_count: usize, stash_count: usize,
  base_branch: String）

## フェーズ 3: バックエンド - Git 操作実装

**目的**: git コマンドを呼び出して情報を取得するロジックを実装

### 基準ブランチ検出 (US6) — 他タスクの前提

- [ ] **T201** [US6] `crates/gwt-core/src/git/diff.rs` に
  `detect_base_branch()` 関数を実装
  （upstream 自動検出 + main フォールバック）
- [ ] **T202** [US6] 同ファイルに `list_base_branch_candidates()` 関数を実装
  （main / develop / upstream から存在するもののみ返却）

### Changes 関連 (US1, US2)

- [ ] **T203** [US1] T101, T102, T201 の後に `get_branch_diff_files()` 関数を
  実装（`git diff --numstat --name-status <base>..HEAD`）
- [ ] **T204** [US1] T103, T201 の後に `get_file_diff()` 関数を実装
  （`git diff <base>..HEAD -- <file>`）。1000 行超の切り詰めロジック含む
- [ ] **T205** [US2] T105 の後に `get_working_tree_status()` 関数を実装
  （`git status --porcelain`、staged / unstaged 分離）

### Commits 関連 (US3)

- [ ] **T206** [US3] T104, T201 の後に `get_branch_commits()` 関数を実装
  （`git log <base>..HEAD --format=...` + offset/limit パラメータ）

### Stash 関連 (US4)

- [ ] **T207** [US4] T106 の後に `get_stash_list()` 関数を実装
  （`git stash list --format=...`）

### サマリー関連 (US5)

- [ ] **T208** [US5] T203, T206, T207 の後に `get_git_change_summary()` 関数を
  実装（各データのカウントを集約）

### テスト検証

- [ ] **T209** [共通] T201〜T208 の後に全テストが成功することを確認

## フェーズ 4: Tauri コマンド追加

**目的**: フロントエンドから呼び出せる Tauri IPC コマンドを追加

### コマンド定義

- [ ] **T301** [共通] `crates/gwt-tauri/src/commands/git_view.rs` に新規ファイルを
  作成
- [ ] **T302** [US5] T301, T208 の後に `get_git_change_summary` コマンドを実装
  （params: project_path, branch, base_branch）
- [ ] **T303** [US1] T301, T203 の後に `get_branch_diff_files` コマンドを実装
  （params: project_path, branch, base_branch）
- [ ] **T304** [US1] T301, T204 の後に `get_file_diff` コマンドを実装
  （params: project_path, branch, base_branch, file_path）
- [ ] **T305** [US3] T301, T206 の後に `get_branch_commits` コマンドを実装
  （params: project_path, branch, base_branch, offset, limit）
- [ ] **T306** [US2] T301, T205 の後に `get_working_tree_status` コマンドを実装
  （params: project_path）
- [ ] **T307** [US4] T301, T207 の後に `get_stash_list` コマンドを実装
  （params: project_path）
- [ ] **T308** [US6] T301, T202 の後に `get_base_branch_candidates` コマンドを
  実装（params: project_path）
- [ ] **T309** [共通] `crates/gwt-tauri/src/commands/mod.rs` に `git_view`
  モジュールを登録
- [ ] **T310** [共通] T309 の後に `crates/gwt-tauri/src/lib.rs`
  （または main.rs）に新コマンドを invoke ハンドラに追加

## フェーズ 5: フロントエンド - TypeScript 型定義

**目的**: バックエンドと一致する TypeScript 型を定義

### 型定義

- [ ] **T401** [P] [共通] `gwt-gui/src/lib/types.ts` に以下の型を追加:
  - `FileChangeKind`: "Added" | "Modified" | "Deleted" | "Renamed"
  - `FileChange`: { path, kind, additions, deletions, is_binary }
  - `FileDiff`: { content, truncated }
  - `CommitEntry`: { sha, message, timestamp, author }
  - `StashEntry`: { index, message, file_count }
  - `WorkingTreeEntry`: { path, status, is_staged }
  - `GitChangeSummary`: { file_count, commit_count, stash_count, base_branch }

## フェーズ 6: フロントエンド - Git セクション UI

**目的**: Svelte コンポーネントで GitView UI を構築

### コンポーネント作成

- [ ] **T501** [US5] T401 の後に `gwt-gui/src/lib/components/GitSection.svelte`
  を作成:
  - 折りたたみヘッダー（デフォルト折りたたみ）
  - サマリー表示（"X files, Y commits, Z stash"）
  - リフレッシュボタン（ヘッダー内、ASCII アイコン）
  - ローディング状態（スピナー + "Loading git info..."）
  - タブ切り替え UI（Changes / Commits / Stash）
  - 基準ブランチドロップダウン
- [ ] **T502** [US1] T501 の後に `gwt-gui/src/lib/components/GitChangesTab.svelte`
  を作成:
  - ディレクトリツリー（デフォルト全展開）
  - 各ファイルに GitHub 風 5 ブロック統計バー + "+N -N" 数値
  - ファイルクリックで diff 展開（monospace、緑/赤色分け）
  - バイナリファイル: "Binary file changed"、展開不可
  - 1000 行超 diff: "Too large to display"
- [ ] **T503** [US2] T502 の後に GitChangesTab にフィルタートグルを追加:
  - "Committed" / "Uncommitted" トグル（Committed がデフォルト）
  - Uncommitted 時: "Staged" / "Unstaged" サブセクション分離
  - 変更なし時: "No uncommitted changes" 空状態
- [ ] **T504** [US3] T501 の後に `gwt-gui/src/lib/components/GitCommitsTab.svelte`
  を作成:
  - コミットリスト（短縮 SHA + メッセージ + 相対日時）
  - 相対日時に hover で絶対日時ツールチップ
  - "Show more" ボタン（20 件ずつ追加読み込み）
  - 0 件時: "No commits" 空状態
- [ ] **T505** [US4] T501 の後に `gwt-gui/src/lib/components/GitStashTab.svelte`
  を作成:
  - stash 一覧（`stash@{N}: メッセージ (X files)`）
  - 0 件時: タブ自体を非表示（GitSection 側で制御）

### Worktree Summary 統合

- [ ] **T506** [共通] T501〜T505 の後に `MainArea.svelte` に GitSection を統合:
  - Quick Start セクションの下に配置
  - ブランチ選択時のデータ取得トリガー

### データ取得ロジック

- [ ] **T507** [共通] T506 の後にタブ表示時の自動取得ロジックを実装:
  - Worktree Summary パネル表示時に `get_git_change_summary` を呼び出し
  - 各タブ切り替え時に対応データを遅延取得
- [ ] **T508** [共通] T507 の後にリフレッシュボタンの再取得ロジックを実装:
  - 全データをクリアして再取得
  - ローディング状態の表示

### 基準ブランチ切り替え

- [ ] **T509** [US6] T506 の後にドロップダウン変更時の再取得ロジックを実装:
  - 基準ブランチ変更で Changes + Commits の内容を再取得・再描画

## フェーズ 7: エッジケース対応

**目的**: 仕様に定義されたエッジケースの処理

- [ ] **T601** [US6] upstream 未設定時の "No upstream" 表示 + main フォールバック
- [ ] **T602** [US3] コミット 0 件時の空状態表示
- [ ] **T603** [US1] 大規模 diff（1000 行超）の切り詰め +
  "Too large to display" 表示
- [ ] **T604** [US1] バイナリファイルの "Binary file changed" 表示
- [ ] **T605** [共通] Git リポジトリでないプロジェクトでの
  Git セクション非表示

## フェーズ 8: 統合とポリッシュ

**目的**: 全ストーリーを統合し、品質を確認

### 検証

- [ ] **T701** [統合] `cargo test` で全テストが成功することを確認
- [ ] **T702** [統合] `cargo clippy --all-targets --all-features -- -D warnings`
  が成功することを確認
- [ ] **T703** [統合] `cargo fmt --check` が成功することを確認
- [ ] **T704** [統合] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json`
  が成功することを確認

### コミット

- [ ] **T705** [デプロイ] すべての変更をコミット＆プッシュ

## タスク凡例

**優先度**:

- **P1**: 最も重要 - Changes + Commits + ヘッダーサマリー (US1, US2, US3, US5)
- **P2**: 重要 - Stash + 基準ブランチ選択 (US4, US6)

**ストーリータグ**:

- **[US1]**: ブランチ変更ファイルの確認
- **[US2]**: Working tree 変更の確認
- **[US3]**: コミット履歴の確認
- **[US4]**: Stash 一覧の確認
- **[US5]**: Git セクションヘッダーのサマリー表示
- **[US6]**: 基準ブランチの選択
- **[共通]**: 全ストーリーで共有
- **[統合]**: 複数ストーリーにまたがる
- **[デプロイ]**: デプロイメント専用

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
