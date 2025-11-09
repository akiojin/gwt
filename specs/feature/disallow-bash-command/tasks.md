# タスク: Worktree内でのコマンド実行制限機能

**入力**: `/specs/SPEC-eae13040/` および `/specs/feature/disallow-bash-command/` からの設計ドキュメント
**前提条件**: plan.md（必須）、spec.md（ユーザーストーリー用に必須）、research.md、data-model.md、quickstart.md

**テスト**: 仕様で明示的にテストが要求されているため、各フェーズにテストタスクを含めます。

**構成**: タスクはユーザーストーリーごとにグループ化され、各ストーリーの独立した実装とテストを可能にします。

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3、US4）
- 説明に正確なファイルパスを含める

## Commitlintルール

- コミットメッセージは件名のみを使用し、空にしてはいけません（`commitlint.config.cjs`の`subject-empty`ルール）。
- 件名は100文字以内に収めてください（`subject-max-length`ルール）。
- タスク生成時は、これらのルールを満たすコミットメッセージが書けるよう変更内容を整理してください。

## Lint最小要件

- `.github/workflows/lint.yml` に対応するため、以下のチェックがローカルで成功することをタスク完了条件に含めてください。
  - `bun run format:check`
  - `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore`
  - `bun run lint`

> ⚠️ Markdown整形ヒント
> - ネストした箇条書きは 2 スペースでインデントしてください。
> - 外部リンクは `[タイトル](https://example.com)` 形式で記載し、裸URLを避けてください。

## パス規約

- **フックスクリプト**: `.claude/hooks/`
- **テスト**: `tests/hooks/`
- **ドキュメント**: `specs/`、`README.md`

## フェーズ1: セットアップ（共有インフラストラクチャ）

**目的**: フックスクリプト開発環境のセットアップと共通ユーティリティの準備

### セットアップタスク

- [ ] **T001** [P] 依存関係の確認(jq、git、realpath、python3、shellcheck)とREADME.mdへのインストール手順追加
- [ ] **T002** [P] tests/hooks/ディレクトリ作成とBatsテストフレームワークのセットアップ
- [ ] **T003** [P] .claude/hooks/common.shに共通ユーティリティ関数(is_within_worktree等)を抽出して共通化

## フェーズ2: ユーザーストーリー1 - Bashコマンドによる作業ディレクトリ保護 (優先度: P1)

**ストーリー**: Claude CodeがWorktree内で作業する際、意図しない作業ディレクトリの変更やWorktree外部へのディレクトリ移動を防止し、常にWorktree内で作業が完結することを保証する。

**価値**: Worktreeの基本設計思想である「起動ブランチで作業を完結する」を技術的に保証する最重要機能。

### フックスクリプト改善

- [ ] **T101** [US1] .claude/hooks/block-cd-command.shにPython shlex.split()によるコマンド解析を追加(既存のsedベース分割のフォールバックとして残す)
- [ ] **T102** [US1] T101の後に.claude/hooks/block-cd-command.shの is_within_worktree()関数にrealpathフォールバック実装(Pythonを使用)を追加
- [ ] **T103** [US1] T101の後に.claude/hooks/block-cd-command.shの相対パス解決ロジックを強化(存在しないディレクトリも正しく判定)

### テスト

- [ ] **T104** [P] [US1] tests/hooks/test-cd-command.batsにWorktree内へのcd許可テストを追加(cd ./src, cd ../等)
- [ ] **T105** [P] [US1] tests/hooks/test-cd-command.batsにWorktree外へのcdブロックテストを追加(cd /tmp, cd ~等)
- [ ] **T106** [P] [US1] tests/hooks/test-cd-command.batsにシンボリックリンク経由のアクセステストを追加
- [ ] **T107** [P] [US1] tests/hooks/test-cd-command.batsに相対パス(`../../..`)でのWorktree外アクセステストを追加
- [ ] **T108** [P] [US1] tests/hooks/test-cd-command.batsに複合コマンド(`echo test && cd /tmp`)のブロックテストを追加

### 検証

- [ ] **T109** [US1] T101-T103完了後にBatsテスト(T104-T108)を実行し、全テストが成功することを確認
- [ ] **T110** [US1] T109完了後にShellCheckでblock-cd-command.shを解析し、警告を修正

**✅ MVP1チェックポイント**: US1完了後、cdコマンド制限が完全に動作し、Worktree境界保護が機能

## フェーズ3: ユーザーストーリー2 - Gitブランチ操作の制御 (優先度: P1)

**ストーリー**: Claude Codeがgitコマンドを実行する際、参照系の操作(ブランチ情報の確認)は許可しつつ、ブランチの切り替えや作成・削除などの変更操作を制限する。

**価値**: Worktreeの「起動ブランチで作業を完結する」という設計思想を保証。ブランチが切り替わると、作業対象が変わり意図しない変更が発生する。

### フックスクリプト改善

- [ ] **T201** [US2] .claude/hooks/block-git-branch-ops.shに`git checkout -- file`パターン判定を追加(ファイル復元は許可)
- [ ] **T202** [US2] T201の後に.claude/hooks/block-git-branch-ops.shのコマンド判定ロジックを修正(git branchを先にチェックし、参照系ならcontinue)
- [ ] **T203** [US2] T202の後に.claude/hooks/block-git-branch-ops.shのis_read_only_git_branch()関数のPython3フォールバックを改善

### テスト

- [ ] **T204** [P] [US2] tests/hooks/test-git-branch-ops.batsに参照系git branchコマンドの許可テストを追加(git branch, git branch --list, git branch --contains等)
- [ ] **T205** [P] [US2] tests/hooks/test-git-branch-ops.batsに変更系git branchコマンドのブロックテストを追加(git branch new-branch, git branch -d等)
- [ ] **T206** [P] [US2] tests/hooks/test-git-branch-ops.batsにgit checkoutブロックテストを追加(git checkout main等)
- [ ] **T207** [P] [US2] tests/hooks/test-git-branch-ops.batsにgit checkout -- fileの許可テストを追加
- [ ] **T208** [P] [US2] tests/hooks/test-git-branch-ops.batsにgit switch/worktreeブロックテストを追加
- [ ] **T209** [P] [US2] tests/hooks/test-git-branch-ops.batsに複合コマンド(`echo test && git checkout main`)のブロックテストを追加

### 検証

- [ ] **T210** [US2] T201-T203完了後にBatsテスト(T204-T209)を実行し、全テストが成功することを確認
- [ ] **T211** [US2] T210完了後にShellCheckでblock-git-branch-ops.shを解析し、警告を修正

**✅ MVP2チェックポイント**: US2完了後、gitブランチ操作制限が完全に動作し、参照系と変更系の区別が正確

## フェーズ4: ユーザーストーリー3 - ファイル・ディレクトリ操作の範囲制限 (優先度: P2)

**ストーリー**: Claude CodeがWorktree外部のファイルシステムに意図しない変更を加えることを防止し、Worktree内でのみファイル・ディレクトリの作成・削除・変更を許可する。

**価値**: セキュリティとデータ保護の観点から重要。Worktree外部のシステムファイルや他のプロジェクトファイルへの意図しない変更を防ぐ。

### 新規フックスクリプト作成

- [ ] **T301** [US3] .claude/hooks/block-file-ops.shを新規作成し、基本構造(JSON入力解析、ツール名チェック、終了コード)を実装
- [ ] **T302** [US3] T301の後に.claude/hooks/block-file-ops.shにファイル操作コマンド検出ロジックを実装(mkdir, rm, rmdir, touch, cp, mv)
- [ ] **T303** [US3] T302の後に.claude/hooks/block-file-ops.shの各コマンドの引数解析とWorktree境界チェックを実装(common.shのis_within_worktree()を再利用)
- [ ] **T304** [US3] T303の後に.claude/hooks/block-file-ops.shの複合コマンド分割ロジックを実装(既存フックと同じロジック)

### 設定追加

- [ ] **T305** [US3] T304完了後に.claude/settings.jsonのhooks.PreToolUseにblock-file-ops.shを追加

### テスト

- [ ] **T306** [P] [US3] tests/hooks/test-file-ops.batsを新規作成し、Worktree内でのファイル操作許可テストを追加(mkdir ./new-dir等)
- [ ] **T307** [P] [US3] tests/hooks/test-file-ops.batsにWorktree外でのファイル操作ブロックテストを追加(rm /tmp/file.txt, touch ../outside.txt等)
- [ ] **T308** [P] [US3] tests/hooks/test-file-ops.batsに複合コマンド(`pwd && rm /tmp/file.txt`)のブロックテストを追加

### 検証

- [ ] **T309** [US3] T301-T305完了後にBatsテスト(T306-T308)を実行し、全テストが成功することを確認
- [ ] **T310** [US3] T309完了後にShellCheckでblock-file-ops.shを解析し、警告を修正

**✅ MVP3チェックポイント**: US3完了後、ファイル操作制限が動作し、Worktree外への変更が防止される

## フェーズ5: ユーザーストーリー4 - 複合コマンドでの制限適用 (優先度: P2)

**ストーリー**: `&&`や`;`、`|`で連結された複合コマンド内でも、各コマンドに対して制限が適用され、禁止コマンドが含まれている場合は全体がブロックされる。

**価値**: 複合コマンドを使った制限回避を防ぐ。単独コマンドの制限が有効でも、複合コマンドで回避できる場合は機能が無意味になる。

### フックスクリプト改善

- [ ] **T401** [US4] .claude/hooks/block-cd-command.shの複合コマンド分割ロジックをPython shlex.split()ベースに置き換え(ヒアドキュメント、クォート、エスケープの堅牢な処理)
- [ ] **T402** [US4] .claude/hooks/block-git-branch-ops.shの複合コマンド分割ロジックをPython shlex.split()ベースに置き換え
- [ ] **T403** [US4] .claude/hooks/block-file-ops.shの複合コマンド分割ロジックをPython shlex.split()ベースに置き換え

### テスト

- [ ] **T404** [P] [US4] tests/hooks/test-cd-command.batsに複雑な複合コマンド(`echo "test && echo test" && cd /tmp`)のテストを追加
- [ ] **T405** [P] [US4] tests/hooks/test-git-branch-ops.batsに複雑なクォート処理(`git branch --list 'my branch'`)のテストを追加
- [ ] **T406** [P] [US4] tests/hooks/test-file-ops.batsにヒアドキュメントを含む複合コマンドのテストを追加

### 検証

- [ ] **T407** [US4] T401-T403完了後にBatsテスト(T404-T406)を実行し、全テストが成功することを確認
- [ ] **T408** [US4] T407完了後に全フックスクリプトでShellCheckを実行し、警告がないことを確認

**✅ 完全な機能**: US4完了後、すべての要件が満たされ、複合コマンドの制限回避が不可能

## フェーズ6: 統合とポリッシュ

**目的**: すべてのストーリーを統合し、プロダクション準備を整える

### 統合テスト

- [ ] **T501** エンドツーエンドテストを実行(実際のClaude Code環境で全フックをテスト)
- [ ] **T502** T501の後にエッジケース処理を追加(空コマンド、特殊文字、非ASCII文字等)
- [ ] **T503** T502の後に全Batsテストを実行し、100%成功することを確認

### ドキュメント

- [ ] **T504** [P] specs/SPEC-eae13040/spec.mdの成功基準が全て満たされていることを確認
- [ ] **T505** [P] specs/feature/disallow-bash-command/quickstart.mdを最新の実装に合わせて更新
- [ ] **T506** [P] README.mdにフックスクリプトのセクションを追加(概要、インストール、使用方法)

### コード品質

- [ ] **T507** 全フックスクリプト(.claude/hooks/*.sh)に対してShellCheckを実行し、SC2155、SC2269等の警告を全て修正
- [ ] **T508** T507の後にmarkdownlintを実行(`bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore`)し、警告を修正
- [ ] **T509** T508の後にformat:checkを実行(`bun run format:check`)し、フォーマット問題があれば修正

### 最終検証

- [ ] **T510** T501-T509完了後に.github/workflows/lint.ymlのチェック項目をローカルで全実行し、全て成功することを確認
- [ ] **T511** T510完了後にCIパイプラインが成功することを確認(pushしてGitHub Actionsを監視)

## タスク凡例

**優先度**:

- **P1**: 最も重要 - MVP1/MVP2に必要
- **P2**: 重要 - MVP3/完全な機能に必要

**依存関係**:

- **[P]**: 並列実行可能
- **[依存なし]**: 他のタスクの後に実行

**ストーリータグ**:

- **[US1]**: ユーザーストーリー1 - Bashコマンドによる作業ディレクトリ保護
- **[US2]**: ユーザーストーリー2 - Gitブランチ操作の制御
- **[US3]**: ユーザーストーリー3 - ファイル・ディレクトリ操作の範囲制限
- **[US4]**: ユーザーストーリー4 - 複合コマンドでの制限適用

## 依存関係グラフ

```text
フェーズ1(セットアップ) → フェーズ2(US1) ┐
                        → フェーズ3(US2) ├→ フェーズ6(統合)
                        → フェーズ4(US3) ┤
                        → フェーズ5(US4) ┘

注: フェーズ2-5は独立しており、並列実行可能
    ただし、フェーズ1完了後に実行すること
```

## 並列実行例

**フェーズ2(US1)内の並列化**:

- T104-T108(テストタスク)は並列実行可能
- T101-T103は順次実行が必要

**フェーズ間の並列化**:

- フェーズ2、3、4、5は独立しているため、フェーズ1完了後に並列実行可能
- 各フェーズのテストタスクは他フェーズのテストと並列実行可能

## 実装戦略

**MVP1スコープ**: フェーズ1 + フェーズ2(US1)

- cdコマンド制限のみを実装
- 最小限の機能で価値を提供

**MVP2スコープ**: MVP1 + フェーズ3(US2)

- gitブランチ操作制限を追加
- P1ストーリーが全て完了

**MVP3スコープ**: MVP2 + フェーズ4(US3)

- ファイル操作制限を追加
- セキュリティが大幅に向上

**完全な機能**: MVP3 + フェーズ5(US4) + フェーズ6(統合)

- 複合コマンド制限回避防止
- プロダクション準備完了

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## 注記

- 各タスクは1時間から1日で完了可能であるべき
- より大きなタスクはより小さなサブタスクに分割済み
- ファイルパスは正確で、プロジェクト構造と一致
- 各ストーリーは独立してテスト・デプロイ可能
- テストは仕様で要求されているため、全フェーズに含める
