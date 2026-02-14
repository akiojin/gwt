# GitView in Worktree Summary Panel

**SPEC ID**: `SPEC-735cbc5d`
**Created**: 2026-02-10
**Updated**: 2026-02-14
**Status**: Draft
**カテゴリ**: Feature (GUI)

**Implementation Phase**: Phase 1 (Full Implementation)

## 依存仕様と責務境界（2026-02-14 追記）

- Session Summary のタブ構成・配置責務は `SPEC-d6949f99` を正本とする
- 本SPECは主に Git 表示領域の内部要件（Changes/Commits/Stash、diff、stash、base branch）を定義する
- レイアウトや初期表示状態が競合する場合は `SPEC-d6949f99` を優先する

## 概要

Session Summary 内に Git 表示領域を追加し、
ブランチの変更状況（Changes / Commits / Stash）を視覚的に確認できるようにする。
TUI 版 GitView の GUI 移植として、エージェントがブランチで何を変えたかの
全体像を把握するための機能。

**最優先体験**: 変更ファイル一覧と diff の確認。エージェントが何を変えたかが
一目でわかること。

## UI レイアウト概要

> 注記: Session Summary の最終レイアウト（`Summary / PR / AI Summary / Git`）は
> `SPEC-d6949f99` に従う。以下は Git 表示領域の内部構造イメージ。

```text
Sidebar（Branch Mode）
├── Worktree一覧（既存）
└── Worktree Summary パネル
    └── Git 表示領域
        ├── ヘッダー: [v] Git  5 files, 3 commits, 1 stash  [Refresh]
        ├── 基準ブランチドロップダウン: [main v]
        └── タブ: [Changes] [Commits] [Stash]
            ├── Changes タブ（デフォルト）
            │   ├── フィルター: [Committed] [Uncommitted]
            │   └── ディレクトリツリー（全展開）
            │       ├── src/
            │       │   ├── main.rs  ████░  +45 -12
            │       │   └── lib.rs   █████  +8 -0
            │       └── Cargo.toml   ██░░░  +3 -5
            ├── Commits タブ
            │   ├── a1b2c3d feat: add login  2 hours ago
            │   ├── d4e5f6g fix: typo        3 hours ago
            │   └── [Show more]
            └── Stash タブ（0件なら非表示）
                └── stash@{0}: WIP on feature (3 files)
```

## User Scenarios and Tests

### User Story 1 - ブランチ変更ファイルの確認 (Priority: P1)

開発者がブランチを選択し、Session Summary の Git セクションを展開すると、
base ブランチとの差分ファイル一覧がディレクトリツリー形式で表示される。
各ファイルには GitHub 風の 5 ブロック統計バーと数値（+N -N）が付き、
クリックで diff 内容を展開できる。

**Reason for priority**: ブランチの変更全体像把握が最も基本的なユースケース。
これが本機能の最優先体験。

**Independent test**: ブランチ選択 -> Git セクション展開 -> Changes タブに
ファイルツリーが表示される。

**Acceptance scenarios**:

1. **Given** ブランチが選択されている, **When** Git セクションを展開,
   **Then** Changes タブがデフォルトで表示され、base ブランチとの差分
   ファイルがディレクトリツリー（全展開）で表示される。
2. **Given** Changes タブが表示されている, **When** 各ファイルを確認,
   **Then** GitHub 風 5 ブロック統計バー + "+N -N" の数値が表示される。
3. **Given** Changes タブが表示されている, **When** ファイルをクリック,
   **Then** そのファイルの unified diff 内容が展開表示される
   （monospace、緑: 追加行、赤: 削除行）。
4. **Given** Changes タブが表示されている, **When** バイナリファイルがある,
   **Then** "Binary file changed" と表示され、diff 展開は不可。
5. **Given** Changes タブが表示されている, **When** diff が 1000 行を超える,
   **Then** 1000 行で切り詰められ "Too large to display" と表示される。

---

### User Story 2 - Working tree 変更の確認 (Priority: P1)

開発者が Changes タブで "Uncommitted" フィルターに切り替えると、
未コミットの working tree 変更が Staged / Unstaged に分離して表示される。

**Reason for priority**: 現在の作業状態の把握に不可欠。

**Independent test**: Changes タブ -> "Uncommitted" フィルター切り替え ->
Staged / Unstaged のサブセクションに分かれて未コミット変更が表示される。

**Acceptance scenarios**:

1. **Given** Changes タブの "Committed" ビュー, **When** "Uncommitted" に
   トグル切り替え, **Then** working tree の未コミット変更が
   "Staged" と "Unstaged" のサブセクションに分離して表示される。
2. **Given** "Uncommitted" ビュー, **When** 変更がない,
   **Then** "No uncommitted changes" と表示される。
3. **Given** "Uncommitted" ビュー, **When** staged ファイルのみ存在,
   **Then** "Staged" セクションのみ表示され、"Unstaged" は空状態。

---

### User Story 3 - コミット履歴の確認 (Priority: P1)

開発者が Commits タブに切り替えると、base ブランチとの差分コミットが
最新 20 件表示される。日付は相対時間で表示され、hover で絶対日時が見える。

**Reason for priority**: エージェントの作業履歴の確認に必要。

**Independent test**: Commits タブ選択 -> 最新 20 件のコミットが SHA +
message + 相対日時で表示される。

**Acceptance scenarios**:

1. **Given** Git セクション展開済み, **When** Commits タブを選択,
   **Then** base ブランチとの差分コミットが最新 20 件、
   短縮 SHA + メッセージ + 相対日時で表示される。
2. **Given** コミットの日時にマウスを hover, **When** ツールチップ表示,
   **Then** 絶対日時（例: "2026-02-10 14:30"）が表示される。
3. **Given** コミットが 20 件以上ある, **When** "Show more" をクリック,
   **Then** さらに 20 件が追加読み込みされる。
4. **Given** ブランチにコミットが 0 件, **When** Commits タブを選択,
   **Then** "No commits" と空状態が表示される。

---

### User Story 4 - Stash 一覧の確認 (Priority: P2)

開発者が Stash タブに切り替えると、stash エントリがメッセージ +
ファイル数で一覧表示される。

**Reason for priority**: 補助的な情報。stash がない場合はタブ自体が非表示。

**Independent test**: stash が存在する場合 -> Stash タブが表示される ->
stash メッセージとファイル数が見える。

**Acceptance scenarios**:

1. **Given** stash が 1 件以上ある, **When** Git セクション展開,
   **Then** Stash タブが表示され、stash 一覧が見える。
2. **Given** stash が 0 件, **When** Git セクション展開,
   **Then** Stash タブは非表示。
3. **Given** Stash タブ表示中, **When** stash エントリを確認,
   **Then** `stash@{N}: メッセージ (X files)` 形式で表示される。

---

### User Story 5 - Git セクションヘッダーのサマリー表示 (Priority: P1)

Git セクションが折りたたまれた状態でも、ヘッダーにサマリー情報
（ファイル数、コミット数、stash 数）が表示される。

**Reason for priority**: 展開せずとも概要が把握できるUXの要。

**Independent test**: Git セクションが折りたたまれている ->
ヘッダーに "5 files, 3 commits, 1 stash" 等のサマリーが見える。

**Acceptance scenarios**:

1. **Given** Worktree Summary パネル表示中, **When** Git セクションが折りたたまれている,
   **Then** ヘッダーに "X files, Y commits, Z stash" のサマリーが表示される。
2. **Given** Git セクションヘッダー, **When** リフレッシュアイコンをクリック,
   **Then** Git 情報が再取得され、スピナー + "Loading git info..." が表示される。

---

### User Story 6 - 基準ブランチの選択 (Priority: P2)

開発者が基準ブランチをドロップダウンで切り替えると、diff の比較先が変わる。

**Reason for priority**: upstream 以外との比較が必要な場合の対応。

**Independent test**: ドロップダウンで "develop" を選択 ->
diff が develop との比較に切り替わる。

**Acceptance scenarios**:

1. **Given** Changes/Commits タブ表示中, **When** 基準ブランチのドロップダウン
   を開く, **Then** main / develop / upstream（存在するもののみ）が選択肢として
   表示される。
2. **Given** ドロップダウンで基準ブランチを変更, **When** 選択確定,
   **Then** Changes と Commits の内容が新しい基準で再取得・再描画される。
3. **Given** upstream が未設定, **When** Git セクション表示,
   **Then** main にフォールバックされる。

---

### Edge Cases

- **upstream 未設定**: "No upstream" 表示 + main にフォールバック
- **コミット 0 件**: 空状態メッセージ表示
- **大規模 diff（1000 行超）**: ファイル単位で切り詰め +
  "Too large to display" 表示
- **バイナリファイル**: "Binary file changed" 表示、diff 展開不可
- **stash 0 件**: Stash タブ自体を非表示
- **Git リポジトリでないプロジェクト**: Git セクション自体を非表示

## Requirements

### Functional Requirements

- **FR-001**: Session Summary 内に Git 表示領域を表示する。配置・タブ構成・初期表示状態は `SPEC-d6949f99` に従う。
- **FR-002**: Git セクションヘッダーにサマリー情報（ファイル数、コミット数、
  stash 数）を "X files, Y commits, Z stash" 形式で表示する。
- **FR-003**: Git セクションヘッダーにリフレッシュボタンを配置する。
  クリックで全 Git 情報を再取得する。
- **FR-004**: Git セクション内に Changes / Commits / Stash のタブ切り替えを
  提供する。デフォルトタブは Changes。Stash は 0 件時に非表示。
- **FR-005**: Changes タブで base ブランチとの差分ファイルをディレクトリツリー
  形式で表示する。ツリーはデフォルトで全展開。
- **FR-006**: 各ファイルに GitHub 風 5 ブロック統計バー + "+N -N" の数値を
  表示する。
- **FR-007**: ファイルクリックで unified diff 内容を展開表示する
  （monospace、緑: 追加行、赤: 削除行）。
- **FR-008**: Changes タブに "Committed" / "Uncommitted" のフィルタートグルを
  提供する。"Committed" がデフォルト。
- **FR-009**: "Uncommitted" フィルターでは "Staged" と "Unstaged" の
  サブセクションに分離して表示する。
- **FR-010**: Commits タブで base ブランチとの差分コミットを最新 20 件表示する。
  各コミットは短縮 SHA + メッセージ + 相対日時。hover で絶対日時をツールチップ
  表示する。
- **FR-011**: Commits タブに "Show more" ボタンで 20 件ずつ追加読み込みを
  提供する。
- **FR-012**: Stash タブで stash 一覧をメッセージ + ファイル数で表示する。
  stash が 0 件の場合はタブ自体を非表示にする。
- **FR-013**: 基準ブランチのドロップダウン選択を提供する。
  選択肢は main / develop / upstream（存在するもののみ）。
  デフォルトは自動検出（upstream > main）。
- **FR-014**: バイナリファイルは "Binary file changed" と表示し、
  diff 展開を不可にする。
- **FR-015**: 1 ファイルあたり 1000 行を超える diff は切り詰めて
  "Too large to display" と表示する。
- **FR-016**: Git 情報は Worktree Summary パネル表示時に取得し、リフレッシュボタンで手動更新する。
  取得中はスピナー + "Loading git info..." を表示する。
- **FR-017**: Git リポジトリでないプロジェクトでは Git セクションを非表示にする。

### Non-Functional Requirements

- **NFR-001**: Git 情報の取得は 3 秒以内に完了すること
  （一般的な規模のリポジトリ）。
- **NFR-002**: 大量ファイル（100+）の場合もツリー描画が滑らかであること。
- **NFR-003**: UI アイコンは ASCII に統一し、全角/絵文字は避けること
  （CLAUDE.md 準拠）。
- **NFR-004**: GUI の表示言語は英語のみ（CLAUDE.md 準拠）。

### Main Entities

**Backend (Rust)**:

- **GitChangeSummary**: ブランチ差分のサマリー情報
  （file_count, commit_count, stash_count, base_branch）
- **FileChange**: 変更ファイル情報
  （path, kind, additions, deletions, is_binary）
- **FileChangeKind**: enum（Added / Modified / Deleted / Renamed）
- **FileDiff**: ファイル単位の diff テキスト（content, truncated）
- **CommitEntry**: コミット情報（sha, message, timestamp, author）
- **StashEntry**: stash 情報（index, message, file_count）
- **WorkingTreeEntry**: Working tree エントリ
  （path, status, is_staged）

**Frontend (TypeScript)**:

- **GitChangeSummary**: バックエンドと同型の TypeScript interface
- **FileChange / FileChangeKind / FileDiff / CommitEntry / StashEntry /
  WorkingTreeEntry**: 同上

**New Tauri Commands** (読み取り専用):

- `get_git_change_summary(project_path, branch, base_branch)`:
  サマリー取得
- `get_branch_diff_files(project_path, branch, base_branch)`:
  ファイル一覧取得
- `get_file_diff(project_path, branch, base_branch, file_path)`:
  個別ファイルの diff 取得（1000 行制限付き）
- `get_branch_commits(project_path, branch, base_branch, offset, limit)`:
  コミット一覧取得（ページネーション対応）
- `get_working_tree_status(project_path, branch)`:
  Working tree 状態取得（staged / unstaged 分離）
- `get_stash_list(project_path, branch)`:
  stash 一覧取得
- `get_base_branch_candidates(project_path)`:
  基準ブランチ候補一覧取得

## Success Criteria

### Measurable Outcomes

- **SC-001**: Session Summary で Git 表示領域が `SPEC-d6949f99` のレイアウト要件に従って表示される。
- **SC-002**: Changes タブでファイルツリーと diff 展開が表示される。
- **SC-003**: Committed/Uncommitted のフィルター切り替えが動作する。
- **SC-004**: Uncommitted ビューで Staged/Unstaged が分離表示される。
- **SC-005**: Commits タブで 20 件表示 + Show more が動作する。
- **SC-006**: コミット日時の相対表示 + hover 絶対表示が動作する。
- **SC-007**: Stash タブで stash 一覧が表示される（0 件時は非表示）。
- **SC-008**: 基準ブランチのドロップダウン切り替えが動作する。
- **SC-009**: リフレッシュボタンで再取得が動作する。
- **SC-010**: ローディング中にスピナーが表示される。
- **SC-011**: エッジケース（upstream 未設定、0 コミット、大規模 diff、
  バイナリファイル）が正しく処理される。

## Out of Scope

- Git 操作（stage/unstage/commit/push/stash pop 等）の UI。
  将来的にフル操作対応を予定するが、本 SPEC のスコープ外。
- シンタックスハイライト付き diff 表示。プレーンテキスト + 色分けのみ。
- リアルタイム自動更新（file watcher）。タブ表示時 + 手動リフレッシュのみ。
- Side-by-side diff 表示。unified diff のみ。

## Dependencies

- 既存の Session Summary タブ（MainArea.svelte）
- Session Summary レイアウト正本: `SPEC-d6949f99`
- gwt-core の Git 操作モジュール（git/branch.rs, git/commit.rs）
- Tauri IPC（invoke）

## Design Decisions

| 項目 | 決定 | 理由 |
| ---- | ---- | ---- |
| Git セクション配置 | Session Summary の Git タブ（正本は SPEC-d6949f99） | レイアウト責務の一元化 |
| 内部構造 | タブ式切り替え | 情報種別が明確に異なるため |
| ファイルリスト | ディレクトリツリー（全展開） | 一目で全体把握可能 |
| diff 表示 | ファイルリスト + 展開式 | 必要なファイルだけ確認できる |
| 変更種別表現 | 5 ブロックバー + 数値 | GitHub PR 風で情報量と直感性の両立 |
| Committed/Uncommitted | フィルター/トグル | 画面遷移なしで切り替え |
| Uncommitted 内部 | Staged / Unstaged 分離 | 将来の stage 操作に備えた構造 |
| コミット表示 | 最新 20 件 + Show more | パフォーマンスと利便性の両立 |
| コミット日時 | 相対時間 + hover 絶対日時 | 可読性とスペース効率の両立 |
| Stash 0 件時 | タブ非表示 | 不要な UI ノイズを排除 |
| 基準ブランチ | 自動検出 + ドロップダウン | 柔軟性と利便性の両立 |
| ドロップダウン候補 | main / develop / upstream | 主要ブランチに絞りシンプルに |
| 更新タイミング | タブ表示時 + 手動 | パフォーマンスと鮮度のバランス |
| ローディング | スピナー + テキスト | ユーザーに取得中であることを伝える |
| diff 閾値 | 1000 行 / ファイル | 一般的な変更は十分表示できるサイズ |
| バイナリファイル | "Binary file changed" 表示 | diff 不可能なファイルの明示 |
| API 設計 | 表示専用でシンプル | YAGNI 原則。操作は将来追加 |

## References

- TUI 版 GitView（以前のバージョンに存在、Tauri GUI 移行で未移植）
- GitHub PR Files Changed UI（+/- 統計バー、ファイルツリーの参考）
