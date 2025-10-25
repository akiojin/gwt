# 実装計画: ブランチ一覧の表示順序改善

**仕様ID**: `SPEC-a5ae4916` | **日付**: 2025-10-25 | **仕様書**: [specs/SPEC-a5ae4916/spec.md](./spec.md)
**入力**: 機能仕様および Clarify セッション結果

## 概要

ブランチ一覧のソート順を拡張し、現在のブランチ → main → develop → worktree あり → ローカル → その他 → 名前順（リモートのみ）で表示する。既存の `createBranchTable` 内ソート処理を最小改修で実現し、UI 表示形式と既存データ構造を維持する。

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.4（Bun 1.1 上で実行）
**主要な依存関係**: Bun、Inquirer、Vitest、TypeScript 標準ライブラリ
**ストレージ**: N/A（メモリ上での計算のみ）
**テスト**: Vitest（ユニット）、既存の CLI 実行テスト
**ターゲットプラットフォーム**: クロスプラットフォーム CLI（macOS / Linux / Windows）
**プロジェクトタイプ**: 単一 CLI アプリケーション（`src/` 配下 TypeScript モノレポ）
**パフォーマンス目標**: 300 ブランチでも 30ms 未満でソート（O(n log n) を維持）
**制約**: 既存の `BranchInfo` / `WorktreeInfo` を変更しない、UI アイコン/テキストを保持、ソートは安定に依存
**スケール/範囲**: ローカル/リモート/リリース系を含む最大 300 ブランチ

## 原則チェック

`.specify/memory/constitution.md` はテンプレート状態で有効な原則が定義されていない。拘束力のあるルールが存在しないためゲートは PASS（情報不足）として進め、Plan 後に原則文書の整備を推奨する。

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-a5ae4916/
├── plan.md              # 本ファイル
├── research.md          # フェーズ0出力
├── data-model.md        # フェーズ1出力（型とソート利用の整理）
├── quickstart.md        # 開発・検証手順
├── contracts/           # 今回は作成対象なし
├── spec.md              # 機能仕様
└── tasks.md             # /speckit.tasks で生成予定
```

### ソースコード（リポジトリルート）

```text
src/
├── ui/
│   ├── table.ts        # ソートロジック（主な変更箇所）
│   └── types.ts        # BranchInfo 型
├── worktree.ts         # worktree 情報の取得
└── ...

tests/
└── unit/ui/table.test.ts  # ソート順ユニットテスト
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 既存実装の理解とソート条件の優先順位決定
**出力**: [research.md](./research.md)

### 調査項目

1. **既存コードの分析**
   - `src/ui/table.ts` の現行ソート順
   - `BranchInfo` / `WorktreeInfo` 構造と取得フロー
   - `worktreeMap` の生成ロジック
   - `tests/unit/ui/table.test.ts` の既存テスト
2. **技術的決定**
   - develop を main の直後に固定
   - worktree 判定は `worktreeMap.has()` を使用
   - ローカル優先は `branch.type` 判定
   - release/hotfix ブランチは特別扱いせず一般ルールへ
3. **制約と依存関係**
   - 型定義変更不可
   - Array.sort の安定性に依存
   - Bun + Vitest 環境でテストを追加

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: ソート条件とテスト戦略を具体化
**出力**: [data-model.md](./data-model.md)、[quickstart.md](./quickstart.md)、contracts なし

### 1.1 データモデル設計

- `BranchInfo` の `branchType` で main / develop を判定
- `WorktreeInfo` を `worktreeMap` 経由で参照
- release/hotfix は worktree が無い限り汎用条件に従う

### 1.2 クイックスタートガイド

- TDD でソート順テストを先行
- Bun コマンドでのテスト/ビルド/フォーマット手順
- ソート結果のデバッグ方法

### 1.3 契約/インターフェース

- 外部 API の変更はなく、contracts/ は空のまま（必要なし）。

## フェーズ2: タスク生成

- `/speckit.tasks` で実装・テスト・検証の細分化タスクを生成予定。

## 実装戦略

### 優先順位付け

1. **P1**: 現在ブランチ、main、develop、worktree 優先を実装
2. **P2**: ローカル優先と release/hotfix の一般ルール確認
3. **P3**: エッジケーステストとパフォーマンス検証

### 独立したデリバリー

- ストーリー1で worktree 優先を提供（最小価値）
- ストーリー2でローカル優先を追加
- ストーリー3で既存優先順位維持と develop 固定を確認

## テスト戦略

- **ユニット**: `createBranchTable` のソート結果をモックデータで検証。現在/ main/ develop/ worktree/ ローカル/ release/hotfix/ 名前順の全パターンをテスト。
- **統合**: CLI 対話までは対象外。必要に応じて別フェーズで準備。
- **E2E**: 対象外。
- **パフォーマンス**: テスト内で 300 ブランチ程度のモックを生成し、計測または閾値チェックを追加検討。

## リスクと緩和策

### 技術的リスク

1. **ソート条件の矛盾**: 条件順が誤ると既存の優先度が変化
   - **緩和策**: テストで個別条件の期待順を保証し、コードにコメントで優先順位を明示
2. **worktreeMap 未生成時の挙動**: Maps が空でも正しい順序を維持できるか
   - **緩和策**: worktree なしケースのテストを追加し、従来順に戻ることを確認

### 依存関係リスク

1. **型変更の巻き込み**: 他タスクで BranchInfo が変更された場合にソートが破綻
   - **緩和策**: 型依存のテストで早期検知し、レビューでのチェック項目に追加

## 次のステップ

1. ✅ フェーズ0完了: research.md で既存分析と決定事項を確定
2. ✅ フェーズ1完了: data-model.md / quickstart.md を更新
3. ⏭️ `/speckit.tasks` で詳細タスクを生成
4. ⏭️ `/speckit.implement` で実装開始
