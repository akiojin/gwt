# 実装計画: main/develop保護強化

**仕様ID**: `SPEC-4e948b89` | **日付**: 2025-11-06 | **仕様書**: ./spec.md
**入力**: `/specs/SPEC-4e948b89/spec.md`

## 概要

main・develop・masterをWorktree対象から除外し、UIとサービス層双方で保護する。Ink UIでは保護ブランチ選択時にWorktree作成アクションを隠し、`App.tsx`の遷移をルート利用フローへ直接迂回させる。同時に`createWorktree`でガードを設け、CLI内部の全経路で保護ブランチに対するWorktree作成を拒否する。

## 技術コンテキスト

- **言語/バージョン**: TypeScript 5.x + Bun 1.x
- **主要な依存関係**: Ink 6.x、React 19、execa 9.x
- **テスト**: Vitest、Ink Testing Library
- **ターゲットプラットフォーム**: Git Worktreeを扱うCLI (Linux/macOS)
- **制約**: 既存UI構造・型定義と整合性を保ちつつ最小変更で実現する

## 原則チェック

- CLAUDE.mdの「設計はシンプルに」「Spec/TDDを先行」方針に準拠。
- 新規ブランチやWorktreeを追加しないという運用ルールを満たす。

## プロジェクト構造

`src/ui/components/App.tsx`でブランチ選択フローを制御。`src/ui/screens/BranchActionSelectorScreen.tsx`のオプション表示変更、`src/worktree.ts`でワークツリー作成ガード、関連テストは`tests/ui`および`tests/unit/worktree.test.ts`で追加する。

## フェーズ0: 調査

- 現状の`SelectedBranchState`にはブランチ分類情報が無く、保護判定のために属性追加が必要。
- `PROTECTED_BRANCHES`定数を既存のクリーニング処理と共有しているため転用可能。
- UIテストは`integration/navigation.test.tsx`や`components/App.shortcuts.test.tsx`で分岐遷移を検証している。

## フェーズ1: 設計

- `SelectedBranchState`へ`branchCategory`を追加し、選択時に`BranchItem.branchType`を格納。
- `App.tsx`の`handleSelect`で保護ブランチを判定し、`navigateTo('ai-tool-selector')`へ直接遷移。警告メッセージをフッターに流すため`cleanupFooterMessage`など既存ステートを再利用。
- `BranchActionSelectorScreen`には`canCreateNew`プロップを追加し、保護ブランチでは「新規ブランチ作成」を表示しない。
- `createWorktree`に事前チェックを追加し、保護対象なら`WorktreeError`を投げる。メッセージには対象ブランチ名と推奨操作を含める。

## フェーズ2: タスク生成

タスクは`tasks.md`でブランチごとに分割し、TDD順序（テスト→実装→検証）で並べる。

## 実装戦略

1. 型とユーティリティの最小変更で保護判定を注入。
2. UIの分岐制御を追加し、`BranchActionSelectorScreen`のテストを更新。
3. Worktreeロジックに保護チェックを追加し、ユニットテストでカバー。
4. 統合テストで分岐フローを検証し、回帰を防ぐ。

## テスト戦略

- **ユニットテスト**: `tests/unit/worktree.test.ts`に保護ブランチ拒否ケースを追加。
- **コンポーネント/統合テスト**: `src/ui/screens/__tests__/BranchActionSelectorScreen.test.tsx`でオプション非表示を確認。`tests/ui/__tests__/integration/navigation.test.tsx`に保護ブランチ遷移の期待を追加。
- **回帰テスト**: 既存のVitestスイートを全面実行。

## リスクと緩和策

1. **型破壊リスク**: `SelectedBranchState`拡張により既存利用箇所が壊れる可能性 → ビルド前に型エラーを洗い、必要に応じてデフォルト値を設定。
2. **UI副作用**: 直接遷移により状態不整合が起こる可能性 → 既存ナビゲーションテストを更新し、回帰を防止。

## 次のステップ

1. `/specs/SPEC-4e948b89/data-model.md` と `quickstart.md` を作成して型および操作手順を整理
2. `/specs/SPEC-4e948b89/tasks.md` にTDD順のタスクを定義
3. テスト追加から実装へ進む
