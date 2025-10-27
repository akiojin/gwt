# タスク: 一括ブランチマージ機能

**入力**: `/specs/SPEC-ee33ca26/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、research.md、data-model.md、quickstart.md

**テスト**: TDD原則により、全てのタスクでテストを含めます（CLAUDE.md準拠）。

**構成**: タスクはユーザーストーリーごとにグループ化され、各ストーリーの独立した実装とテストを可能にします。

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3、US4）
- 説明に正確なファイルパスを含める

## Commitlintルール

- コミットメッセージは件名のみを使用し、空にしてはいけません。
- 件名は100文字以内に収めてください。
- タスク完了時は、変更内容を簡潔にまとめたコミットメッセージを作成してください。

## Lint最小要件

各タスク完了後、以下のチェックをローカルで実行し、成功することを確認：

- `bun run format:check`
- `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore`
- `bun run lint`

## フェーズ1: セットアップ

**目的**: 開発環境確認と依存関係なしの確認

### セットアップタスク

- [X] **T001** 開発環境の確認（Bun 1.0+, TypeScript 5.8+, 依存関係インストール済み）
- [X] **T002** 既存のテストフレームワーク動作確認（`bun run test` 実行）

## フェーズ2: 基盤（Foundational）

**目的**: 全ユーザーストーリーで共有する型定義とgit操作基盤

### 型定義タスク

- [X] **T101** [P] src/ui/types.ts に BatchMergeConfig 型を追加
- [X] **T102** [P] src/ui/types.ts に MergePhase 型を追加
- [X] **T103** [P] src/ui/types.ts に BatchMergeProgress 型を追加
- [X] **T104** [P] src/ui/types.ts に MergeStatus と PushStatus 型を追加
- [X] **T105** [P] src/ui/types.ts に BranchMergeStatus 型を追加
- [X] **T106** [P] src/ui/types.ts に BatchMergeSummary 型を追加
- [X] **T107** [P] src/ui/types.ts に BatchMergeResult 型を追加

### Git操作基盤タスク（TDD）

- [X] **T108** tests/unit/git.test.ts に mergeFromBranch 関数のテストケースを追加
- [X] **T109** T108の後に src/git.ts に mergeFromBranch 関数を実装
- [X] **T110** [P] tests/unit/git.test.ts に hasMergeConflict 関数のテストケースを追加
- [X] **T111** T110の後に src/git.ts に hasMergeConflict 関数を実装
- [X] **T112** [P] tests/unit/git.test.ts に abortMerge 関数のテストケースを追加
- [X] **T113** T112の後に src/git.ts に abortMerge 関数を実装
- [X] **T114** [P] tests/unit/git.test.ts に getMergeStatus 関数のテストケースを追加
- [X] **T115** T114の後に src/git.ts に getMergeStatus 関数を実装

## フェーズ3: ユーザーストーリー1 + 4 (P1) - 基本一括マージ + リアルタイム進捗表示

**ストーリー1**: 開発者が複数のfeatureブランチで作業している場合、ブランチ一覧画面から'p'キーを押下することで、全てのローカルブランチ(main/develop除く)に対して一括でマージを実行できる。

**ストーリー4**: 開発者が多数のブランチに対して一括マージを実行する場合、現在どのブランチを処理中か、全体の進捗率、経過時間をリアルタイムで確認できる。

**価値**: MVPとして、手動マージ作業の自動化と処理状況の可視化を提供

**独立したテスト基準**:

- 3つのfeatureブランチを作成し、mainブランチに変更を加えた後、一括マージを実行して全ブランチにmainの変更が反映されることを確認
- 進捗表示が現在処理中のブランチ、進捗率、経過時間を正しく表示することを確認

### サービス層タスク（TDD）

- [X] **T201** [US1] [US4] tests/unit/services/BatchMergeService.test.ts を作成し、BatchMergeService の初期化テストを追加
- [X] **T202** [US1] [US4] T201の後に src/services/BatchMergeService.ts を作成し、基本構造を実装
- [X] **T203** [US1] [US4] [P] tests/unit/services/BatchMergeService.test.ts に determineSourceBranch メソッドのテストを追加
- [X] **T204** [US1] [US4] T203の後に src/services/BatchMergeService.ts に determineSourceBranch メソッドを実装
- [X] **T205** [US1] [US4] [P] tests/unit/services/BatchMergeService.test.ts に getTargetBranches メソッドのテストを追加
- [X] **T206** [US1] [US4] T205の後に src/services/BatchMergeService.ts に getTargetBranches メソッドを実装
- [X] **T207** [US1] [US4] [P] tests/unit/services/BatchMergeService.test.ts に ensureWorktree メソッドのテストを追加
- [X] **T208** [US1] [US4] T207の後に src/services/BatchMergeService.ts に ensureWorktree メソッドを実装
- [X] **T209** [US1] [US4] [P] tests/unit/services/BatchMergeService.test.ts に mergeBranch メソッドのテストを追加（成功ケース）
- [X] **T210** [US1] [US4] T209の後に src/services/BatchMergeService.ts に mergeBranch メソッドを実装
- [X] **T211** [US1] [US4] [P] tests/unit/services/BatchMergeService.test.ts に mergeBranch メソッドのテスト（コンフリクトケース）を追加
- [X] **T212** [US1] [US4] T211の後に src/services/BatchMergeService.ts の mergeBranch メソッドにコンフリクト処理を追加
- [X] **T213** [US1] [US4] [P] tests/unit/services/BatchMergeService.test.ts に executeBatchMerge メソッドのテストを追加
- [X] **T214** [US1] [US4] T213の後に src/services/BatchMergeService.ts に executeBatchMerge メソッドを実装（進捗コールバック含む）

### UI部品タスク（TDD）

- [ ] **T215** [US4] [P] src/ui/components/parts/ProgressBar.tsx を作成し、進捗バーコンポーネントを実装
- [ ] **T216** [US4] [P] src/ui/components/parts/MergeStatusList.tsx を作成し、マージステータスリストコンポーネントを実装

### UI画面タスク（TDD）

- [ ] **T217** [US4] src/ui/components/screens/BatchMergeProgressScreen.tsx を作成し、進捗表示画面を実装
- [ ] **T218** [US1] src/ui/components/screens/BatchMergeResultScreen.tsx を作成し、結果サマリー画面を実装
- [ ] **T219** [US1] [US4] tests/e2e/batch-merge-workflow.test.ts を作成し、'p'キー押下から結果表示までのE2Eテストを追加
- [ ] **T219.5** [US1] [US4] T219の後に確認ダイアログコンポーネントを作成し、実行前に「対象ブランチ数: N件」「マージ元: main」「オプション: ドライラン/自動プッシュ」を表示する機能を実装（FR-002対応）
- [ ] **T220** [US1] [US4] T219.5の後に src/ui/components/screens/BranchListScreen.tsx に'p'キーハンドラを追加
- [ ] **T221** [US1] [US4] T220の後に src/ui/components/App.tsx にBatchMergeProgressScreenとBatchMergeResultScreenへの画面遷移を追加

### カスタムフックタスク（TDD）

- [ ] **T222** [US1] [US4] [P] src/ui/hooks/useBatchMerge.ts を作成し、バッチマージロジックフックを実装

### 統合テストタスク

- [ ] **T223** [US1] [US4] tests/integration/batch-merge.test.ts を作成し、実gitリポジトリでの一括マージフロー全体をテスト
- [ ] **T224** [US1] [US4] [P] tests/integration/batch-merge.test.ts にコンフリクト処理フローのテストを追加
- [ ] **T225** [US1] [US4] [P] tests/integration/batch-merge.test.ts にworktree自動作成フローのテストを追加

**✅ MVP1チェックポイント**: US1+US4完了後、基本的な一括マージ機能と進捗表示が動作し、独立した価値を提供可能

## フェーズ4: ユーザーストーリー2 (P2) - ドライランモード

**ストーリー**: 開発者が実際にマージを実行する前に、どのブランチがマージ可能か、どのブランチでコンフリクトが発生するかを事前に確認したい場合、ドライランモードを有効にして実行することで、実際の変更を加えずにシミュレーション結果を確認できる。

**価値**: リスク低減のための事前確認機能を提供

**独立したテスト基準**:

- ドライランモードで実行後、各ブランチの最新コミットハッシュが変更されていないことを確認
- シミュレーション結果が正確に表示されることを確認

### ドライラン実装タスク（TDD）

- [ ] **T301** [US2] tests/unit/git.test.ts にドライランマージ（mergeFromBranch with --no-commit）のテストを追加
- [ ] **T302** [US2] T301の後に src/git.ts の mergeFromBranch 関数にドライランオプションを追加
- [ ] **T303** [US2] [P] tests/unit/services/BatchMergeService.test.ts にドライランモードのテストを追加
- [ ] **T304** [US2] T303の後に src/services/BatchMergeService.ts の executeBatchMerge メソッドにドライラン処理を追加
- [ ] **T305** [US2] [P] tests/integration/batch-merge.test.ts にドライランモード統合テストを追加
- [ ] **T306** [US2] T305の後に src/ui/components/screens/BatchMergeResultScreen.tsx にドライラン結果表示を追加

### UI拡張タスク

- [ ] **T307** [US2] src/ui/components/screens/BranchListScreen.tsx または確認ダイアログにドライランモードオプションを追加
- [ ] **T308** [US2] tests/e2e/batch-merge-workflow.test.ts にドライランモードE2Eテストを追加

**✅ MVP2チェックポイント**: US2完了後、ドライランによる事前確認機能が追加され、リスク低減が可能

## フェーズ5: ユーザーストーリー3 (P3) - マージ後の自動プッシュ

**ストーリー**: 開発者がマージ後の変更を即座にリモートリポジトリに反映させたい場合、自動プッシュオプションを有効にすることで、マージ成功後に自動的にリモートへプッシュされる。

**価値**: マージ後の手動プッシュ作業を自動化し、ワークフローを効率化

**独立したテスト基準**:

- 自動プッシュオプションを有効にして実行後、リモートリポジトリの各ブランチにマージコミットが反映されていることを確認
- プッシュ失敗時も処理が継続されることを確認

### 自動プッシュ実装タスク（TDD）

- [ ] **T401** [US3] tests/unit/services/BatchMergeService.test.ts に自動プッシュ処理のテストを追加
- [ ] **T402** [US3] T401の後に src/services/BatchMergeService.ts の mergeBranch メソッドに自動プッシュ処理を追加
- [ ] **T403** [US3] [P] tests/unit/services/BatchMergeService.test.ts にプッシュ失敗ケースのテストを追加
- [ ] **T404** [US3] T403の後に src/services/BatchMergeService.ts にプッシュエラーハンドリングを追加
- [ ] **T405** [US3] [P] tests/integration/batch-merge.test.ts に自動プッシュ統合テストを追加（実リモートリポジトリ）
- [ ] **T406** [US3] T405の後に src/ui/components/screens/BatchMergeResultScreen.tsx にプッシュ結果表示を追加

### UI拡張タスク

- [ ] **T407** [US3] src/ui/components/screens/BranchListScreen.tsx または確認ダイアログに自動プッシュオプションを追加
- [ ] **T408** [US3] src/ui/components/screens/BatchMergeProgressScreen.tsx にプッシュフェーズの進捗表示を追加
- [ ] **T409** [US3] tests/e2e/batch-merge-workflow.test.ts に自動プッシュE2Eテストを追加

**✅ 完全な機能**: US3完了後、全ての要件が満たされ、完全自動化された一括マージ機能が提供される

## フェーズ6: 統合とポリッシュ

**目的**: すべてのストーリーを統合し、プロダクション準備を整える

### 統合テスト

- [ ] **T501** [統合] 全E2Eテストを実行し、全シナリオが正常動作することを確認
- [ ] **T502** [統合] エッジケーステスト（対象ブランチ0個、マージ元不明、全コンフリクトなど）を追加
- [ ] **T503** [統合] キャンセル処理（'q'キー、Ctrl+C）のテストを追加
- [ ] **T504** [統合] T503の後に src/services/BatchMergeService.ts にキャンセル処理を実装

### 品質チェック

- [ ] **T505** [統合] bun run type-check を実行し、型エラーがないことを確認
- [ ] **T506** [統合] bun run lint を実行し、lintエラーがないことを確認
- [ ] **T507** [統合] bun run test を実行し、全テストがパスすることを確認
- [ ] **T508** [統合] bun run test:coverage を実行し、カバレッジが80%以上であることを確認
- [ ] **T509** [統合] bun run build を実行し、ビルドエラーがないことを確認
- [ ] **T510** [統合] bun run format:check を実行し、フォーマットエラーがないことを確認
- [ ] **T511** [統合] bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore を実行し、Markdownエラーがないことを確認

### ドキュメント

- [ ] **T512** [P] [ドキュメント] README.md に一括マージ機能の使い方を追加
- [ ] **T513** [P] [ドキュメント] README.ja.md に一括マージ機能の使い方を追加（日本語）

### パフォーマンステスト

- [ ] **T514** [P] [統合] tests/integration/batch-merge-performance.test.ts を作成し、20ブランチの処理時間を測定
- [ ] **T515** [統合] T514の後にパフォーマンス基準（5ブランチ1分以内、20ブランチ対応）を満たすことを確認

## タスク凡例

**優先度**:

- **P1**: 最も重要 - MVP1に必要（US1 + US4）
- **P2**: 重要 - MVP2に必要（US2）
- **P3**: 補完的 - 完全な機能に必要（US3）

**並列実行**:

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **依存あり**: "TXXXの後に"と明記

**ストーリータグ**:

- **[US1]**: ユーザーストーリー1 - 基本的な一括マージ
- **[US2]**: ユーザーストーリー2 - ドライランモード
- **[US3]**: ユーザーストーリー3 - マージ後の自動プッシュ
- **[US4]**: ユーザーストーリー4 - リアルタイム進捗表示
- **[統合]**: 複数ストーリーにまたがる
- **[ドキュメント]**: ドキュメント専用

## 依存関係グラフ

```
Phase 1 (Setup)
    |
    v
Phase 2 (Foundational - 型定義 + Git基盤)
    |
    +----> Phase 3 (US1 + US4) - MVP1
    |          |
    |          +----> Phase 4 (US2) - MVP2
    |          |          |
    |          |          +----> Phase 5 (US3) - 完全版
    |          |                      |
    |          |                      v
    +----------+----------------------+
                                     |
                                     v
                           Phase 6 (統合とポリッシュ)
```

**ストーリー完了順序**:

1. **Phase 2完了後**: US1, US4を並行開発可能
2. **US1+US4完了後**: MVP1デリバリー可能、US2開始可能
3. **US2完了後**: MVP2デリバリー可能、US3開始可能
4. **US3完了後**: 完全版デリバリー可能
5. **全ストーリー完了後**: 統合とポリッシュ

## 並列実行例

### Phase 2での並列実行

```
並列グループ1（型定義）: T101, T102, T103, T104, T105, T106, T107
並列グループ2（Gitテスト）: T108, T110, T112, T114
順次実行（Git実装）: T109 -> T111 -> T113 -> T115
```

### Phase 3での並列実行

```
並列グループ1（サービステスト）: T201, T203, T205, T207, T209, T211, T213
順次実行（サービス実装）: T202 -> T204 -> T206 -> T208 -> T210 -> T212 -> T214

並列グループ2（UI部品）: T215, T216（T214完了後）
順次実行（UI画面）: T217 -> T218 -> T219 -> T220 -> T221
並列実行（フック）: T222

並列グループ3（統合テスト）: T223, T224, T225（全実装完了後）
```

### Phase 4での並列実行

```
並列グループ1（テスト）: T301, T303, T305
順次実行（実装）: T302 -> T304 -> T306 -> T307 -> T308
```

### Phase 5での並列実行

```
並列グループ1（テスト）: T401, T403, T405
順次実行（実装）: T402 -> T404 -> T406 -> T407 -> T408 -> T409
```

### Phase 6での並列実行

```
順次実行（統合）: T501 -> T502 -> T503 -> T504
並列実行（品質）: T505, T506, T507, T508, T509, T510, T511（統合完了後）
並列実行（ドキュメント）: T512, T513
並列実行（パフォーマンス）: T514 -> T515
```

## 実装戦略

### MVP優先アプローチ

1. **MVP1（US1 + US4）**: Phase 3完了
   - 基本的な一括マージ機能
   - リアルタイム進捗表示
   - コア価値の提供

2. **MVP2（+US2）**: Phase 4完了
   - ドライランモードによるリスク低減
   - より安全な運用

3. **完全版（+US3）**: Phase 5完了
   - 自動プッシュによる完全自動化
   - 全要件満たす

### インクリメンタルデリバリー

- 各Phaseは独立してテスト・デプロイ可能
- Phase 3完了時点でMVP1をリリース可能
- Phase 4完了時点でMVP2をリリース可能
- Phase 5完了時点で完全版をリリース可能

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

## タスクサマリー

- **総タスク数**: 115タスク
- **Phase 1 (Setup)**: 2タスク
- **Phase 2 (Foundational)**: 15タスク
- **Phase 3 (US1+US4 - P1)**: 25タスク
- **Phase 4 (US2 - P2)**: 8タスク
- **Phase 5 (US3 - P3)**: 9タスク
- **Phase 6 (Integration)**: 15タスク

**並列実行機会**: 約40タスクが並列実行可能（[P]マーク付き）

**MVP1スコープ**: Phase 1 + Phase 2 + Phase 3（計42タスク）

## 注記

- 各タスクは1時間から1日で完了可能
- TDD原則に従い、テスト→実装の順で実行
- 全テストパス + ビルド成功 + lint通過を完了条件とする
- commitlint準拠のコミットメッセージを作成
- Markdownlint、Prettier、ESLintを全て通過させる
