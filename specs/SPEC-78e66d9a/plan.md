# 実装計画: semantic-release リリースワークフローの安定化

**仕様ID**: `SPEC-78e66d9a` | **日付**: 2025-11-05 | **仕様書**: `specs/SPEC-78e66d9a/spec.md`
**入力**: `/specs/SPEC-78e66d9a/spec.md` からの機能仕様

## 概要

リリースワークフローで発生した LoadingIndicator テストのタイミング依存失敗を解消し、Spinner が期待通りに進行することを保証する。疑似タイマーを用いた deterministic な検証へ置き換え、低速環境でも安定するよう調整する。

## 技術コンテキスト

- **言語/バージョン**: TypeScript 5.8 / React 19 / Ink 6 / Bun 1.0+ / Node 22 系
- **主要な依存関係**: `react`, `ink`, `@testing-library/react`, `vitest`, `happy-dom`
- **ストレージ**: N/A
- **テスト**: Vitest (DOM: happy-dom), @testing-library/react
- **ターゲットプラットフォーム**: CLI (Ink) / GitHub Actions 上の Bun ランナー
- **プロジェクトタイプ**: 単一 CLI プロジェクト
- **パフォーマンス目標**: テストファイルの実行時間 < 0.5s、リリースワークフロー全体の追加時間 0s
- **制約**: 既存 Props/API を変更しない、追加依存は導入しない
- **スケール/範囲**: 単一コンポーネントとテストの修正に限定

## 原則チェック

- 既存ワークフローを尊重し、テストを deterministic に保つ。
- リリース安定性を最優先し、UI 表示仕様は現状維持。

## プロジェクト構造

```text
specs/SPEC-78e66d9a/
├── plan.md
├── spec.md
├── research.md          # 本計画で調査結果を記載予定
├── data-model.md        # コンポーネントとテスト対象の構造を記載予定
├── quickstart.md        # 再現手順とローカル検証を記載予定
└── tasks.md             # TDD 実装タスクリスト
```

ソース変更は以下を対象とする:

- `src/ui/components/common/LoadingIndicator.tsx`
- `src/ui/__tests__/components/common/LoadingIndicator.test.tsx`
- 必要に応じて関連ヘルパー

## フェーズ0: 調査（技術スタック選定）

- release ワークフローの失敗ログ確認 (GitHub Actions 19107220150)
- LoadingIndicator の現行実装とタイマー依存箇所確認
- Vitest のタイマー API と happy-dom の互換性確認

**出力**: `specs/SPEC-78e66d9a/research.md`

## フェーズ1: 設計（アーキテクチャと契約）

### 1.1 データモデル設計

`data-model.md` で以下を定義:

- LoadingIndicator の state 遷移 (visible, frameIndex)
- Timer リソース (timeout / interval) のライフサイクル
- テストフロー (fake timers, frame capture)

### 1.2 クイックスタートガイド

`quickstart.md` に以下を記載:

- release ワークフロー失敗の再現手順
- ローカルでのテスト実行と失敗再現 (必要なら --repeat)
- 修正後の確認手順 (`bun run test ...`)
- semantic-release ワークフローの手動再実行手順

### 1.3 契約/インターフェース

API 変更は行わないため新規契約ファイルは不要。

## フェーズ2: タスク生成

`/speckit.tasks` を実行し、フェーズ別タスクリストを `tasks.md` にまとめる。  
主なタスク例:

1. 失敗ログの抜粋と flaky 条件分析
2. LoadingIndicator 実装のタイマー管理確認
3. テストの fake timers 化と安定化
4. ローカル/CI での再検証と release ワークフローの再実行

## 実装戦略

1. P1: テストを deterministic にする（fake timers への切替）
2. P1: Component 側で必要があれば interval/drain ロジックを調整
3. P2: ログや quickstart 更新で再現手順と確認方法を明文化

## テスト戦略

- **ユニットテスト**: LoadingIndicator の既存テストを更新し deterministic に。
- **統合テスト**: N/A。
- **エンドツーエンド**: release ワークフローを手動トリガーし確認。
- **パフォーマンステスト**: なし。

## リスクと緩和策

1. **疑似タイマーと happy-dom の互換性問題**  
   - **緩和策**: サンプリングテストを複数回実行し挙動を確認、必要なら `advanceTimersToNextTimer` を使用。
2. **Spinner ロジックの変更による副作用**  
   - **緩和策**: コンポーネントの既存 props 振る舞いを維持し、テストでメッセージ表示など副作用をチェック。

## 次のステップ

1. フェーズ0 調査を実施し `research.md` を更新
2. フェーズ1 設計資料 (`data-model.md`, `quickstart.md`) を作成
3. `/speckit.tasks` でタスクリスト作成
4. タスクに従って実装とテストを進める
