# タスク: Claude Code / Codex CLI 対応の対話型Gitワークツリーマネージャー

**入力**: `/specs/SPEC-473b3d47/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、data-model.md、quickstart.md

**注**: この機能は既に実装済み（v0.6.1）です。タスクはテスト追加、ドキュメント改善、コード品質向上に焦点を当てています。

**テスト**: すべての機能に対してテストを追加することが推奨されます（現在テストは未実装）。

**構成**: タスクはユーザーストーリーごとにグループ化され、各ストーリーの独立した検証を可能にします。

## フォーマット: `[ID] [P?] [Story] Description`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[Story]**: このタスクが属するユーザーストーリー（例: US1、US2）
- 説明に正確なファイルパスを含める

## プロジェクト構造

```text
src/
├── index.ts                    # メインエントリーポイント
├── git.ts                      # Git操作
├── worktree.ts                 # ワークツリー管理
├── claude.ts / codex.ts        # AIツール統合
├── github.ts                   # GitHub統合
├── config/index.ts             # セッション管理
└── ui/                         # UI層

tests/                          # 新規作成
├── unit/                       # ユニットテスト
├── integration/                # 統合テスト
└── e2e/                        # E2Eテスト
```

## フェーズ1: テストインフラのセットアップ

**目的**: テストフレームワークと基盤を構築

### セットアップタスク

- [x] T001 [P] テストフレームワーク選定（Vitest推奨）を`package.json`に追加
- [x] T002 [P] テスト設定ファイル`vitest.config.ts`を作成
- [x] T003 [P] テストディレクトリ構造を作成: `tests/unit/`, `tests/integration/`, `tests/e2e/`
- [x] T004 [P] モックヘルパーを`tests/helpers/mocks.ts`に作成（execa, fs, inquirerのモック）
- [x] T005 [P] テスト用フィクスチャを`tests/fixtures/`に作成（サンプルGitリポジトリデータ）
- [x] T006 [P] CIワークフローを`.github/workflows/test.yml`に作成（GitHub Actions）
- [x] T007 [P] カバレッジレポート設定を`vitest.config.ts`に追加（80%目標）

**✅ チェックポイント**: テストフレームワークが動作し、サンプルテストが実行可能

## フェーズ2: US1 - 対話型ブランチ選択とワークツリー自動作成のテスト (優先度: P1)

**ストーリー**: ローカル・リモートブランチを選択し、ワークツリーを自動作成してAIツールを起動

**独立した検証**: ブランチ一覧→選択→ワークツリー作成→AIツール起動の一連のフローをテスト

### ユニットテスト

- [x] T101 [P] [US1] `src/git.ts`の`getAllBranches()`をテスト: `tests/unit/git.test.ts`
- [x] T102 [P] [US1] `src/git.ts`の`getLocalBranches()`をテスト: `tests/unit/git.test.ts`
- [x] T103 [P] [US1] `src/git.ts`の`getRemoteBranches()`をテスト: `tests/unit/git.test.ts`
- [x] T104 [P] [US1] `src/worktree.ts`の`worktreeExists()`をテスト: `tests/unit/worktree.test.ts`
- [x] T105 [P] [US1] `src/worktree.ts`の`generateWorktreePath()`をテスト: `tests/unit/worktree.test.ts`
- [x] T106 [P] [US1] `src/worktree.ts`の`createWorktree()`をテスト: `tests/unit/worktree.test.ts`
- [x] T107 [P] [US1] `src/ui/table.ts`の`createBranchTable()`をテスト: `tests/unit/worktree.test.ts`

### 統合テスト

- [ ] T108 [US1] T101-T106完了後、ブランチ選択からワークツリー作成までの統合テスト: `tests/integration/branch-selection.test.ts`
- [ ] T109 [US1] T108完了後、リモートブランチからローカルブランチ自動作成をテスト: `tests/integration/remote-branch.test.ts`

### E2Eテスト

- [ ] T110 [US1] T108-T109完了後、完全なユーザーフローをテスト（モックAIツール使用）: `tests/e2e/branch-to-worktree.test.ts`

**✅ MVP1チェックポイント**: US1の全受け入れシナリオがテストでカバーされ、独立して検証可能

## フェーズ3: US2 - スマートブランチ作成ワークフローのテスト (優先度: P1)

**ストーリー**: feature/hotfix/releaseタイプのブランチを作成し、ワークツリーをセットアップ

**独立した検証**: ブランチタイプ選択→名前入力→ベースブランチ選択→ワークツリー作成をテスト

### ユニットテスト

- [ ] T201 [P] [US2] `src/git.ts`の`createBranch()`をテスト: `tests/unit/git.test.ts`
- [ ] T202 [P] [US2] `src/git.ts`の`branchExists()`をテスト: `tests/unit/git.test.ts`
- [ ] T203 [P] [US2] `src/git.ts`のブランチタイプ決定ロジックをテスト: `tests/unit/git.test.ts`
- [ ] T204 [P] [US2] `src/git.ts`の`getCurrentVersion()`をテスト: `tests/unit/git.test.ts`
- [ ] T205 [P] [US2] `src/git.ts`の`calculateNewVersion()`をテスト: `tests/unit/git.test.ts`
- [ ] T206 [P] [US2] `src/git.ts`の`executeNpmVersionInWorktree()`をテスト: `tests/unit/git.test.ts`

### 統合テスト

- [ ] T207 [US2] T201-T203完了後、feature/hotfixブランチ作成フローをテスト: `tests/integration/branch-creation.test.ts`
- [ ] T208 [US2] T204-T206完了後、releaseブランチ作成とバージョン更新をテスト: `tests/integration/release-branch.test.ts`

### E2Eテスト

- [ ] T209 [US2] T207-T208完了後、全ブランチタイプの作成フローをテスト: `tests/e2e/create-branch-workflow.test.ts`

**✅ MVP2チェックポイント**: US2の全受け入れシナリオがテストでカバーされ、独立して検証可能

## フェーズ4: US3 - セッション管理と継続機能のテスト (優先度: P2)

**ストーリー**: `-c`オプションで最後のセッションを継続、`-r`オプションでセッション選択

**独立した検証**: セッション保存→継続/選択→ワークツリー復元をテスト

### ユニットテスト

- [ ] T301 [P] [US3] `src/config/index.ts`の`saveSession()`をテスト: `tests/unit/config/session.test.ts`
- [ ] T302 [P] [US3] `src/config/index.ts`の`loadSession()`をテスト: `tests/unit/config/session.test.ts`
- [ ] T303 [P] [US3] `src/config/index.ts`の`getAllSessions()`をテスト: `tests/unit/config/session.test.ts`

### 統合テスト

- [ ] T304 [US3] T301-T303完了後、`-c`オプションによるセッション継続をテスト: `tests/integration/session-continue.test.ts`
- [ ] T305 [US3] T301-T303完了後、`-r`オプションによるセッション選択をテスト: `tests/integration/session-resume.test.ts`
- [ ] T306 [US3] T304完了後、存在しないワークツリーのフォールバック処理をテスト: `tests/integration/session-fallback.test.ts`

### E2Eテスト

- [ ] T307 [US3] T304-T306完了後、セッション継続の完全なフローをテスト: `tests/e2e/session-workflow.test.ts`

**✅ チェックポイント**: US3の全受け入れシナリオがテストでカバーされ、独立して検証可能

## フェーズ5: US7 - AIツール統合と実行モード管理のテスト (優先度: P1)

**ストーリー**: Claude Code / Codex CLIを選択・起動し、実行モードと権限を管理

**独立した検証**: AIツール選択→実行モード選択→起動をテスト

### ユニットテスト

- [ ] T401 [P] [US7] `src/claude.ts`の`launchClaudeCode()`をテスト: `tests/unit/claude.test.ts`
- [ ] T402 [P] [US7] `src/claude.ts`の`isClaudeCodeAvailable()`をテスト: `tests/unit/claude.test.ts`
- [ ] T403 [P] [US7] `src/codex.ts`の`launchCodexCLI()`をテスト: `tests/unit/codex.test.ts`
- [ ] T404 [P] [US7] `src/codex.ts`の`isCodexAvailable()`をテスト: `tests/unit/codex.test.ts`

### 統合テスト

- [ ] T405 [US7] T401-T402完了後、Claude Code起動フローをテスト: `tests/integration/claude-launch.test.ts`
- [ ] T406 [US7] T403-T404完了後、Codex CLI起動フローをテスト: `tests/integration/codex-launch.test.ts`
- [ ] T407 [US7] T405-T406完了後、`--tool`オプションによる直接指定をテスト: `tests/integration/tool-selection.test.ts`
- [ ] T408 [US7] T405-T406完了後、引数パススルー（`--`以降）をテスト: `tests/integration/tool-passthrough.test.ts`

### E2Eテスト

- [ ] T409 [US7] T405-T408完了後、AIツール統合の完全なフローをテスト: `tests/e2e/ai-tool-integration.test.ts`

**✅ チェックポイント**: US7の全受け入れシナリオがテストでカバーされ、独立して検証可能

## フェーズ6: US8 - 変更管理と開発セッション終了処理のテスト (優先度: P2)

**ストーリー**: AIツール終了後、未コミット変更をcommit/stash/discardで処理

**独立した検証**: 変更検出→アクション選択→実行をテスト

### ユニットテスト

- [ ] T501 [P] [US8] `src/git.ts`の`hasUncommittedChanges()`をテスト: `tests/unit/git.test.ts`
- [ ] T502 [P] [US8] `src/git.ts`の`showStatus()`をテスト: `tests/unit/git.test.ts`
- [ ] T503 [P] [US8] `src/git.ts`の`commitChanges()`をテスト: `tests/unit/git.test.ts`
- [ ] T504 [P] [US8] `src/git.ts`の`stashChanges()`をテスト: `tests/unit/git.test.ts`
- [ ] T505 [P] [US8] `src/git.ts`の`discardAllChanges()`をテスト: `tests/unit/git.test.ts`

### 統合テスト

- [ ] T506 [US8] T501-T505完了後、変更管理フロー（commit）をテスト: `tests/integration/changes-commit.test.ts`
- [ ] T507 [US8] T501-T505完了後、変更管理フロー（stash）をテスト: `tests/integration/changes-stash.test.ts`
- [ ] T508 [US8] T501-T505完了後、変更管理フロー（discard）をテスト: `tests/integration/changes-discard.test.ts`

### E2Eテスト

- [ ] T509 [US8] T506-T508完了後、変更管理の完全なフローをテスト: `tests/e2e/changes-management.test.ts`

**✅ チェックポイント**: US8の全受け入れシナリオがテストでカバーされ、独立して検証可能

## フェーズ7: US4 - マージ済みPRクリーンアップのテスト (優先度: P2)

**ストーリー**: GitHub CLI連携でマージ済みPRを検出し、ブランチとワークツリーを一括削除

**独立した検証**: PR検出→選択→クリーンアップ実行をテスト

### ユニットテスト

- [ ] T601 [P] [US4] `src/github.ts`の`getMergedPullRequests()`をテスト: `tests/unit/github.test.ts`
- [ ] T602 [P] [US4] `src/github.ts`の`isGitHubCLIAvailable()`をテスト: `tests/unit/github.test.ts`
- [ ] T603 [P] [US4] `src/github.ts`の`checkGitHubAuth()`をテスト: `tests/unit/github.test.ts`
- [ ] T604 [P] [US4] `src/worktree.ts`の`getMergedPRWorktrees()`をテスト: `tests/unit/worktree.test.ts`
- [ ] T605 [P] [US4] `src/git.ts`の`deleteBranch()`をテスト: `tests/unit/git.test.ts`
- [ ] T606 [P] [US4] `src/git.ts`の`deleteRemoteBranch()`をテスト: `tests/unit/git.test.ts`
- [ ] T607 [P] [US4] `src/git.ts`の`pushBranchToRemote()`をテスト: `tests/unit/git.test.ts`

### 統合テスト

- [ ] T608 [US4] T601-T604完了後、PR検出とCleanupTarget生成をテスト: `tests/integration/pr-detection.test.ts`
- [ ] T609 [US4] T605-T607完了後、クリーンアップ実行（ローカルのみ）をテスト: `tests/integration/cleanup-local.test.ts`
- [ ] T610 [US4] T605-T607完了後、クリーンアップ実行（リモート含む）をテスト: `tests/integration/cleanup-remote.test.ts`
- [ ] T611 [US4] T607完了後、未プッシュコミット処理をテスト: `tests/integration/cleanup-unpushed.test.ts`

### E2Eテスト

- [ ] T612 [US4] T608-T611完了後、PRクリーンアップの完全なフローをテスト: `tests/e2e/pr-cleanup.test.ts`

**✅ チェックポイント**: US4の全受け入れシナリオがテストでカバーされ、独立して検証可能

## フェーズ8: US5 - ワークツリー管理のテスト (優先度: P2)

**ストーリー**: 既存ワークツリーを一覧表示し、開く/削除操作を実行

**独立した検証**: ワークツリー一覧→選択→アクション実行をテスト

### ユニットテスト

- [ ] T701 [P] [US5] `src/worktree.ts`の`listAdditionalWorktrees()`をテスト: `tests/unit/worktree.test.ts`
- [ ] T702 [P] [US5] `src/worktree.ts`の`removeWorktree()`をテスト: `tests/unit/worktree.test.ts`

### 統合テスト

- [ ] T703 [US5] T701完了後、ワークツリー一覧表示をテスト: `tests/integration/worktree-list.test.ts`
- [ ] T704 [US5] T702完了後、ワークツリー削除（ワークツリーのみ）をテスト: `tests/integration/worktree-remove.test.ts`
- [ ] T705 [US5] T702完了後、ワークツリー削除（ブランチも）をテスト: `tests/integration/worktree-remove-branch.test.ts`
- [ ] T706 [US5] T702完了後、アクセス不可能なワークツリー削除をテスト: `tests/integration/worktree-force-remove.test.ts`

### E2Eテスト

- [ ] T707 [US5] T703-T706完了後、ワークツリー管理の完全なフローをテスト: `tests/e2e/worktree-management.test.ts`

**✅ チェックポイント**: US5の全受け入れシナリオがテストでカバーされ、独立して検証可能

## フェーズ9: US6 - リリース管理とGit Flowのテスト (優先度: P3)

**ストーリー**: Git Flow準拠のリリースブランチ作成、バージョン管理、PR自動作成

**独立した検証**: リリースブランチ作成→バージョン更新→PR作成をテスト

### ユニットテスト

- [ ] T801 [P] [US6] `src/git.ts`のバージョン計算ロジックをテスト: `tests/unit/git.test.ts`

### 統合テスト

- [ ] T802 [US6] T801完了後、リリースブランチ作成フローをテスト: `tests/integration/release-creation.test.ts`
- [ ] T803 [US6] T802完了後、リリース完了とPR作成をテスト: `tests/integration/release-complete.test.ts`

### E2Eテスト

- [ ] T804 [US6] T802-T803完了後、リリース管理の完全なフローをテスト: `tests/e2e/release-workflow.test.ts`

**✅ チェックポイント**: US6の全受け入れシナリオがテストでカバーされ、独立して検証可能

## フェーズ10: UI層とエラーハンドリングのテスト

**目的**: UI層とエラーハンドリングの品質を保証

### ユニットテスト

- [ ] T901 [P] `src/ui/display.ts`の出力フォーマット関数をテスト: `tests/unit/ui/display.test.ts`
- [ ] T902 [P] `src/ui/table.ts`のテーブル生成ロジックをテスト: `tests/unit/ui/table.test.ts`
- [ ] T903 [P] `src/utils.ts`のエラーハンドリングをテスト: `tests/unit/utils.test.ts`
- [ ] T904 [P] `src/utils.ts`の`setupExitHandlers()`をテスト: `tests/unit/utils.test.ts`

### 統合テスト

- [ ] T905 エッジケースハンドリング（Gitリポジトリでない）をテスト: `tests/integration/edge-cases.test.ts`
- [ ] T906 エッジケースハンドリング（AIツール未インストール）をテスト: `tests/integration/edge-cases.test.ts`
- [ ] T907 エッジケースハンドリング（GitHub CLI未認証）をテスト: `tests/integration/edge-cases.test.ts`

**✅ チェックポイント**: UI層とエラーハンドリングが適切にテストされている

## フェーズ11: ドキュメント改善

**目的**: ドキュメントの充実とメンテナンス性向上

### ドキュメントタスク

- [ ] T1001 [P] [Doc] APIドキュメントを`docs/api.md`に作成（全公開関数）
- [ ] T1002 [P] [Doc] アーキテクチャドキュメントを`docs/architecture.md`に作成
- [ ] T1003 [P] [Doc] コントリビューションガイドを`CONTRIBUTING.md`に作成
- [ ] T1004 [P] [Doc] トラブルシューティングガイドを`docs/troubleshooting.md`に作成
- [ ] T1005 [P] [Doc] 変更履歴を`CHANGELOG.md`に整理（Keep a Changelog形式）
- [ ] T1006 [P] [Doc] TypeScriptドキュメントコメント（JSDoc）を主要関数に追加

**✅ チェックポイント**: ドキュメントが充実し、新規コントリビューターが理解しやすい

## フェーズ12: コード品質とCI/CD

**目的**: コード品質の自動化とCI/CDパイプライン構築

### コード品質タスク

- [ ] T1101 [P] [Quality] ESLint設定を強化（既存のeslint.config.jsを拡張）
- [ ] T1102 [P] [Quality] Prettier設定を追加（`.prettierrc.json`）
- [ ] T1103 [P] [Quality] commitlint設定を検証（`.commitlintrc.json`）
- [ ] T1104 [P] [Quality] markdownlint設定を検証（既存の`.markdownlint.json`）
- [ ] T1105 [P] [Quality] pre-commitフックを`.husky/`に設定（lint + test）

### CI/CDタスク

- [ ] T1106 [CI] T006完了後、テストワークフロー（`.github/workflows/test.yml`）を強化
- [ ] T1107 [P] [CI] lintワークフローを`.github/workflows/lint.yml`に作成
- [ ] T1108 [P] [CI] リリースワークフローを`.github/workflows/release.yml`に作成
- [ ] T1109 [P] [CI] カバレッジレポートをCodecov/Coverallsに統合

**✅ チェックポイント**: CI/CDが自動化され、コード品質が保証されている

## フェーズ13: リファクタリング（オプション）

**目的**: コードの保守性とテスト性を向上

### リファクタリングタスク

- [ ] T1201 [Refactor] `src/index.ts`の長い関数を分割（1000行超を500行以下に）
- [ ] T1202 [Refactor] T1201完了後、`handleSelection()`を小さな関数に分解
- [ ] T1203 [Refactor] T1201完了後、`handleBranchSelection()`を小さな関数に分解
- [ ] T1204 [Refactor] T1201完了後、`handleCreateNewBranch()`を小さな関数に分解
- [ ] T1205 [Refactor] T1201完了後、`handleManageWorktrees()`を小さな関数に分解
- [ ] T1206 [Refactor] T1201完了後、`handleCleanupMergedPRs()`を小さな関数に分解
- [ ] T1207 [Refactor] T1201完了後、`handlePostClaudeChanges()`を小さな関数に分解

### サービス層実装（オプション）

- [ ] T1208 [P] [Refactor] `src/services/git.service.ts`を実装（`src/git.ts`から移行）
- [ ] T1209 [P] [Refactor] `src/services/worktree.service.ts`を実装（`src/worktree.ts`から移行）
- [ ] T1210 [P] [Refactor] `src/services/github.service.ts`を実装（`src/github.ts`から移行）
- [ ] T1211 [Refactor] T1208-T1210完了後、`src/index.ts`からサービス層を呼び出すように変更

### リポジトリ層実装（オプション）

- [ ] T1212 [P] [Refactor] `src/repositories/git.repository.ts`を実装
- [ ] T1213 [P] [Refactor] `src/repositories/worktree.repository.ts`を実装
- [ ] T1214 [P] [Refactor] `src/repositories/github.repository.ts`を実装
- [ ] T1215 [Refactor] T1212-T1214完了後、サービス層からリポジトリ層を呼び出すように変更

**✅ チェックポイント**: コードがレイヤードアーキテクチャに完全準拠し、保守性が向上

## タスク凡例

- **T###**: タスクID（実行順序を示す）
- **[P]**: 並列実行可能（他のタスクと同時に実行可能）
- **[US#]**: ユーザーストーリー番号（spec.mdに対応）
- **[Doc]**: ドキュメントタスク
- **[Quality]**: コード品質タスク
- **[CI]**: CI/CDタスク
- **[Refactor]**: リファクタリングタスク

## 依存関係と実行順序

### 必須フェーズ（順次実行）

1. **フェーズ1**: テストインフラ（T001-T007） → すべてのテストフェーズの前提条件
2. **フェーズ2-9**: ユーザーストーリー別テスト → 独立実行可能（優先度順推奨）
3. **フェーズ10**: UI/エラーハンドリング → フェーズ2-9と並行可能
4. **フェーズ11**: ドキュメント → いつでも実行可能（並行推奨）
5. **フェーズ12**: CI/CD → フェーズ1完了後いつでも可能

### オプションフェーズ

- **フェーズ13**: リファクタリング → すべてのテスト完了後推奨

### 並列実行の機会

**グループA（テストインフラ後、並列実行可能）**:
- フェーズ2（US1）、フェーズ3（US2）、フェーズ4（US3）、フェーズ5（US7）
- これらは独立したストーリーのため、完全に並列で開発・テスト可能

**グループB（並列実行可能）**:
- フェーズ6（US8）、フェーズ7（US4）、フェーズ8（US5）、フェーズ9（US6）

**グループC（いつでも並列実行可能）**:
- フェーズ11（ドキュメント）の全タスク（T1001-T1006）
- フェーズ12（CI/CD）の一部タスク（T1107-T1109）

## MVP定義

### MVP1（最小限の機能）
- フェーズ1（テストインフラ）
- フェーズ2（US1: ブランチ選択とワークツリー作成）
- フェーズ5（US7: AIツール統合）

**価値**: 基本的なブランチ選択とワークツリー作成が検証済み

### MVP2（実用レベル）
- MVP1 +
- フェーズ3（US2: ブランチ作成）
- フェーズ4（US3: セッション管理）

**価値**: 日常的な開発ワークフローに対応

### 完全機能
- MVP2 +
- フェーズ6-9（US8, US4, US5, US6）
- フェーズ10-12（UI/エラー、ドキュメント、CI/CD）

**価値**: すべての機能がテストされ、プロダクション準備完了

## 実装戦略

1. **既存実装のテスト追加**: すべての機能は実装済みのため、テストのみを追加
2. **ストーリーごとの検証**: 各ユーザーストーリーを独立してテスト可能にする
3. **段階的な品質向上**: テスト → ドキュメント → CI/CD → リファクタリング
4. **並列開発**: 独立したストーリーは並列でテスト追加可能
5. **継続的な改善**: リファクタリングはオプションとし、テストとドキュメントを優先

## サマリー

- **総タスク数**: 約130タスク
- **並列実行可能**: 約70タスク（[P]マーク）
- **ユーザーストーリー**: 8ストーリー
- **独立テスト可能**: 各ストーリーごとに独立検証
- **推奨MVP**: フェーズ1-5（テストインフラ + US1 + US7）
- **テストカバレッジ目標**: 80%以上
