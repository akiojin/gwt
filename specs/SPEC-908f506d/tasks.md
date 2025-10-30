# タスク: ブランチ作成・選択機能の改善

**入力**: `/specs/SPEC-908f506d/` からの設計ドキュメント
**前提条件**: plan.md（必須）、spec.md（ユーザーストーリー用に必須）、research.md、data-model.md、quickstart.md

**テスト**: CLAUDE.mdに「Spec Kitを用いたSDD/TDDの絶対遵守」とあるため、すべてのフェーズでテストタスクを含めます。

**構成**: タスクはユーザーストーリーごとにグループ化され、各ストーリーの独立した実装とテストを可能にします。

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3）
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

## フェーズ1: セットアップ（共有インフラストラクチャ）

**目的**: 開発環境の準備と設計ドキュメントの確認

### セットアップタスク

- [x] T001 設計ドキュメントを確認（spec.md, plan.md, data-model.md, quickstart.md）
- [x] T002 開発環境をセットアップ（`bun install`、ビルド確認）
- [x] T003 [P] 既存のテストスイートを実行して正常動作を確認（`bun test`） - 327 pass, 2 fail (既存の問題)

## フェーズ2: 基盤タスク（すべてのストーリーの前提条件）

**目的**: すべてのユーザーストーリーで共有される型定義と基礎ロジック

### 型定義

- [x] T101 [P] `src/ui/types.ts` に `BranchAction` 型を追加（'use-existing' | 'create-new'）
- [x] T102 [P] `src/ui/types.ts` の `ScreenType` に `'branch-action-selector'` を追加

### Git操作（既存）

- [x] T103 `src/git.ts` の `getCurrentBranch()` 関数をエクスポート

## フェーズ3: ユーザーストーリー1 - カレントブランチでの直接作業 (優先度: P1)

**ストーリー**: 開発者がブランチ一覧から、現在作業中のブランチ（カレントブランチ）を選択した場合、システムは新しいWorktreeを作成せず、ルートディレクトリでそのままAIツールを起動する。

**価値**: 既存の不具合修正。開発者がカレントブランチで作業を続ける際の無駄なWorktree作成を防ぎ、即座に作業を継続できる。

### テスト（TDD: Red）

- [ ] T201 [P] [US1] `src/services/__tests__/WorktreeOrchestrator.test.ts` にカレントブランチ判定のテストケースを追加
  - カレントブランチを選択した場合、リポジトリルートを返すテスト
  - カレントブランチ以外を選択した場合、Worktreeパスを返すテスト
  - `getCurrentBranch()` が null の場合のフォールバックテスト

### 実装（TDD: Green）

- [ ] T202 [US1] T201の後に `src/services/WorktreeOrchestrator.ts` の `ensureWorktree()` にカレントブランチ判定を追加
  - `getCurrentBranch()` をインポート
  - ブランチ名がカレントブランチと一致する場合、`repoRoot` を返す
  - 一致しない場合、既存のロジックを実行
- [ ] T203 [US1] T202の後にテストを実行して Green を確認（`bun test src/services/__tests__/WorktreeOrchestrator.test.ts`）

### リファクタリング（TDD: Refactor）

- [ ] T204 [US1] T203の後にロギングを追加（カレントブランチ選択時のログ）
- [ ] T205 [US1] T204の後にテストを再実行して動作確認

**✅ MVP1チェックポイント**: US1完了後、カレントブランチ選択が正常に動作するMVP

## フェーズ4: ユーザーストーリー2 - 既存ブランチでの作業継続 (優先度: P2)

**ストーリー**: 開発者がブランチ一覧から任意のブランチを選択した後、「既存ブランチで続行」を選択すると、そのブランチのWorktreeが作成または再利用され、AIツールが起動される。

**価値**: 新機能の基本動作。ユーザーが既存のブランチで作業を開始する際の標準フローを明示的に選択できる。

### UI層: BranchActionSelectorScreen作成

#### テスト（TDD: Red）

- [ ] T301 [P] [US2] `src/ui/__tests__/components/screens/BranchActionSelectorScreen.test.tsx` を作成
  - 2つの選択肢が表示されるテスト
  - 「既存ブランチで続行」選択時に `onUseExisting` が呼ばれるテスト
  - 「新規ブランチを作成」選択時に `onCreateNew` が呼ばれるテスト
  - 'q' キー入力時に `onBack` が呼ばれるテスト

#### 実装（TDD: Green）

- [ ] T302 [US2] T301の後に `src/ui/components/screens/BranchActionSelectorScreen.tsx` を作成
  - Props定義（`BranchActionSelectorScreenProps`）
  - 2択のアイテム配列作成
  - `Select` コンポーネントの使用
  - ブランチ情報の表示
  - キーボードハンドラ（'q'キー）
- [ ] T303 [US2] T302の後にテストを実行して Green を確認

### App.tsx: 画面遷移フロー改修

#### テスト（TDD: Red）

- [ ] T304 [US2] `src/ui/__tests__/components/App.test.tsx` に画面遷移テストを追加
  - カレントブランチ選択時に直接 `ai-tool-selector` へ遷移するテスト
  - 他のブランチ選択時に `branch-action-selector` へ遷移するテスト

#### 実装（TDD: Green）

- [ ] T305 [US2] T304の後に `src/ui/components/App.tsx` に状態管理を追加
  - `baseBranchForCreation` の useState を追加
- [ ] T306 [US2] T305の後に `src/ui/components/App.tsx` の `handleSelect` を改修
  - `getCurrentBranch()` を呼び出してカレントブランチ判定
  - カレントブランチの場合、直接 `ai-tool-selector` へ遷移
  - カレントブランチでない場合、`branch-action-selector` へ遷移
- [ ] T307 [US2] T306の後に新しいハンドラーを追加
  - `handleBranchActionUseExisting`: `ai-tool-selector` へ遷移
  - `handleBranchActionCreate`: `baseBranchForCreation` を設定して `branch-creator` へ遷移
- [ ] T308 [US2] T307の後に `renderScreen()` に `branch-action-selector` ケースを追加
  - `BranchActionSelectorScreen` コンポーネントをレンダリング
  - Props を適切に渡す
- [ ] T309 [US2] T308の後にテストを実行して Green を確認

### 統合テスト

- [ ] T310 [US2] `src/ui/__tests__/integration/branch-selection-flow.test.ts` を作成
  - カレントブランチフロー: BranchListScreen → AIToolSelectorScreen
  - 他のブランチ + 既存使用フロー: BranchListScreen → BranchActionSelectorScreen → AIToolSelectorScreen

**✅ MVP2チェックポイント**: US2完了後、アクション選択機能が使えるMVP

## フェーズ5: ユーザーストーリー3 - 選択ブランチをベースに新規ブランチ作成 (優先度: P3)

**ストーリー**: 開発者がブランチ一覧から任意のブランチを選択した後、「新規ブランチを作成」を選択すると、選択したブランチをベースとして新しいブランチを作成できる。

**価値**: 開発フローの柔軟性を高める追加機能。選択したブランチを基に新規ブランチを作成可能。

### BranchCreatorScreen: ベースブランチパラメータ追加

#### テスト（TDD: Red）

- [ ] T401 [P] [US3] `src/ui/__tests__/components/screens/BranchCreatorScreen.test.tsx` にベースブランチテストを追加
  - `baseBranch` が指定されている場合、それを使用するテスト
  - `baseBranch` が未指定の場合、`resolveBaseBranch()` を使用するテスト

#### 実装（TDD: Green）

- [ ] T402 [US3] T401の後に `src/ui/components/screens/BranchCreatorScreen.tsx` の Props に `baseBranch?: string` を追加
- [ ] T403 [US3] T402の後にブランチ作成ロジックを改修
  - `baseBranch` が指定されていればそれを使用
  - 未指定なら `resolveBaseBranch()` を使用（既存の挙動）
- [ ] T404 [US3] T403の後に `src/ui/components/App.tsx` の `renderScreen()` で `branch-creator` ケースを更新
  - `baseBranch={baseBranchForCreation}` を渡す
- [ ] T405 [US3] T404の後にテストを実行して Green を確認

### 統合テスト

- [ ] T406 [US3] `src/ui/__tests__/integration/branch-selection-flow.test.ts` に新規作成フローテストを追加
  - ブランチ選択 → アクション選択（新規作成） → ブランチ作成画面
  - 選択したブランチがベースブランチとして設定されていることを確認

**✅ 完全な機能**: US3完了後、すべての要件が満たされます

## フェーズ6: 統合とポリッシュ

**目的**: すべてのストーリーを統合し、プロダクション準備を整える

### エンドツーエンドテスト

- [ ] T501 [統合] `tests/integration/branch-selection-flow.test.ts` ですべてのフローを確認
  - フロー1: カレントブランチ選択 → AIツール起動
  - フロー2: 他のブランチ選択 → 既存使用 → AIツール起動
  - フロー3: 他のブランチ選択 → 新規作成 → ブランチ作成 → AIツール起動
- [ ] T502 [統合] T501の後にエッジケースのテストを追加
  - `getCurrentBranch()` が null の場合
  - リモートブランチをベースに新規作成する場合
  - detached HEAD状態の処理

### 品質チェック

- [ ] T503 [統合] T502の後に型チェックを実行（`bun run type-check`）、エラーがあれば修正
- [ ] T504 [統合] T503の後にLintチェックを実行（`bun run lint`）、エラーがあれば修正
- [ ] T505 [統合] T504の後にフォーマットチェックを実行（`bun run format:check`）、エラーがあれば修正
- [ ] T506 [統合] T505の後にMarkdownlintチェックを実行（`bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore`）、エラーがあれば修正
- [ ] T507 [統合] T506の後にすべてのテストを実行（`bun test`）、失敗があれば修正
- [ ] T508 [統合] T507の後にカバレッジチェックを実行（`bun run test:coverage`）、カバレッジが低い箇所があれば追加テスト

### ビルドと動作確認

- [ ] T509 [統合] T508の後にビルドを実行（`bun run build`）、エラーがあれば修正
- [ ] T510 [統合] T509の後に実際のリポジトリで動作確認（`bun run start`）
  - カレントブランチを選択して動作確認
  - 他のブランチを選択してアクション選択画面の確認
  - 新規ブランチ作成の動作確認

### ドキュメント

- [ ] T511 [P] [ドキュメント] `README.md` または `README.ja.md` の機能説明を更新（必要に応じて）
- [ ] T512 [P] [ドキュメント] `CHANGELOG.md` に変更内容を追加（セマンティックリリースで自動生成される場合はスキップ）

### コミットとプッシュ

- [ ] T513 [統合] すべての変更をコミット（日本語のコミットメッセージ、100文字以内）
- [ ] T514 [統合] T513の後にリモートにプッシュ

## タスク凡例

**優先度**:

- **P1**: 最も重要 - MVP1に必要（カレントブランチでの直接作業）
- **P2**: 重要 - MVP2に必要（既存ブランチでの作業継続）
- **P3**: 補完的 - 完全な機能に必要（選択ブランチをベースに新規作成）

**依存関係**:

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **Txxxの後に**: 指定されたタスクの完了後に実行

**ストーリータグ**:

- **[US1]**: ユーザーストーリー1 - カレントブランチでの直接作業
- **[US2]**: ユーザーストーリー2 - 既存ブランチでの作業継続
- **[US3]**: ユーザーストーリー3 - 選択ブランチをベースに新規作成
- **[統合]**: 複数ストーリーにまたがる
- **[ドキュメント]**: ドキュメント専用

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## タスク統計

**総タスク数**: 51タスク

**フェーズ別タスク数**:

- Phase 1（Setup）: 3タスク
- Phase 2（Foundational）: 3タスク
- Phase 3（US1）: 5タスク
- Phase 4（US2）: 10タスク
- Phase 5（US3）: 6タスク
- Phase 6（統合とポリッシュ）: 14タスク

**ストーリー別タスク数**:

- US1: 5タスク（テスト3 + 実装2）
- US2: 10タスク（テスト3 + 実装7）
- US3: 6タスク（テスト2 + 実装4）

**並列実行可能なタスク**: 14タスク（[P]マーク付き）

## 独立したテスト基準

### US1（カレントブランチでの直接作業）

**テスト方法**:

1. カレントブランチを選択
2. Worktreeが作成されないことを確認
3. ルートディレクトリでAIツールが起動することを確認

**合格基準**: カレントブランチ選択時、1秒以内にAIツールが起動される

### US2（既存ブランチでの作業継続）

**テスト方法**:

1. カレントブランチ以外を選択
2. アクション選択画面が表示されることを確認
3. 「既存ブランチで続行」を選択
4. Worktreeが作成/再利用され、AIツールが起動することを確認

**合格基準**: アクション選択画面が1秒以内に表示され、「既存ブランチで続行」選択後3秒以内にAIツールが起動される

### US3（選択ブランチをベースに新規作成）

**テスト方法**:

1. 任意のブランチを選択
2. アクション選択画面で「新規ブランチを作成」を選択
3. ブランチ作成画面が表示され、選択したブランチがベースとして設定されていることを確認
4. 新規ブランチを作成してWorktreeが作成され、AIツールが起動することを確認

**合格基準**: ブランチ作成画面が1秒以内に表示され、選択したブランチがベースとして正しく使用される

## MVP推奨スコープ

**MVP1**: User Story 1（P1）のみ

- カレントブランチ選択時のWorktree作成スキップ
- 不具合修正として最優先で実装
- 独立してデリバリー可能

**MVP2**: User Story 1 + 2（P1 + P2）

- MVP1 + アクション選択機能
- 標準的な開発フローをカバー
- 実用的な機能として十分

**完全版**: User Story 1 + 2 + 3（P1 + P2 + P3）

- すべての要件を満たす
- 最大限の柔軟性を提供

## 実装戦略

### TDD（テスト駆動開発）サイクル

各機能の実装は以下のサイクルで進めます：

1. **Red**: テストを先に書く（失敗することを確認）
2. **Green**: 最小限の実装でテストをパスさせる
3. **Refactor**: コードをリファクタリング（テストは依然としてパス）

### 段階的デリバリー

1. **Phase 3完了**: MVP1をデリバリー可能（カレントブランチ対応）
2. **Phase 4完了**: MVP2をデリバリー可能（アクション選択対応）
3. **Phase 5完了**: 完全版をデリバリー可能（新規作成対応）
4. **Phase 6完了**: プロダクション準備完了

### 並列実行の推奨

以下のタスクは並列実行可能です（[P]マーク付き）：

- Phase 2: T101、T102、T103
- Phase 3: T201（テスト作成）
- Phase 4: T301（テスト作成）
- Phase 5: T401（テスト作成）
- Phase 6: T511、T512（ドキュメント）

## 注記

- 各タスクは1時間から1日で完了可能であるべき
- より大きなタスクはより小さなサブタスクに分割
- ファイルパスは正確で、プロジェクト構造と一致させる
- TDDサイクルに従い、テストを先に書いてから実装
- 各ストーリーは独立してテスト・デプロイ可能
- CLAUDE.mdの開発指針に従い、シンプルさを追求
- コミットメッセージは日本語で、100文字以内に収める
