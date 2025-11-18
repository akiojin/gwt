# タスク: bugfixブランチタイプのサポート追加

**入力**: `/specs/SPEC-1defd8fd/` からの設計ドキュメント
**前提条件**: plan.md（必須）、spec.md（ユーザーストーリー用に必須）

**注**: このタスクリストは既存実装の事後ドキュメント化として作成されています。すべてのタスクはコミットca915a0で完了済みです。

**構成**: タスクはユーザーストーリーごとにグループ化され、各ストーリーの独立した実装とテストを可能にします。

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3）
- 説明に正確なファイルパスを含める

## フェーズ1: 基盤（型定義と定数）

**目的**: bugfixブランチタイプの基本構造を確立

### 型定義と定数の追加

- [x] **T001** [P] [基盤] src/cli/ui/types.ts のBranchType型に"bugfix"を追加
- [x] **T002** [P] [基盤] src/cli/ui/types.ts のBranchInfo.branchTypeに"bugfix"を追加
- [x] **T003** [P] [基盤] src/config/constants.ts のBRANCH_TYPESにBUGFIX: "bugfix"を追加
- [x] **T004** [P] [基盤] src/config/constants.ts のBRANCH_PREFIXESにBUGFIX: "bugfix/"を追加

## フェーズ2: ユーザーストーリー1 - 通常のバグ修正用ブランチを作成できる (優先度: P1)

**ストーリー**: 開発者が通常のバグ修正（緊急ではない）を行う際に、適切な名前とアイコンでブランチを作成・識別できる。hotfixは緊急バグ修正用として区別される。

**価値**: バグ修正を緊急度に応じて適切に分類し、視覚的に識別できる。

### ブランチタイプ判定ロジック

- [x] **T101** [US1] src/git.ts のgetBranchType関数にbugfix/プレフィックス判定を追加（line 354）
- [x] **T102** [US1] src/services/git.service.ts のgetBranchType関数にbugfix/プレフィックス判定を追加（line 55）

### UI層の実装

- [x] **T103** [P] [US1] src/cli/ui/utils/branchFormatter.ts のbranchIconsに"bugfix": "🐛"を追加
- [x] **T104** [US1] src/cli/ui/components/screens/BranchCreatorScreen.tsx のBranchType型に"bugfix"を追加（line 10）
- [x] **T105** [US1] src/cli/ui/components/screens/BranchCreatorScreen.tsx のbranchTypeItemsにbugfix選択肢を追加（label: "bugfix", description: "Bug fix"）

### テスト

- [x] **T106** [P] [US1] src/cli/ui/__tests__/utils/branchFormatter.test.ts にbugfixブランチフォーマットのテストを追加
- [x] **T107** [P] [US1] src/cli/ui/__tests__/components/screens/BranchCreatorScreen.test.tsx にbugfix選択肢の表示テストを追加
- [x] **T108** [P] [US1] tests/fixtures/branches.ts にbugfix/null-pointer-exceptionサンプルを追加

**✅ MVP1チェックポイント**: US1完了後、bugfixブランチタイプを作成・表示できる独立した機能を提供

## フェーズ3: ユーザーストーリー2 - bug/プレフィックスもbugfixとして認識される (優先度: P2)

**ストーリー**: 開発者がbug/プレフィックスでブランチを作成した場合も、bugfixブランチタイプとして認識され、同じアイコンで表示される。

**価値**: 後方互換性を確保し、既存プロジェクトのbug/プレフィックスブランチもサポート。

### ブランチタイプ判定の拡張

- [x] **T201** [US2] src/git.ts のgetBranchType関数にbug/プレフィックス判定を追加（"bugfix/" || "bug/"）
- [x] **T202** [US2] src/services/git.service.ts のgetBranchType関数にbug/プレフィックス判定を追加（"bugfix/" || "bug/"）

**✅ MVP2チェックポイント**: US2完了後、bug/プレフィックスもbugfixとして認識される

## フェーズ4: ユーザーストーリー3 - 既存のfeature/hotfix/releaseと同等に扱われる (優先度: P1)

**ストーリー**: bugfixブランチタイプは、既存のブランチタイプ（feature、hotfix、release）と同じ方法で、ソート、フィルタリング、表示において扱われる。

**価値**: 一貫性のあるユーザー体験を提供し、既存機能と統合。

### 統合確認

- [x] **T301** [US3] 既存のソート・フィルタリングロジックがbugfixを正しく扱うことを確認（型システムによる保証）
- [x] **T302** [US3] 既存のブランチリスト表示ロジックがbugfixを正しく表示することを確認（Record<BranchType, string>による保証）

**✅ 完全な機能**: US3完了後、すべての要件が満たされます

## フェーズ5: 検証とポリッシュ

**目的**: すべてのストーリーを統合し、品質を確認

### ビルドとテスト

- [x] **T401** [統合] bun run build でTypeScript型チェックを実行し、エラーなしを確認
- [x] **T402** [統合] bun run test で全テストを実行し、既存テストが全てパスすることを確認
- [x] **T403** [統合] bun run lint でLintチェックを実行し、既存警告のみを確認

### コミットと文書化

- [x] **T404** [統合] 全変更をコミット（ca915a0 feat: bugfixブランチタイプのサポートを追加）
- [x] **T405** [統合] git push でリモートリポジトリにプッシュ

### ドキュメント更新（TODO）

- [ ] **T406** [P] [ドキュメント] README.md の行14（機能概要）にbugfixブランチタイプを追加
- [ ] **T407** [P] [ドキュメント] README.md の行72（使い方）にbugfixブランチタイプを追加
- [ ] **T408** [P] [ドキュメント] README.md の行83（ブランチタイプ選択）にbugfixを追加
- [ ] **T409** [P] [ドキュメント] README.ja.md の行14（機能概要）にbugfixブランチタイプを追加
- [ ] **T410** [P] [ドキュメント] README.ja.md の行72（使い方）にbugfixブランチタイプを追加
- [ ] **T411** [P] [ドキュメント] README.ja.md の行83（ブランチタイプ選択）にbugfixを追加

### Spec Kit対応（TODO）

- [x] **T412** [ドキュメント] /speckit.specify でspec.mdを作成
- [x] **T413** [ドキュメント] /speckit.plan でplan.mdを作成
- [x] **T414** [ドキュメント] /speckit.tasks でtasks.md（このファイル）を作成
- [ ] **T415** [ドキュメント] /speckit.analyze で品質分析を実行

## タスク凡例

**優先度**:

- **P1**: 最も重要 - MVP1に必要
- **P2**: 重要 - MVP2に必要

**依存関係**:

- **[P]**: 並列実行可能
- **[依存なし]**: 他のタスクの後に実行

**ストーリータグ**:

- **[US1]**: ユーザーストーリー1 - 通常のバグ修正用ブランチを作成できる
- **[US2]**: ユーザーストーリー2 - bug/プレフィックスもbugfixとして認識される
- **[US3]**: ユーザーストーリー3 - 既存のfeature/hotfix/releaseと同等に扱われる
- **[基盤]**: すべてのストーリーで共有される基盤
- **[統合]**: 複数ストーリーにまたがる統合タスク
- **[ドキュメント]**: ドキュメント専用タスク

## 実装戦略

### MVPインクリメント

1. **MVP1 (US1完了時)**: bugfixブランチタイプの基本機能
   - 型定義、定数、判定ロジック、UI、アイコン、テスト
   - この時点で独立した価値を提供

2. **MVP2 (US2完了時)**: bug/プレフィックスのサポート
   - 後方互換性の追加
   - 既存プロジェクトでの使用が可能

3. **完全機能 (US3完了時)**: 既存機能との完全統合
   - 一貫性のあるUX
   - すべての要件を満たす

### 並列実行の機会

**フェーズ1（基盤）**: T001-T004は並列実行可能（異なるファイル）

**フェーズ2（US1）**:

- T103（branchFormatter.ts）は並列実行可能
- T106-T108（テスト）は並列実行可能

**フェーズ5（ドキュメント）**: T406-T411は並列実行可能（異なるファイル）

## 進捗追跡

- **完了したタスク**: 15/21 (71%)
  - フェーズ1: 4/4 (100%)
  - フェーズ2: 8/8 (100%)
  - フェーズ3: 2/2 (100%)
  - フェーズ4: 2/2 (100%)
  - フェーズ5: 5/11 (45%)

- **残りのタスク**: 6タスク（すべてドキュメント更新）
  - T406-T411: README更新
  - T415: /speckit.analyze実行

## 注記

- 実装は1コミット（ca915a0）で一括完了
- 型システムによる静的検証により、既存機能との互換性を保証
- すべての既存テストがパス（後方互換性維持）
- 残作業: ドキュメント更新とSpec Kit分析のみ
