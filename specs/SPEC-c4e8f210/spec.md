# 機能仕様: Worktree Cleanup（GUI）

**仕様ID**: `SPEC-c4e8f210`
**作成日**: 2026-02-10
**ステータス**: 実装完了
**カテゴリ**: GUI / Worktree Management

**入力**: ユーザー説明: "TUI版にあったWorktreeの一括クリーンアップ機能と安全性表示をGUI版にも対応する"

## 背景

gwt の TUI 版では、Worktree にチェックを入れて一括クリーンアップする機能と、各 Worktree の安全性（uncommitted changes / unpushed commits の有無）を表示する機能が存在していた。

GUI 版（Tauri v2 + Svelte 5）への移行後、これらの機能が未実装のままとなっている。gwt-core には `remove()`, `remove_with_branch()`, `cleanup_branch()`, `detect_orphans()` などの API が存在するが、Tauri コマンドとして未公開であり、フロントエンドにもクリーンアップ UI が存在しない。

## ユーザーシナリオとテスト

### ユーザーストーリー 1 - 一括クリーンアップ（優先度: P0）

開発者として、不要な Worktree を一括で選択・削除したい。安全なもの（uncommitted changes も unpushed commits もない）を素早く選んで一括削除できること。

**受け入れシナリオ**:

1. **前提条件** Worktree が3つ以上存在する、**操作** Sidebar ヘッダーの Cleanup ボタンをクリック、**期待結果** Cleanup モーダルが開き、全 Worktree が安全性順（safe, warn, danger, disabled）で一覧表示される
2. **前提条件** 安全な Worktree が2つ存在する、**操作** "Select All Safe" ボタンをクリック、**期待結果** 安全な Worktree のみにチェックが入る
3. **前提条件** 安全な Worktree を2つ選択済み、**操作** "Cleanup" ボタンをクリック、**期待結果** モーダルが即座に閉じ、Sidebar の対象ブランチ行にスピナーが表示され、バックグラウンドで削除が進行する
4. **前提条件** 削除がすべて成功、**期待結果** Sidebar が自動リフレッシュされ、削除されたブランチが一覧から消える

### ユーザーストーリー 2 - 安全性表示（優先度: P0）

開発者として、各 Worktree が削除しても安全かどうかを一目で判断したい。

**受け入れシナリオ**:

1. **前提条件** uncommitted changes も unpushed commits もないブランチが存在する、**期待結果** Sidebar と Cleanup モーダルの両方で緑色のインジケーターが表示される
2. **前提条件** unpushed commits があるブランチが存在する、**期待結果** 黄色のインジケーターが表示される
3. **前提条件** uncommitted changes と unpushed commits の両方があるブランチが存在する、**期待結果** 赤色のインジケーターが表示される
4. **前提条件** protected branch（main/develop 等）の Worktree が存在する、**期待結果** グレーのインジケーターが表示され、チェックボックスが無効化される

### ユーザーストーリー 3 - 単体削除（優先度: P1）

開発者として、特定のブランチを右クリックから素早く削除したい。

**受け入れシナリオ**:

1. **前提条件** 安全なブランチが存在する、**操作** ブランチを右クリックし "Cleanup this branch" を選択、**期待結果** 確認ダイアログが表示される
2. **前提条件** 確認ダイアログで OK をクリック、**期待結果** Worktree とローカルブランチが削除され、Sidebar がリフレッシュされる

### ユーザーストーリー 4 - 危険なブランチの削除（優先度: P1）

開発者として、未コミットの変更がある Worktree も、リスクを承知の上で削除したい。

**受け入れシナリオ**:

1. **前提条件** unsafe なブランチを Cleanup モーダルで選択、**操作** "Cleanup" ボタンをクリック、**期待結果** 「N 個の unsafe な worktree が含まれています。削除しますか？」の確認ダイアログが表示される
2. **前提条件** 確認を承認、**期待結果** force オプション付きで削除が実行される

### ユーザーストーリー 5 - 削除失敗のハンドリング（優先度: P1）

開発者として、一括削除で一部が失敗した場合、どれが失敗したか知りたい。

**受け入れシナリオ**:

1. **前提条件** 5つの Worktree を一括削除、うち1つがロック中、**期待結果** 成功した4つは Sidebar から消え、失敗した1つについては Cleanup モーダルが再表示されエラー理由が表示される

### エッジケース

- 現在アクティブな（`is_current=true`）Worktree はモーダルに表示されるがチェック不可
- Protected branch（main/master/develop/release）はモーダルに表示されるがチェック不可
- エージェントが稼働中の Worktree はチェック不可
- 削除中のブランチは Sidebar でグレーアウトし、クリック無効
- リモートブランチは削除しない（ローカルブランチのみ削除）

## 要件

### 機能要件

- **FR-500**: システムは、全 Worktree の安全性を `has_changes` と `has_unpushed` の組み合わせで判定しなければ**ならない**
  - Safe: `has_changes=false` AND `has_unpushed=false`
  - Warning: `has_changes=true` XOR `has_unpushed=true`
  - Danger: `has_changes=true` AND `has_unpushed=true`
- **FR-501**: Sidebar のブランチ一覧に安全性インジケーター（CSS ドット: 緑=safe, 黄=warning, 赤=danger, グレー=protected/current）を表示しなければ**ならない**
- **FR-502**: Cleanup モーダルを Sidebar ヘッダーボタン、コンテキストメニュー、Git メニュー、Cmd+Shift+K の4箇所から起動できなければ**ならない**
- **FR-503**: Cleanup モーダルは全 Worktree を安全性順（safe, warn, danger, disabled）で表示しなければ**ならない**
- **FR-504**: Cleanup モーダルの各行に、安全性、ブランチ名、status、has_changes、has_unpushed、ahead/behind、is_gone、last_tool_usage を表示しなければ**ならない**
- **FR-505**: "Select All Safe" ボタンにより、安全な Worktree のみを一括選択できなければ**ならない**
- **FR-506**: protected branch、current worktree、エージェント稼働中の Worktree はチェック不可（disabled）としなければ**ならない**
- **FR-507**: unsafe な項目が選択された場合のみ、追加の確認ダイアログを表示しなければ**ならない**
- **FR-508**: 削除は Worktree ディレクトリとローカルブランチをセットで削除しなければ**ならない**。リモートブランチは削除しては**ならない**
- **FR-509**: 削除は非同期で実行し、モーダルは即座に閉じ、Sidebar のブランチ行に削除中のスピナーを表示しなければ**ならない**
- **FR-510**: 削除中のブランチは Sidebar でクリック無効にしなければ**ならない**
- **FR-511**: 一括削除時に一部が失敗した場合、失敗をスキップして残りを継続し、完了後にモーダルを再表示してエラーを表示しなければ**ならない**
- **FR-512**: コンテキストメニューに "Cleanup this branch"（単体即時削除、常に確認ダイアログ）と "Cleanup Worktrees..."（モーダル起動、プリセレクト付き）の2項目を提供しなければ**ならない**
- **FR-513**: 新規 "Git" メニューをメニューバーに追加し、"Cleanup Worktrees..." を配置しなければ**ならない**
- **FR-514**: Cmd+Shift+K で Cleanup モーダルを起動できなければ**ならない**

### 非機能要件

- **NFR-500**: 安全性判定（`has_changes`, `has_unpushed`）の取得は Worktree 一覧取得時に同時に行い、追加の I/O を最小限に抑えなければ**ならない**
- **NFR-501**: 自動クリーンアップ（`auto_cleanup_orphans`）は無効のまま維持し、手動クリーンアップのみを提供する

## インターフェース（フロント/バック間）

### Tauri Commands（新規）

- `list_worktrees(project_path: String) -> Vec<WorktreeInfo>`
  - `WorktreeInfo`: `path`, `branch`, `commit`, `status`, `is_main`, `has_changes`, `has_unpushed`, `is_current`, `is_protected`, `is_agent_running`, `ahead`, `behind`, `is_gone`, `last_tool_usage`
- `cleanup_worktrees(project_path: String, branches: Vec<String>, force: bool) -> Vec<CleanupResult>`
  - `CleanupResult`: `branch`, `success`, `error`
- `cleanup_single_worktree(project_path: String, branch: String, force: bool) -> Result<(), String>`

### Tauri Events

- `worktrees-changed`（既存）: 削除完了時に emit
- `cleanup-progress`（新規）: 各 Worktree の削除進捗を emit
  - payload: `{ branch: String, status: "deleting" | "deleted" | "failed", error?: String }`
- `cleanup-completed`（新規）: 一括削除完了時に emit
  - payload: `{ results: Vec<CleanupResult> }`

### UI

- Sidebar ヘッダー: Cleanup ボタン
- メニュー: Git > Cleanup Worktrees...（Cmd+Shift+K）
- コンテキストメニュー: Cleanup this branch / Cleanup Worktrees...
- Cleanup モーダルダイアログ

## 範囲外

- リモートブランチの削除
- 自動クリーンアップ（起動時の orphan 自動削除）
- GitHub PR マージ状態の確認による安全性判定
- Worktree の新規作成 UI（別機能）
