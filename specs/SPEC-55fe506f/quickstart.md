# クイックスタートガイド: Worktreeクリーンアップ選択機能

**仕様ID**: SPEC-55fe506f
**作成日**: 2025-11-10

## 概要

このガイドは、複数ブランチ選択機能の開発を始めるための手順を提供します。

## セットアップ

### 1. 依存関係のインストール

```bash
bun install
```

### 2. ビルド

```bash
bun run build
```

### 3. テストの実行

```bash
# 全テストを実行
bun run test

# ウォッチモードで実行
bun run test:watch

# カバレッジ付きで実行
bun run test:coverage
```

## 開発ワークフロー

### TDD（テスト駆動開発）の実践

このプロジェクトはTDDを採用しています。実装の前に必ずテストを作成してください。

#### 1. テストファイルの作成

```bash
# テストファイルの場所
tests/ui/components/common/Select.test.tsx
tests/ui/components/screens/BranchListScreen.test.tsx
tests/ui/components/App.test.tsx
```

#### 2. Red-Green-Refactorサイクル

```bash
# 1. Red: テストを書く（失敗する）
bun run test:watch

# 2. Green: 最小限の実装でテストを通す
# コードを書く

# 3. Refactor: リファクタリング
# コードを改善する
```

### テスト実行例

```bash
# 特定のテストファイルのみ実行
bun run test tests/ui/components/common/Select.test.tsx

# パターンマッチで実行
bun run test Select
```

## よくある操作

### 1. 新規ステートの追加

選択ブランチの状態は `src/cli/ui/components/App.tsx` で管理する。

### 2. コールバック関数の作成

同ファイルで、選択の切り替え・全解除のコールバックを用意する。

### 3. Props の渡し方

- App.tsx → BranchListScreen: `selectedBranches` と選択用コールバックを渡す。
- BranchListScreen.tsx → Select: スペース/ESC入力に応じて選択切り替えや解除を委譲する。

### 4. キーハンドリングの追加

入力処理は `src/cli/ui/components/screens/BranchListScreen.tsx` または
`src/cli/ui/components/common/Select.tsx` に集約し、スペースで選択切り替え、
ESCで選択解除を行う。

### 5. マーカー表示の実装

表示ロジックは `src/cli/ui/components/screens/BranchListScreen.tsx` の
`renderBranchRow` で統合する。

## デバッグ方法

### 1. コンソールログ

一時的なログ出力で選択状態の変化を追跡する。

### 2. Ink Devtools

Inkの公式devtoolsは現在利用不可のため、状態変化のログ出力やテスト実行で代替する。

### 3. テストでのデバッグ

`ink-testing-library` の `lastFrame` で表示内容を確認する。

## トラブルシューティング

### 問題1: テストが失敗する

**症状**: `TypeError: Cannot read property 'has' of undefined`

**原因**: `selectedBranches` がundefinedの可能性

**解決策**:
`selectedBranches` が未定義でも処理できるよう、防御的に判定する。

---

### 問題2: キー入力が反応しない

**症状**: スペースキーを押しても選択されない

**原因**: `onSpace` が正しく渡されていない可能性

**解決策**:
1. Propsの型定義を確認
2. コンソールログで `onSpace` が関数として渡されているか確認
3. `disabled` 状態を確認

---

### 問題3: マーカーが表示されない

**症状**: `*` マーカーが画面に表示されない

**原因**: レイアウト計算の問題

**解決策**:
1. `staticPrefix` の文字列を確認
2. `stringWidth()` の計算が正しいか確認
3. ターミナルの幅が十分か確認

---

### 問題4: ビルドエラー

**症状**: `Property 'onSpace' does not exist on type 'SelectProps'`

**原因**: 型定義が更新されていない

**解決策**:
```bash
# TypeScriptの型チェック
bun run type-check

# ビルドのクリーン
bun run clean
bun run build
```

---

## 開発Tips

### 1. 型安全性の確保

OptionalなPropsは常に `?:` を使用し、使用時は `?.` / `??` でnullチェックする。

### 2. パフォーマンスの最適化

`useCallback` と `useMemo` を使い、依存配列を最小限にして再計算を抑える。

### 3. コミットメッセージ

Conventional Commitsに従う（`feat` / `fix` / `test` など）。

### 4. コードレビュー前のチェックリスト

- [ ] テストがすべてパスする
- [ ] 型チェックがパスする (`bun run type-check`)
- [ ] Lintエラーがない (`bun run lint`)
- [ ] フォーマットが整っている (`bun run format`)
- [ ] コミットメッセージが Conventional Commits に従っている

## 参考リソース

- [Ink Documentation](https://github.com/vadimdemedes/ink)
- [Vitest Documentation](https://vitest.dev/)
- [React Testing Library](https://testing-library.com/docs/react-testing-library/intro/)
- [Conventional Commits](https://www.conventionalcommits.org/)

## 次のステップ

1. [spec.md](./spec.md) を読んで要件を理解
2. [research.md](./research.md) を読んで技術的決定を確認
3. [data-model.md](./data-model.md) を読んでデータ構造を理解
4. `/speckit.tasks` を実行してタスクリストを生成
5. テストを書いて実装を開始
