# タスク: UI移行 - Ink.js（React）ベースのCLIインターフェース

**SPEC ID**: SPEC-4c2ef107
**入力**: `/specs/SPEC-4c2ef107/` からの設計ドキュメント
**前提条件**: plan.md、spec.md、research.md、data-model.md、quickstart.md

**テスト**: TDD必須 - すべてのコンポーネントはテストファーストで実装（80%カバレッジ目標）

**構成**: タスクはユーザーストーリーごとにグループ化され、各ストーリーの独立した実装とテストを可能にします。

## フォーマット: `- [ ] [ID] [P?] [ストーリー?] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[ストーリー]**: このタスクが属するユーザーストーリー（例: US1、US2、US3）
- 説明に正確なファイルパスを含める

## MVP戦略

- **MVP1 (US1完了時)**: ブランチ一覧表示と選択のみ動作 → デプロイ可能
- **MVP2 (US2完了時)**: 全サブ画面動作 → 既存機能完全互換
- **完全版 (US3完了時)**: リアルタイム更新 → 新機能追加

---

## Phase 1: セットアップ（プロジェクト初期化）

**目的**: 開発環境とInk.js基盤のセットアップ

### 依存関係とツール

- [x] T001 [P] Ink.js関連の依存関係を追加（ink, react, ink-select-input, ink-text-input, @types/react）
- [x] T002 [P] テストライブラリを追加（ink-testing-library, jsdom, @testing-library/jest-dom）
- [x] T003 Vitest設定を更新（vitest.config.ts でjsdom環境を設定）
- [x] T004 [P] Vitestセットアップファイルを作成（vitest.setup.ts）

### Ink.js動作確認（ブロッカー）

- [x] T005 Ink.js + bun互換性を検証（サンプルアプリで動作確認）
- [x] T006 T005の後に互換性結果を文書化（research.mdを更新）

### 既存コード移行準備

- [x] T007 [P] 既存UIコードをlegacyディレクトリに移動（src/ui/display.ts → src/ui/legacy/display.ts）
- [x] T008 [P] 既存UIコードをlegacyディレクトリに移動（src/ui/prompts.ts → src/ui/legacy/prompts.ts）
- [x] T009 [P] 既存UIコードをlegacyディレクトリに移動（src/ui/table.ts → src/ui/legacy/table.ts）
- [x] T010 フィーチャーフラグをsrc/index.tsに実装（USE_INK_UI環境変数）

---

## Phase 2: 基盤コンポーネント（すべてのストーリーで使用）

**目的**: 全画面で共有される基盤コンポーネントとフックの実装

### 型定義の拡張

- [ ] T011 data-model.mdに基づいてsrc/ui/types.tsを拡張（BranchItem, Statistics, Layout, Screen型を追加）

### カスタムフック（TDD）

- [ ] T012 [P] src/ui/hooks/useTerminalSize.tsのテストを作成
- [ ] T013 [P] T012の後にsrc/ui/hooks/useTerminalSize.tsを実装（ターミナルサイズ取得とresize監視）
- [ ] T014 [P] src/ui/hooks/useScreenState.tsのテストを作成
- [ ] T015 [P] T014の後にsrc/ui/hooks/useScreenState.tsを実装（画面状態管理）

### 共通コンポーネント（TDD）

- [ ] T016 [P] src/ui/components/common/ErrorBoundary.tsxのテストを作成
- [ ] T017 T016の後にsrc/ui/components/common/ErrorBoundary.tsxを実装（エラーキャッチとメッセージ表示）
- [ ] T018 [P] src/ui/components/common/Select.tsxのテストを作成
- [ ] T019 T018の後にsrc/ui/components/common/Select.tsxを実装（ink-select-inputのラッパー）
- [ ] T020 [P] src/ui/components/common/Confirm.tsxのテストを作成
- [ ] T021 T020の後にsrc/ui/components/common/Confirm.tsxを実装（確認ダイアログ）
- [ ] T022 [P] src/ui/components/common/Input.tsxのテストを作成
- [ ] T023 T022の後にsrc/ui/components/common/Input.tsxを実装（ink-text-inputのラッパー）

### UI部品コンポーネント（TDD）

- [ ] T024 [P] src/ui/components/parts/Header.tsxのテストを作成
- [ ] T025 T024の後にsrc/ui/components/parts/Header.tsxを実装（タイトルと区切り線）
- [ ] T026 [P] src/ui/components/parts/Footer.tsxのテストを作成
- [ ] T027 T026の後にsrc/ui/components/parts/Footer.tsxを実装（アクション説明）
- [ ] T028 [P] src/ui/components/parts/Stats.tsxのテストを作成
- [ ] T029 T028の後にsrc/ui/components/parts/Stats.tsxを実装（統計情報1行表示）
- [ ] T030 [P] src/ui/components/parts/ScrollableList.tsxのテストを作成
- [ ] T031 T030の後にsrc/ui/components/parts/ScrollableList.tsxを実装（スクロール可能リストコンテナ）

---

## Phase 3: ユーザーストーリー 1 - ブランチ一覧表示と選択 (優先度: P1)

**ストーリー**: 開発者がclaude-worktreeを起動すると、全画面レイアウトでブランチ一覧が表示される。ヘッダーにはタイトルと統計情報が固定表示され、ブランチリストはスクロール可能で、フッターにはキーボードアクションが常に表示される。

**価値**: MVP1 - 基本的なブランチ選択機能が動作

### データ変換ロジック（TDD）

- [ ] T032 [US1] src/ui/utils/branchFormatter.tsのテストを作成（BranchInfo → BranchItem変換）
- [ ] T033 [US1] T032の後にsrc/ui/utils/branchFormatter.tsを実装（アイコン生成、ラベル作成）
- [ ] T034 [US1] src/ui/utils/statisticsCalculator.tsのテストを作成（Statistics計算）
- [ ] T035 [US1] T034の後にsrc/ui/utils/statisticsCalculator.tsを実装（ブランチ/Worktree集計）

### カスタムフック（TDD）

- [ ] T036 [US1] src/ui/hooks/useGitData.tsのテストを作成（Git情報取得）
- [ ] T037 [US1] T036の後にsrc/ui/hooks/useGitData.tsを実装（getAllBranches, listAdditionalWorktreesを呼び出し）

### メイン画面コンポーネント（TDD）

- [ ] T038 [US1] src/ui/components/screens/BranchListScreen.tsxのテストを作成
- [ ] T039 [US1] T038の後にsrc/ui/components/screens/BranchListScreen.tsxを実装（全画面レイアウト: Header + Stats + ScrollableList + Footer）
- [ ] T040 [US1] T039の後にsrc/ui/components/screens/BranchListScreen.tsxにスクロール機能を統合（limitプロパティで動的行数制御）
- [ ] T041 [US1] T040の後にsrc/ui/components/screens/BranchListScreen.tsxにキーボードナビゲーションを実装（q=終了、Enter=選択）

### メインアプリケーション（TDD）

- [ ] T042 [US1] src/ui/components/App.tsxのテストを作成
- [ ] T043 [US1] T042の後にsrc/ui/components/App.tsxを実装（ErrorBoundary + BranchListScreenを統合）
- [ ] T044 [US1] T043の後にsrc/index.tsを更新（フィーチャーフラグでInk Appを起動）

### 統合テスト

- [ ] T045 [US1] src/ui/__tests__/integration/branchList.test.tsxを作成（ブランチ一覧画面の統合テスト）
- [ ] T046 [US1] T045の後にE2Eテストを実行（実際のGitリポジトリで動作確認）

**✅ MVP1チェックポイント**: US1完了後、ブランチ選択機能が独立して動作可能

### 受け入れテスト（US1）

- [ ] T047 [US1] 受け入れシナリオ1を検証: 1秒以内に全画面レイアウトが表示される
- [ ] T048 [US1] 受け入れシナリオ2を検証: 20個以上のブランチでスクロールがスムーズに動作
- [ ] T049 [US1] 受け入れシナリオ3を検証: ターミナルリサイズで表示行数が自動調整される
- [ ] T050 [US1] 受け入れシナリオ4を検証: ブランチ選択とEnterキーで処理開始
- [ ] T051 [US1] 受け入れシナリオ5を検証: qキーでアプリケーション終了

---

## Phase 4: ユーザーストーリー 2 - サブ画面のナビゲーション (優先度: P2)

**ストーリー**: 開発者が各種アクション（新規ブランチ作成、Worktree管理、PRクリーンアップ、AIツール選択など）を実行する際、同じ全画面レイアウトパターンで一貫したUIが提供される。画面遷移がスムーズで、戻る操作も直感的に行える。

**価値**: MVP2 - すべての既存機能が新UIで動作

### 画面状態管理の拡張

- [ ] T052 [US2] src/ui/hooks/useScreenState.tsを拡張（すべての画面タイプに対応）
- [ ] T053 [US2] T052の後にsrc/ui/components/App.tsxに画面遷移ロジックを追加

### Worktree管理画面（TDD）

- [ ] T054 [P] [US2] src/ui/components/screens/WorktreeManagerScreen.tsxのテストを作成
- [ ] T055 [US2] T054の後にsrc/ui/components/screens/WorktreeManagerScreen.tsxを実装（Worktree一覧表示とアクション選択）
- [ ] T056 [US2] T055の後にsrc/ui/components/App.tsxにWorktreeManager画面遷移を統合（mキー）

### 新規ブランチ作成画面（TDD）

- [ ] T057 [P] [US2] src/ui/components/screens/BranchCreatorScreen.tsxのテストを作成
- [ ] T058 [US2] T057の後にsrc/ui/components/screens/BranchCreatorScreen.tsxを実装（ブランチタイプ選択 → 名前入力）
- [ ] T059 [US2] T058の後にsrc/ui/components/App.tsxにBranchCreator画面遷移を統合（nキー）

### PRクリーンアップ画面（TDD）

- [ ] T060 [P] [US2] src/ui/components/screens/PRCleanupScreen.tsxのテストを作成
- [ ] T061 [US2] T060の後にsrc/ui/components/screens/PRCleanupScreen.tsxを実装（マージ済みPR一覧とクリーンアップ）
- [ ] T062 [US2] T061の後にsrc/ui/components/App.tsxにPRCleanup画面遷移を統合（cキー）

### AIツール選択画面（TDD）

- [ ] T063 [P] [US2] src/ui/components/screens/AIToolSelectorScreen.tsxのテストを作成
- [ ] T064 [US2] T063の後にsrc/ui/components/screens/AIToolSelectorScreen.tsxを実装（Claude/Codex選択）
- [ ] T065 [US2] T064の後にsrc/ui/components/App.tsxにAIToolSelector画面遷移を統合（ブランチ選択後）

### セッション選択画面（TDD）

- [ ] T066 [P] [US2] src/ui/components/screens/SessionSelectorScreen.tsxのテストを作成
- [ ] T067 [US2] T066の後にsrc/ui/components/screens/SessionSelectorScreen.tsxを実装（セッション一覧表示）
- [ ] T068 [US2] T067の後にsrc/ui/components/App.tsxにSessionSelector画面遷移を統合（-rオプション時）

### 実行モード選択画面（TDD）

- [ ] T069 [P] [US2] src/ui/components/screens/ExecutionModeSelectorScreen.tsxのテストを作成
- [ ] T070 [US2] T069の後にsrc/ui/components/screens/ExecutionModeSelectorScreen.tsxを実装（Normal/Continue/Resume選択）
- [ ] T071 [US2] T070の後にsrc/ui/components/App.tsxにExecutionModeSelector画面遷移を統合（AIツール選択後）

### 統合テスト

- [ ] T072 [US2] src/ui/__tests__/integration/navigation.test.tsxを作成（画面遷移フローのテスト）
- [ ] T073 [US2] T072の後にすべてのサブ画面でE2Eテストを実行

**✅ MVP2チェックポイント**: US2完了後、すべての既存機能が新UIで動作

### 受け入れテスト（US2）

- [ ] T074 [US2] 受け入れシナリオ1を検証: nキーで新規ブランチ作成画面に遷移
- [ ] T075 [US2] 受け入れシナリオ2を検証: qキー/ESCキーでメイン画面に戻る
- [ ] T076 [US2] 受け入れシナリオ3を検証: Worktree管理でアクション実行後に適切に遷移

---

## Phase 5: ユーザーストーリー 3 - リアルタイム統計情報の更新 (優先度: P3)

**ストーリー**: 開発者がアプリケーションを使用中に、統計情報（ブランチ数、Worktree数、変更ファイル数など）がバックグラウンドで更新され、常に最新の状態が表示される。Git操作を実行した後、画面を再起動せずに統計が自動更新される。

**価値**: 完全版 - 既存にない新機能（リアルタイム更新）を追加

### リアルタイム更新ロジック（TDD）

- [ ] T077 [US3] src/ui/hooks/useGitData.tsを拡張（定期更新ロジックを追加）
- [ ] T078 [US3] T077の後にsrc/ui/hooks/useGitData.tsのテストを更新（ポーリング動作を検証）

### 統計情報の動的更新

- [ ] T079 [US3] src/ui/components/parts/Stats.tsxを拡張（lastUpdated表示を追加）
- [ ] T080 [US3] T079の後にsrc/ui/components/parts/Stats.tsxのテストを更新

### パフォーマンス最適化

- [ ] T081 [P] [US3] React.memoを適用（BranchItem等の頻繁に再レンダリングされるコンポーネント）
- [ ] T082 [P] [US3] useMemo/useCallbackを適用（高コストな計算とコールバック）
- [ ] T083 [US3] T081, T082の後にパフォーマンステストを実行（100+ブランチで動作確認）

### 統合テスト

- [ ] T084 [US3] src/ui/__tests__/integration/realtimeUpdate.test.tsxを作成（リアルタイム更新のテスト）

**✅ 完全な機能**: US3完了後、すべての要件が満たされる

### 受け入れテスト（US3）

- [ ] T085 [US3] 受け入れシナリオ1を検証: 別ターミナルでGit操作後、数秒以内に統計情報が更新
- [ ] T086 [US3] 受け入れシナリオ2を検証: Worktree作成/削除後、統計情報が即座に更新

---

## Phase 6: 統合、ポリッシュ、移行完了

**目的**: すべてのストーリーを統合し、既存UIから完全移行

### テストカバレッジ検証

- [ ] T087 カバレッジレポートを生成（`bun test --coverage`）
- [ ] T088 T087の後に80%カバレッジ未達の場合、追加テストを作成
- [ ] T089 T088の後にカバレッジ目標達成を確認

### エッジケース対応

- [ ] T090 [P] 大量ブランチ（100+）でパフォーマンステスト実行
- [ ] T091 [P] ターミナルサイズが極小（10行以下）の場合の動作確認
- [ ] T092 [P] 非常に長いブランチ名の表示確認
- [ ] T093 Error Boundaryの動作確認（意図的にエラーを発生させてテスト）

### レガシーコード削除

- [ ] T094 フィーチャーフラグをデフォルトで新UIに変更（src/index.ts）
- [ ] T095 T094の後に既存テストがすべて通ることを確認
- [ ] T096 [P] T095の後にsrc/ui/legacy/ディレクトリを削除
- [ ] T097 [P] T095の後に@inquirer/prompts依存関係を削除

### ドキュメント更新

- [ ] T098 [P] README.mdを更新（Ink.js UI への移行を記載）
- [ ] T099 [P] CHANGELOG.mdを更新（UI移行の変更を記録）

### コード行数検証

- [ ] T100 UIコードの総行数を測定（目標: 760行以下）
- [ ] T101 T100の後に70%削減達成を確認

### 最終検証

- [ ] T102 すべての既存機能が正常動作することを確認（regressionゼロ）
- [ ] T103 すべてのパフォーマンス目標を達成していることを確認（<1秒起動、<50msスクロール）
- [ ] T104 すべての成功基準（SC-001～SC-008）を満たしていることを確認

---

## タスク統計

**総タスク数**: 104タスク

**ストーリー別内訳**:
- セットアップ（Phase 1）: 10タスク
- 基盤（Phase 2）: 21タスク
- US1（Phase 3）: 20タスク
- US2（Phase 4）: 25タスク
- US3（Phase 5）: 10タスク
- 統合・ポリッシュ（Phase 6）: 18タスク

**並列実行可能タスク**: 42タスク（[P]マーク付き）

**テストタスク**: 約50タスク（TDD により全コンポーネントでテスト作成）

---

## 依存関係グラフ

```
Phase 1（セットアップ）
    ↓
Phase 2（基盤）
    ↓
┌───────────┬───────────┬───────────┐
│  Phase 3  │  Phase 4  │  Phase 5  │
│   (US1)   │   (US2)   │   (US3)   │
│   [独立]  │[US1依存]  │[US2依存]  │
└───────────┴───────────┴───────────┘
    ↓
Phase 6（統合・ポリッシュ）
```

**ストーリー完了順序**:
1. US1完了 → MVP1デプロイ可能
2. US2完了 → MVP2デプロイ可能（既存機能完全互換）
3. US3完了 → 完全版デプロイ

---

## 並列実行例

### Phase 2（基盤）での並列実行

```bash
# グループ1: カスタムフック
T012 + T014 並列実行（テスト作成）
↓
T013 + T015 並列実行（実装）

# グループ2: 共通コンポーネント
T016 + T018 + T020 + T022 並列実行（テスト作成）
↓
T017 + T019 + T021 + T023 並列実行（実装）

# グループ3: UI部品
T024 + T026 + T028 + T030 並列実行（テスト作成）
↓
T025 + T027 + T029 + T031 並列実行（実装）
```

### Phase 4（US2）での並列実行

```bash
# すべてのサブ画面のテストを並列作成
T054 + T057 + T060 + T063 + T066 + T069 並列実行
↓
# 各画面の実装（順次または一部並列）
T055, T058, T061, T064, T067, T070
↓
# App.tsxへの統合（順次）
T056 → T059 → T062 → T065 → T068 → T071
```

---

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化
- **スキップしたタスク**: 理由と共に文書化

---

## 注記

- 各タスクは1時間から1日で完了可能
- TDD必須: すべてのコンポーネントはテストファーストで実装
- ファイルパスはplan.mdのプロジェクト構造に準拠
- 各ストーリーは独立してテスト・デプロイ可能
- フィーチャーフラグにより既存UIと新UIを切り替え可能

---

**作成日**: 2025-01-25
**次のコマンド**: `/speckit.implement` で実装開始
