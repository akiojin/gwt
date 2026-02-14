# 機能仕様: GitHub Issue連携によるブランチ作成（GUI版）

**仕様ID**: `SPEC-c6ba640a`
**作成日**: 2026-02-13
**ステータス**: ドラフト
**カテゴリ**: GUI
**依存仕様**:

- SPEC-e4798383（TUI版GitHub Issue連携 - archived）

**入力**: ユーザー説明: "TUI版に存在したGitHub Issueからのブランチ作成機能をGUI版のAgent Launch Formに移植する"

## 背景

- TUI版gwtではWizardの一ステップとしてGitHub Issue選択→ブランチ自動生成→`gh issue develop`によるGitHubリンクが実装されていた
- GUI版（Tauri + Svelte 5）のAgent Launch FormにはNew Branch作成機能があるが、GitHub Issue連携が完全に欠落している
- gwt-coreの`git/issue.rs`にはIssue取得・フィルタ・ブランチリンクのコアロジックが既に実装済み（660行）
- Tauriコマンドとしての公開、およびSvelteフロントエンドのUI実装が必要

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - GitHub IssueからNew Branchを作成できる (優先度: P0)

開発者がAgent Launch FormのNew Branchモードで「From Issue」タブを選択し、GitHub Issueを選んでブランチを自動生成できる。

**独立したテスト**: Agent Launch Formで「From Issue」タブ→Issue選択→ブランチ名自動生成→Launch実行で完全にテスト可能。

**受け入れシナリオ**:

1. **前提条件** Agent Launch Formが開かれ「New Branch」モード、**操作** 「From Issue」タブをクリック、**期待結果** GitHub Issue一覧がローディング表示後に表示される
2. **前提条件** Issue一覧が表示されている、**操作** Issue #42「Fix login bug」を選択、**期待結果** Branch Prefixに応じて`feature/issue-42`がブランチ名として自動設定される
3. **前提条件** ブランチ名が自動設定されている、**操作** Launchボタンを押す、**期待結果** Worktree作成→`gh issue develop`→エージェント起動が順に実行される

---

### ユーザーストーリー 2 - 従来のManual入力と共存できる (優先度: P0)

開発者がIssue連携を使わず「Manual」タブで従来通り手動ブランチ名を入力できる。

**独立したテスト**: 「Manual」タブで手動入力→Launch実行で完全にテスト可能。

**受け入れシナリオ**:

1. **前提条件** Agent Launch FormのNew Branchモード、**操作** 「Manual」タブが選択された状態（デフォルト）、**期待結果** 従来通りのPrefix選択 + Suffix入力 + AI Suggest UIが表示される
2. **前提条件** 「Manual」タブ、**操作** 手動で「my-feature」と入力しLaunch、**期待結果** `feature/my-feature`として正常に処理される（Issue連携なし）

---

### ユーザーストーリー 3 - Issue一覧をテキスト検索で絞り込める (優先度: P1)

開発者がIssue一覧から目的のIssueをテキスト検索で素早く見つけられる。

**独立したテスト**: Issue一覧のSearch入力欄にキーワード入力→タイトルマッチするIssueのみ表示で完全にテスト可能。

**受け入れシナリオ**:

1. **前提条件** Issue一覧（多数）が表示されている、**操作** 検索欄に「login」と入力、**期待結果** タイトルに「login」を含むIssueのみがフィルタ表示される
2. **前提条件** 検索結果が絞り込まれている、**操作** 検索文字を全て削除、**期待結果** 全Issueが再表示される
3. **前提条件** 検索結果が0件、**操作** マッチしないキーワードを入力、**期待結果** 「No matching issues」メッセージが表示される

---

### ユーザーストーリー 4 - 無限スクロールでIssueを追加取得できる (優先度: P1)

大量のIssueがあるリポジトリでも、スクロールで次のページを自動取得して表示できる。

**独立したテスト**: 50件超のIssueがあるリポジトリで、リスト末尾までスクロール→追加Issueが読み込まれることで完全にテスト可能。

**受け入れシナリオ**:

1. **前提条件** 100件のIssueがあるリポジトリ、**操作** 「From Issue」タブを開く、**期待結果** 最初の50件が表示される
2. **前提条件** 50件が表示されている、**操作** リスト末尾付近までスクロール、**期待結果** ローディングインジケータが表示され、次の50件が追加読み込みされる
3. **前提条件** 全Issueが読み込み済み、**操作** リスト末尾までスクロール、**期待結果** 追加取得は行われない

---

### ユーザーストーリー 5 - 既にブランチがあるIssueは選択できない (優先度: P1)

同一Issueに対する重複ブランチ作成を防止するため、既存ブランチがあるIssueは選択不可にする。

**独立したテスト**: 既存ブランチがあるIssueがリスト上で無効化（disabled）表示されることで完全にテスト可能。

**受け入れシナリオ**:

1. **前提条件** `feature/issue-42`ブランチが既に存在する、**操作** 「From Issue」タブでIssue一覧を表示、**期待結果** Issue #42はグレーアウトされ選択不可。既存ブランチ名が表示される
2. **前提条件** ブランチのないIssue #43が存在する、**操作** Issue #43を選択、**期待結果** 正常に選択でき、ブランチ名が自動生成される

---

### ユーザーストーリー 6 - gh CLIが利用不可の場合「From Issue」タブが無効化される (優先度: P2)

gh CLIが未インストールまたは未認証の場合、「From Issue」タブをグレーアウトしてツールチップで案内する。

**独立したテスト**: gh CLI未検出環境で「From Issue」タブが無効化＋ツールチップ表示で完全にテスト可能。

**受け入れシナリオ**:

1. **前提条件** gh CLIが未インストール、**操作** Agent Launch Formを開く、**期待結果** 「From Issue」タブがdisabled状態で、ホバー時に「GitHub CLI (gh) is required」ツールチップが表示される
2. **前提条件** gh CLIがインストール済みだが未認証、**操作** Agent Launch Formを開く、**期待結果** 「From Issue」タブがdisabled状態で、ホバー時に「GitHub CLI authentication required」ツールチップが表示される

---

### ユーザーストーリー 7 - 失敗時の完全ロールバック (優先度: P1)

ブランチ作成→Worktree作成→gh issue develop→エージェント起動の途中で失敗した場合、作成済みリソースを全て巻き戻す。

**独立したテスト**: 各ステップで意図的に失敗を発生させ、ロールバック後にクリーンな状態に戻ることで完全にテスト可能。

**受け入れシナリオ**:

1. **前提条件** Issue選択済みでLaunch実行、**操作** Worktree作成後に`gh issue develop`が失敗、**期待結果** 作成済みWorktreeとローカルブランチが自動削除され、エラーメッセージが表示される
2. **前提条件** Issue選択済みでLaunch実行、**操作** `gh issue develop`成功後にエージェント起動が失敗、**期待結果** Worktree、ローカルブランチ、リモートブランチ（`git push origin --delete`）が自動削除され、エラーメッセージが表示される

## エッジケース

- gh CLIがインストールされていないまたは未認証 → 「From Issue」タブをdisabled + ツールチップ
- GitHubに接続できない（オフライン、レートリミット） → エラーメッセージ表示、リトライ不要
- Issue一覧が0件 → 「No open issues found」メッセージ表示
- Issueタイトルに特殊文字（引用符、HTML等）が含まれる → エスケープして正常表示
- 全ブランチタイプ（feature/bugfix/hotfix/release）でIssue連携が動作する
- ロールバック中にリモートブランチ削除が失敗した場合 → ローカルは削除しエラーログを出力（致命的ではない）
- レートリミットエラー受信時 → ユーザーに通知しリスト取得を中止

## 要件 *(必須)*

### 機能要件

#### バックエンド（gwt-core拡張）

- **FR-001**: `fetch_open_issues`にページネーション対応引数（`page: u32`, `per_page: u32`）を追加**しなければならない**
- **FR-001a**: デフォルト値は`page=1, per_page=50`とし、既存呼び出しとの後方互換性を保つ**こと**
- **FR-001b**: `gh issue list --limit {per_page}`のオフセットを`--json`出力のカーソルで制御する**こと**
- **FR-002**: `GitHubIssue`構造体に`labels: Vec<String>`フィールドを追加**しなければならない**（表示用）
- **FR-003**: `is_gh_cli_available`に加え、`is_gh_cli_authenticated`関数を追加**しなければならない**
- **FR-003a**: `gh auth status`コマンドで認証状態を確認する**こと**

#### バックエンド（Tauriコマンド）

- **FR-010**: `fetch_github_issues(project_path, page, per_page)` Tauriコマンドを公開**しなければならない**
- **FR-010a**: 戻り値は`{ issues: Vec<GitHubIssue>, has_next_page: bool }`の形式**であること**
- **FR-011**: `check_gh_cli_status(project_path)` Tauriコマンドを公開**しなければならない**
- **FR-011a**: 戻り値は`{ available: bool, authenticated: bool }`の形式**であること**
- **FR-012**: `find_existing_issue_branch(project_path, issue_number)` Tauriコマンドを公開**しなければならない**
- **FR-012a**: 戻り値は`Option<String>`（既存ブランチ名）**であること**
- **FR-013**: `link_branch_to_issue(project_path, issue_number, branch_name)` Tauriコマンドを公開**しなければならない**
- **FR-014**: `rollback_issue_branch(project_path, branch_name, delete_remote)` Tauriコマンドを公開**しなければならない**
- **FR-014a**: ローカルブランチ/Worktree削除 → リモートブランチ削除（`delete_remote=true`時）の順で実行する**こと**

#### フロントエンド（Svelte）

- **FR-019**: AgentLaunchFormのセクション順序をTUI版Wizardの流れに合わせて再配置**しなければならない**
- **FR-019a**: 順序は以下の通り: (1) Branch Mode → (2) Branch Config → (3) Agent Selection → (4) Model Selection + Claude Provider → (5) Agent Version → (6) Reasoning Level → (7) Session Mode → (8) Permissions → (9) Advanced Options → (10) Docker Runtime **であること**
- **FR-020**: AgentLaunchFormのNew Branchモード内に「Manual」「From Issue」のタブUIを追加**しなければならない**
- **FR-020a**: デフォルトは「Manual」タブ**であること**
- **FR-021**: 「From Issue」タブはgh CLI検出状態に応じてdisabled/enabled制御**しなければならない**
- **FR-021a**: disabled時はツールチップでgh CLIが必要な旨を表示する**こと**
- **FR-022**: AgentLaunchFormが開かれた時点でIssue一覧のバックグラウンド取得を開始**しなければならない**（Branch Modeに関わらず常に取得）
- **FR-023**: Issue一覧はテキスト検索による絞り込みを提供**しなければならない**
- **FR-023a**: 検索はクライアントサイドでタイトルの部分一致（大小文字不問）で行う**こと**
- **FR-024**: Issue一覧は無限スクロール（リスト末尾到達時にオンデマンドで次ページ取得）を実装**しなければならない**
- **FR-024a**: 取得中はリスト末尾にローディングインジケータを表示する**こと**
- **FR-024b**: 次ページがない場合は追加取得を行わない**こと**
- **FR-025**: 各Issue行は`#{number}: {title} (labels)`形式で表示**しなければならない**
- **FR-025a**: ラベルはカラーバッジとして表示する**こと**
- **FR-026**: 既存ブランチがあるIssueはリスト上でdisabled（選択不可）表示**しなければならない**
- **FR-026a**: disabled行には既存ブランチ名を表示する**こと**
- **FR-027**: Issue選択時、ブランチ名を`{prefix}/issue-{number}`形式で自動生成**しなければならない**
- **FR-027a**: 自動生成されたブランチ名はユーザーが編集不可**であること**（`gh issue develop`との整合性を保つため）
- **FR-027b**: Issueをクリック（シングルクリック）した時点でブランチ名が即座に確定する**こと**
- **FR-028**: Issue連携ブランチのLaunch時、`gh issue develop`でGitHub上のリンクを自動実行**しなければならない**
- **FR-029**: Launch途中の失敗時、完全ロールバック（ローカルブランチ/Worktree削除 + リモートブランチ削除）を実行**しなければならない**
- **FR-029a**: ロールバックの各ステップ（Worktree削除/ブランチ削除/リモートブランチ削除）の進捗をリアルタイムでUI表示する**こと**
- **FR-029b**: ロールバック中にリモートブランチ削除が失敗した場合、ローカル側の削除は完了した上でエラーログを出力する**こと**（致命的エラーとしない）
- **FR-030**: レートリミットエラー検出時、ユーザーに通知しリスト取得を中止**しなければならない**

### 非機能要件

- **NFR-001**: Issue一覧の初回取得は5秒以内に完了**しなければならない**（50件以下の場合）
- **NFR-002**: クライアントサイドテキスト検索は100ms以内に結果を更新**しなければならない**
- **NFR-003**: 無限スクロールの追加ページ取得は3秒以内に完了**しなければならない**

## 制約と仮定

### 制約

- gh CLI（GitHub CLI）がインストール・認証済みであることが前提（未検出時はFrom Issueタブを無効化）
- gwt-coreの既存`fetch_open_issues` APIを引数追加で拡張する（新規関数は作らない）
- TUI版は既に削除済みのため、後方互換性の考慮はgwt-core API呼び出し箇所のみ

### 仮定

- ユーザーはGitHub Issueを活用したワークフローを採用している
- gh CLIのレートリミットは通常使用で問題にならないが、エラーハンドリングは必要

## 成功基準 *(必須)*

- **SC-001**: Agent Launch FormでGitHub Issueを選択し、`{prefix}/issue-{number}`ブランチを自動生成できる
- **SC-002**: 無限スクロールで50件超のIssueを閲覧できる
- **SC-003**: 既存ブランチがあるIssueの選択が100%ブロックされる
- **SC-004**: gh CLI未検出環境でも既存のManualモードが正常に動作する
- **SC-005**: Launch失敗時に作成済みリソース（ブランチ/Worktree/リモートブランチ）が完全にロールバックされる
