# 実装タスク: Worktreeクリーンアップ選択機能

**仕様ID**: SPEC-55fe506f
**作成日**: 2025-11-10
**関連ドキュメント**: [spec.md](./spec.md) | [plan.md](./plan.md) | [data-model.md](./data-model.md)

## 概要

このドキュメントは、複数ブランチ選択機能の実装タスクを定義します。各ユーザーストーリーを独立して実装・テスト・デプロイ可能な単位に分割しています。

## タスク凡例

- `- [ ]`: 未完了タスク
- `[TXX]`: タスクID（実行順序）
- `[P]`: 並列実行可能（他のタスクと依存関係なし）
- `[USX]`: ユーザーストーリーラベル

## Phase 1: セットアップ

### 目標
プロジェクトの初期設定と依存関係の確認。

### タスク

- [ ] T001 プロジェクト構造の確認（package.json、tsconfig.json、既存コンポーネント）
- [ ] T002 [P] 開発環境のセットアップ（`bun install`、ビルド確認）
- [ ] T003 [P] テスト実行の確認（`bun run test`）

## Phase 2: 基盤実装

### 目標
全ユーザーストーリーで使用される共通基盤を実装。

### タスク

- [ ] T004 選択状態管理の型定義を `src/ui/types.ts` に追加
- [ ] T005 [P] `src/ui/components/App.tsx` に選択状態ステート (`selectedBranches: Set<string>`) を追加
- [ ] T006 [P] `src/ui/components/App.tsx` に選択トグル関数 (`toggleBranchSelection`) を実装
- [ ] T007 [P] `src/ui/components/App.tsx` に全選択解除関数 (`clearBranchSelection`) を実装
- [ ] T008 `src/ui/components/common/Select.tsx` の Props に `onSpace` と `onEscape` を追加
- [ ] T009 `src/ui/components/screens/BranchListScreen.tsx` の Props に選択関連Props を追加

## Phase 3: ユーザーストーリー 4 - 既存機能の維持 (P1)

### 目標
既存のEnterキーによるブランチ切り替え機能が正常に動作することを確認。

### 独立したテスト基準
- Enterキーでブランチ切り替えが正常に動作する
- 選択状態に関係なくEnterキーの動作は変わらない

### テスト

- [ ] T010 [P] [US4] `tests/ui/components/App.test.tsx` に既存ブランチ切り替えの回帰テストを追加
- [ ] T011 [P] [US4] `tests/ui/components/screens/BranchListScreen.test.tsx` に Enterキー動作のテストを追加

### 実装

- [ ] T012 [US4] `src/ui/components/screens/BranchListScreen.tsx` の `onSelect` コールバックが正常に動作することを確認（コード変更なし、既存動作の検証のみ）

## Phase 4: ユーザーストーリー 1 - 複数ブランチの選択とクリーンアップ (P1)

### 目標
スペースキーで複数ブランチを選択し、選択されたブランチのみをクリーンアップできる。

### 独立したテスト基準
- スペースキーでブランチを選択/解除できる
- 選択されたブランチのみがクリーンアップされる
- 保護ブランチは選択できない

### テスト

- [ ] T013 [P] [US1] `tests/ui/components/common/Select.test.tsx` にスペースキー処理のテストを追加
- [ ] T014 [P] [US1] `tests/ui/components/screens/BranchListScreen.test.tsx` に選択トグルのテストを追加
- [ ] T015 [P] [US1] `tests/ui/components/App.test.tsx` に選択状態管理のテストを追加
- [ ] T016 [P] [US1] `tests/ui/components/App.test.tsx` にクリーンアップ実行（選択ブランチのみ）のテストを追加

### 実装

- [ ] T017 [US1] `src/ui/components/common/Select.tsx` にスペースキー処理を実装（`onSpace` コールバック呼び出し）
- [ ] T018 [US1] `src/ui/components/screens/BranchListScreen.tsx` に `onSpace` と `onToggleSelection` を接続
- [ ] T019 [US1] `src/ui/components/screens/BranchListScreen.tsx` に保護ブランチ判定を実装（`isProtectedBranchName` 使用）
- [ ] T020 [US1] `src/ui/components/App.tsx` の `handleCleanupCommand` を修正（選択ブランチのみクリーンアップ）
- [ ] T021 [US1] `src/ui/components/App.tsx` に0個選択時の警告処理を追加

### 統合テスト

- [ ] T022 [US1] 手動テスト: スペースキーでの選択とクリーンアップの動作確認

## Phase 5: ユーザーストーリー 2 - 選択状態の視覚的フィードバック (P1)

### 目標
選択されたブランチに `*` マーカーを表示し、警告対象は赤色で表示する。

### 独立したテスト基準
- 選択されたブランチに `*` マーカーが表示される
- 通常ブランチは白色の `*`、警告対象は赤色の `*` で表示される
- 選択数がリアルタイムで表示される

### テスト

- [ ] T023 [P] [US2] `tests/ui/components/screens/BranchListScreen.test.tsx` にマーカー表示のテストを追加
- [ ] T024 [P] [US2] `tests/ui/components/screens/BranchListScreen.test.tsx` にマーカー色分けのテストを追加
- [ ] T025 [P] [US2] `tests/ui/components/screens/BranchListScreen.test.tsx` に選択数表示のテストを追加

### 実装

- [ ] T026 [US2] `src/ui/components/screens/BranchListScreen.tsx` の `renderBranchRow` に選択マーカー表示を実装
- [ ] T027 [US2] `src/ui/components/screens/BranchListScreen.tsx` に警告判定ロジックを実装（`hasUnpushedCommits` || `!mergedPR`）
- [ ] T028 [US2] `src/ui/components/screens/BranchListScreen.tsx` にマーカー色分けロジックを実装（`chalk.red` 使用）
- [ ] T029 [US2] `src/ui/components/screens/BranchListScreen.tsx` のFooterに選択数表示を追加

### 統合テスト

- [ ] T030 [US2] 手動テスト: 各種ブランチ（通常/警告）でのマーカー表示確認

## Phase 6: ユーザーストーリー 3 - 選択解除と修正 (P2)

### 目標
ESCキーで全選択を解除し、柔軟に選択内容を修正できる。

### 独立したテスト基準
- スペースキーで個別に選択を解除できる
- ESCキーで全選択を解除できる
- 何も選択されていない状態でESCキーを押してもエラーにならない

### テスト

- [ ] T031 [P] [US3] `tests/ui/components/common/Select.test.tsx` に ESCキー処理のテストを追加
- [ ] T032 [P] [US3] `tests/ui/components/App.test.tsx` に全選択解除のテストを追加
- [ ] T033 [P] [US3] `tests/ui/components/screens/BranchListScreen.test.tsx` に ESCキー動作のテストを追加

### 実装

- [ ] T034 [US3] `src/ui/components/common/Select.tsx` に ESCキー処理を実装（`onEscape` コールバック呼び出し）
- [ ] T035 [US3] `src/ui/components/screens/BranchListScreen.tsx` に `onEscape` と `onClearSelection` を接続
- [ ] T036 [US3] `src/ui/components/App.tsx` の `clearBranchSelection` が正しく動作することを確認（Phase 2 で実装済み）

### 統合テスト

- [ ] T037 [US3] 手動テスト: 個別解除とESCキーでの全選択解除の動作確認

## Phase 7: 統合とポリッシュ

### 目標
全ユーザーストーリーの統合テストとドキュメント更新。

### タスク

- [ ] T038 [P] 全ユーザーストーリーの統合テスト実行
- [ ] T039 [P] 型チェック実行（`bun run type-check`）
- [ ] T040 [P] Lint実行（`bun run lint`）
- [ ] T041 [P] フォーマット実行（`bun run format`）
- [ ] T042 E2Eテスト: 実際のGitリポジトリでの動作確認
- [ ] T043 パフォーマンステスト: 100+ブランチでの動作確認
- [ ] T044 [P] README.md の更新（機能説明を追加）
- [ ] T045 コミットとプッシュ

## 依存関係

### ユーザーストーリー完了順序

```
Phase 1: セットアップ
    ↓
Phase 2: 基盤実装
    ↓
┌───────────────┬───────────────┐
│  Phase 3: US4 │  Phase 4: US1 │  ← 並列実行可能
│  (既存機能)   │  (選択機能)   │
└───────┬───────┴───────┬───────┘
        │               │
        └───────┬───────┘
                ↓
        Phase 5: US2
        (視覚フィードバック)
                ↓
        Phase 6: US3
        (選択解除)
                ↓
        Phase 7: 統合
```

### MVP スコープ

**推奨MVP**: Phase 2 + Phase 4 (US1)
- 基本的な選択機能とクリーンアップが動作
- 最小限の価値提供が可能

**フルMVP**: Phase 2 + Phase 3 + Phase 4 + Phase 5
- 視覚フィードバック付きで安全性が向上
- 実用的なレベルで提供可能

## 並列実行の機会

### Phase 2（基盤実装）
```bash
# 並列実行可能
T005, T006, T007  # App.tsx のステート管理（独立した関数）
T008              # Select.tsx の Props 拡張
T009              # BranchListScreen.tsx の Props 拡張
```

### Phase 3（US4）
```bash
# 並列実行可能
T010, T011  # テストファイル作成（独立したファイル）
```

### Phase 4（US1）
```bash
# 並列実行可能（テスト作成）
T013, T014, T015, T016  # 各テストファイル（独立したファイル）
```

### Phase 5（US2）
```bash
# 並列実行可能（テスト作成）
T023, T024, T025  # テストファイル（独立したファイル）
```

### Phase 6（US3）
```bash
# 並列実行可能（テスト作成）
T031, T032, T033  # テストファイル（独立したファイル）
```

### Phase 7（統合）
```bash
# 並列実行可能
T038, T039, T040, T041, T044  # 独立した検証・ドキュメント作業
```

## 実装戦略

### TDD アプローチ

1. **Red**: テストを書く（失敗する）
2. **Green**: 最小限の実装でテストを通す
3. **Refactor**: リファクタリング

### 段階的デリバリー

- **Stage 1**: Phase 2 + Phase 4 → 基本的な選択機能
- **Stage 2**: Phase 5 → 視覚フィードバック追加
- **Stage 3**: Phase 6 → 選択解除機能追加
- **Stage 4**: Phase 7 → 統合とポリッシュ

各ステージで独立してデプロイ可能。

## タスク統計

- **総タスク数**: 45
- **Phase 1（セットアップ）**: 3タスク
- **Phase 2（基盤）**: 6タスク
- **Phase 3（US4 - P1）**: 3タスク
- **Phase 4（US1 - P1）**: 10タスク
- **Phase 5（US2 - P1）**: 8タスク
- **Phase 6（US3 - P2）**: 7タスク
- **Phase 7（統合）**: 8タスク

**並列実行可能タスク**: 24タスク（53%）

## 進捗追跡

進捗は以下のコマンドで確認できます：

```bash
# 完了タスク数を確認
grep -c "\- \[x\]" specs/SPEC-55fe506f/tasks.md

# 未完了タスク数を確認
grep -c "\- \[ \]" specs/SPEC-55fe506f/tasks.md

# ユーザーストーリー別の進捗
grep "\[US1\]" specs/SPEC-55fe506f/tasks.md | grep -c "\[x\]"
grep "\[US2\]" specs/SPEC-55fe506f/tasks.md | grep -c "\[x\]"
grep "\[US3\]" specs/SPEC-55fe506f/tasks.md | grep -c "\[x\]"
grep "\[US4\]" specs/SPEC-55fe506f/tasks.md | grep -c "\[x\]"
```

## 次のステップ

```bash
# 実装を開始
/speckit.implement

# または手動でタスクを実行
bun run test:watch  # TDDモードでテストを実行
```
