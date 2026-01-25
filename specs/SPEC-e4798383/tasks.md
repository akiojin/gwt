# タスク: GitHub Issue連携によるブランチ作成

**入力**: `/specs/SPEC-e4798383/` からの設計ドキュメント
**前提条件**: spec.md（ユーザーストーリー用に必須）

**構成**: タスクはユーザーストーリーごとにグループ化され、各ストーリーの独立した実装とテストを可能にします。

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3）
- 説明に正確なファイルパスを含める

## フェーズ1: 基盤（型定義とデータモデル）

**目的**: GitHub Issue連携の基本構造を確立

### 型定義の追加

- [x] **T001** [P] [基盤] crates/gwt-core/src/git/issue.rs を新規作成し、GitHubIssue構造体（number: u64, title: String, updated_at: String）を定義
- [x] **T002** [P] [基盤] crates/gwt-core/src/git.rs にissueモジュールをエクスポート追加
- [x] **T003** [P] [基盤] crates/gwt-cli/src/tui/screens/wizard.rs のWizardStepにIssueSelectを追加

### WizardStateの拡張

- [x] **T004** [基盤] crates/gwt-cli/src/tui/screens/wizard.rs のWizardStateにselected_issue: Option<GitHubIssue>フィールドを追加
- [x] **T005** [基盤] crates/gwt-cli/src/tui/screens/wizard.rs のWizardStateにissue_list: Vec<GitHubIssue>フィールドを追加
- [x] **T006** [基盤] crates/gwt-cli/src/tui/screens/wizard.rs のWizardStateにissue_search_query: Stringフィールドを追加
- [x] **T007** [基盤] crates/gwt-cli/src/tui/screens/wizard.rs のWizardStateにissue_selected_index: usizeフィールドを追加

## フェーズ2: ユーザーストーリー1 - GitHub Issueを選択してブランチを作成できる (優先度: P1)

**ストーリー**: 開発者がブランチ作成時にGitHub Issueを選択し、Issue番号を含む命名規則に従ったブランチを自動生成できる。

**価値**: Issue駆動開発において、ブランチとIssueの紐付けを自動化し、ミスを防止。

### gh CLI連携

- [x] **T101** [US1] crates/gwt-core/src/git/issue.rs にfetch_open_issues()関数を実装（gh issue list --state open --json number,title,updatedAt --limit 50）
- [x] **T102** [US1] crates/gwt-core/src/git/issue.rs にparse_gh_issues_json()関数を実装（JSONパース）
- [x] **T103** [US1] crates/gwt-core/src/git/issue.rs にis_gh_cli_available()関数を実装（gh CLI存在チェック）

### ウィザードフロー

- [x] **T104** [US1] crates/gwt-cli/src/tui/screens/wizard.rs のnext_step()関数にBranchTypeSelect → IssueSelectの遷移を追加
- [x] **T105** [US1] crates/gwt-cli/src/tui/screens/wizard.rs のnext_step()関数にIssueSelect → BranchNameInputの遷移を追加
- [x] **T106** [US1] crates/gwt-cli/src/tui/screens/wizard.rs のprev_step()関数にIssueSelect → BranchTypeSelectの遷移を追加

### ブランチ名自動生成

- [x] **T107** [US1] crates/gwt-core/src/git/issue.rs にgenerate_branch_name()関数を実装（{type}/issue-{number}形式）
- [x] **T108** [US1] crates/gwt-cli/src/tui/screens/wizard.rs のIssue選択確定時にnew_branch_nameを自動設定する処理を追加

### UI実装

- [x] **T109** [US1] crates/gwt-cli/src/tui/screens/wizard.rs にrender_issue_select_step()関数を実装
- [x] **T109a** [US1] Issue取得中は「Loading issues...」を表示する処理を実装
- [x] **T109b** [US1] Issue0件時は「No open issues」を表示する処理を実装
- [x] **T110** [US1] render_issue_select_step()でIssue一覧を「#番号: タイトル」形式で表示
- [x] **T111** [US1] crates/gwt-cli/src/tui/screens/wizard.rs のrender_wizard()にWizardStep::IssueSelectのケースを追加
- [x] **T112** [US1] crates/gwt-cli/src/tui/screens/wizard.rs のget_step_title()に「GitHub Issue」を追加

### 確認画面

- [x] **T113** [US1] crates/gwt-cli/src/tui/screens/wizard.rs のrender_branch_name_step()にIssue情報表示を追加（FR-014）

### テスト

- [x] **T114** [P] [US1] crates/gwt-core/src/git/issue.rs にparse_gh_issues_json()のユニットテストを追加
- [x] **T115** [P] [US1] crates/gwt-core/src/git/issue.rs にgenerate_branch_name()のユニットテストを追加

**MVP1チェックポイント**: US1完了後、Issueを選択してブランチ名を自動生成できる独立した機能を提供

## フェーズ3: ユーザーストーリー2 - Issue選択をスキップして従来通りブランチを作成できる (優先度: P1)

**ストーリー**: 開発者がIssue連携を使わず、従来通り手動でブランチ名を入力して作成できる。

**価値**: 柔軟性を確保し、Issue駆動でない作業にも対応。

### スキップ機能

- [x] **T201** [US2] crates/gwt-cli/src/tui/screens/wizard.rs のhandle_issue_select_input()で空Enterでスキップする処理を実装
- [x] **T202** [US2] スキップ時はselected_issueをNoneのまま維持し、new_branch_nameを空のまま次のステップへ遷移

### gh CLI未インストール時の自動スキップ

- [x] **T203** [US2] crates/gwt-cli/src/tui/screens/wizard.rs のnext_step()でgh CLI未インストール時にIssueSelectをスキップ
- [x] **T204** [US2] スキップ時のユーザーへの通知は不要（シームレスに次のステップへ）

### テスト

- [ ] **T205** [P] [US2] スキップ時にnew_branch_nameが空のままであることのユニットテストを追加

**MVP2チェックポイント**: US2完了後、Issue連携なしでも従来通りブランチ作成可能

## フェーズ4: ユーザーストーリー3 - インクリメンタル検索でIssueを絞り込める (優先度: P2)

**ストーリー**: 開発者がIssue一覧から目的のIssueをキーワード入力で素早く見つけられる。

**価値**: 多数のIssueがある場合の作業効率向上。

### 検索機能

- [x] **T301** [US3] crates/gwt-core/src/git/issue.rs にfilter_issues_by_title()関数を実装
- [x] **T302** [US3] handle_issue_select_input()でキー入力時にissue_search_queryを更新
- [x] **T303** [US3] render_issue_select_step()でフィルタリング済みリストを表示
- [x] **T304** [US3] 検索クエリの表示UIを追加（入力欄）

### ソート

- [x] **T305** [US3] parse_gh_issues_json()で更新日時降順ソートを実装

### テスト

- [x] **T306** [P] [US3] filter_issues_by_title()のユニットテストを追加（部分一致、大文字小文字無視）

**MVP3チェックポイント**: US3完了後、検索によるIssue絞り込み可能

## フェーズ5: ユーザーストーリー4 - 全ブランチタイプでIssue連携が使える (優先度: P1)

**ストーリー**: feature、bugfix、hotfix、releaseの全ブランチタイプでIssue連携機能が利用できる。

**価値**: ブランチタイプに関係なくIssue連携の恩恵を受けられる。

### 全タイプ対応

- [x] **T401** [US4] generate_branch_name_from_issue()が全BranchType（Feature, Bugfix, Hotfix, Release）に対応していることを確認
- [x] **T402** [P] [US4] 各BranchTypeでのブランチ名生成のユニットテストを追加

**注**: 基本実装（US1）で全タイプ対応済み。テスト（test_generate_branch_name_feature/bugfix/hotfix/release）で確認完了

## フェーズ6: ユーザーストーリー5 - 同一Issueで重複ブランチを防止できる (優先度: P2)

**ストーリー**: 開発者が既にブランチが存在するIssueを選択しようとした場合、重複作成を防止する。

**価値**: 同一Issueに複数ブランチが存在することによる混乱を防止。

### 重複チェック

- [x] **T501** [US5] crates/gwt-core/src/git/issue.rs にfind_branch_for_issue()関数を実装（issue-{number}を含むブランチを検索）
- [x] **T502** [US5] crates/gwt-cli/src/tui/screens/wizard.rs のIssue選択時に重複チェックを追加
- [x] **T503** [US5] 重複時のエラーメッセージ表示を実装

### テスト

- [ ] **T504** [P] [US5] find_branch_for_issue()のユニットテストを追加
- [ ] **T505** [P] [US5] 重複ブランチ選択時のブロック動作のテストを追加

## フェーズ7: エラーハンドリング

**目的**: 堅牢なエラー処理を実装

### オフライン・認証エラー

- [x] **T601** [エラー] fetch_open_issues()でgh CLI実行エラー時のResult型を適切に返す
- [x] **T602** [エラー] crates/gwt-cli/src/tui/screens/wizard.rs でIssue取得失敗時のエラーメッセージ表示を実装
- [x] **T603** [エラー] エラー発生時は従来フロー（Issue選択スキップ）へ誘導

### Issue存在確認

- [ ] **T604** [エラー] 手動入力されたIssue番号の存在確認機能を実装（オプション）
- [ ] **T605** [エラー] 存在しないIssue番号の場合、警告表示後に続行を許可

## フェーズ8: 検証と統合

**目的**: すべてのストーリーを統合し、品質を確認

### ビルドとテスト

- [x] **T701** [統合] cargo build --release でビルドエラーなしを確認
- [x] **T702** [統合] cargo test で全テストを実行し、既存テストが全てパスすることを確認
- [x] **T703** [統合] cargo clippy --all-targets --all-features -- -D warnings でLintチェック
- [x] **T704** [統合] cargo fmt でフォーマット確認

### 手動テスト

- [ ] **T705** [統合] 実際のGitHubリポジトリでIssue選択→ブランチ作成の一連の動作を確認
- [ ] **T706** [統合] gh CLIなし環境でのスキップ動作を確認
- [ ] **T707** [統合] オフライン環境でのエラーハンドリングを確認

### コミットとプッシュ

- [x] **T708** [統合] 全変更をConventional Commits形式でコミット
- [x] **T709** [統合] git push でリモートリポジトリにプッシュ

## タスク凡例

**優先度**:

- **P1**: 最も重要 - MVP1に必要
- **P2**: 重要 - MVP2/3に必要

**依存関係**:

- **[P]**: 並列実行可能
- **[依存なし]**: 他のタスクの後に実行

**ストーリータグ**:

- **[US1]**: ユーザーストーリー1 - GitHub Issueを選択してブランチを作成できる
- **[US2]**: ユーザーストーリー2 - Issue選択をスキップして従来通りブランチを作成できる
- **[US3]**: ユーザーストーリー3 - インクリメンタル検索でIssueを絞り込める
- **[US4]**: ユーザーストーリー4 - 全ブランチタイプでIssue連携が使える
- **[US5]**: ユーザーストーリー5 - 同一Issueで重複ブランチを防止できる
- **[基盤]**: すべてのストーリーで共有される基盤
- **[統合]**: 複数ストーリーにまたがる統合タスク
- **[エラー]**: エラーハンドリング

## 実装戦略

### MVPインクリメント

1. **MVP1 (US1完了時)**: Issue選択→ブランチ名自動生成
   - 基盤型定義、gh CLI連携、UI、ブランチ名生成
   - この時点で独立した価値を提供

2. **MVP2 (US2完了時)**: スキップ機能
   - Issue連携なしでも従来通り使用可能
   - gh CLI未インストール環境対応

3. **MVP3 (US3完了時)**: 検索機能
   - 大量Issue対応
   - 作業効率向上

4. **完全機能 (US4+US5+エラー完了時)**: 全機能統合
   - 全ブランチタイプ対応確認
   - 重複防止
   - 堅牢なエラーハンドリング

### 並列実行の機会

**フェーズ1（基盤）**: T001-T003は並列実行可能（異なるファイル）

**フェーズ2（US1）**:

- T114-T115（テスト）は並列実行可能

**フェーズ4（US3）**:

- T306（テスト）は実装完了後に並列実行可能

**フェーズ6（US5）**:

- T504-T505（テスト）は並列実行可能

## 進捗追跡

- **完了したタスク**: 48/54 (89%)
  - フェーズ1（基盤）: 7/7 (100%)
  - フェーズ2（US1）: 17/17 (100%)
  - フェーズ3（US2）: 4/5 (80%) - T205テストのみ未実装
  - フェーズ4（US3）: 6/6 (100%)
  - フェーズ5（US4）: 2/2 (100%)
  - フェーズ6（US5）: 3/5 (60%) - T504-T505テストのみ未実装
  - フェーズ7（エラー）: 3/5 (60%) - T604-T605未実装（オプション）
  - フェーズ8（統合）: 6/9 (67%) - T705-T707残り（手動テストのみ）
