# 実装計画: 一括ブランチマージ機能

**仕様ID**: `SPEC-ee33ca26` | **日付**: 2025-10-27 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-ee33ca26/spec.md` からの機能仕様

## 概要

本機能は、ブランチ一覧画面から全てのローカルブランチに対してmain/developから最新の変更を一括でマージする機能を提供します。開発者は'p'キーを押下するだけで、複数のfeatureブランチへの手動マージ作業を自動化でき、作業時間を80%以上削減できます。

主要機能：
- **基本一括マージ** (P1): 全ローカルブランチへの自動マージ、worktree自動作成、コンフリクトスキップ
- **リアルタイム進捗表示** (P1): 現在処理中のブランチ、進捗率、経過時間の表示
- **ドライランモード** (P2): 実行シミュレーションによる事前確認
- **自動プッシュ** (P3): マージ成功後のリモート自動反映

技術的アプローチ：
- 既存のgit操作モジュール(git.ts)を拡張してマージ関連関数を追加
- 新規BatchMergeServiceを作成してオーケストレーション処理を実装
- Ink.jsベースの新規UI画面（進捗表示、結果サマリー）を追加
- Vitestによる包括的なテストカバレッジ確保

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.8+ (target: ES2022, module: ESNext)
**ランタイム**: Bun 1.0+
**主要な依存関係**:
- Ink.js 6.3+ (React for CLI UI)
- execa 9.6+ (git コマンド実行)
- React 19.2+ (Ink.jsのベース)
- Vitest 2.1+ (テストフレームワーク)
**ストレージ**: N/A (全てgitリポジトリ操作)
**テスト**: Vitest (unit + integration + e2e), ink-testing-library, @testing-library/react
**ターゲットプラットフォーム**: CLI (Linux, macOS, Windows via Bun)
**プロジェクトタイプ**: 単一CLIアプリケーション
**パフォーマンス目標**:
- 5ブランチを1分以内に処理
- 20ブランチの同時処理サポート
- リアルタイム進捗更新（500ms以内）
**制約**:
- Worktree設計思想準拠（ブランチ切り替え禁止）
- コンフリクトは手動解決のみ
- 既存のgit.ts、worktree.tsモジュールと統合
**スケール/範囲**:
- 想定対象ブランチ数: 1〜50ブランチ
- 1ブランチあたりの処理時間: 5〜10秒

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

### CLAUDE.md原則との整合性

✅ **シンプルさの極限追求**:
- 既存のgit.ts、worktree.tsモジュールを再利用
- 新規モジュールはBatchMergeServiceのみ
- UI追加も既存のInk.jsパターンを踏襲

✅ **ユーザビリティと開発者体験**:
- 'p'キー1つで一括処理開始
- リアルタイム進捗表示による透明性確保
- ドライランモードによるリスク低減

✅ **TDD絶対遵守**:
- 全ての新規関数にunit test作成
- 統合テスト（マージフロー全体）
- E2Eテスト（UI操作含む）

✅ **並列化**:
- 調査・設計・実装タスクは最大限並列化
- ただし、ブランチのマージ処理自体は順次実行（仕様準拠）

✅ **エラーなしでの完了**:
- 全テストパス後のみ完了とする
- ビルドエラー0件
- lint/type-check通過

**違反なし - Phase 0へ進行可**

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-ee33ca26/
├── plan.md              # このファイル（/speckit.plan コマンド出力）
├── research.md          # フェーズ0出力（/speckit.plan コマンド）
├── data-model.md        # フェーズ1出力（/speckit.plan コマンド）
├── quickstart.md        # フェーズ1出力（/speckit.plan コマンド）
├── contracts/           # フェーズ1出力（該当なし - 内部API）
└── tasks.md             # フェーズ2出力（/speckit.tasks コマンド - /speckit.planでは作成されません）
```

### ソースコード（リポジトリルート）

```text
src/
├── git.ts                        # [拡張] マージ関連関数追加
├── services/
│   └── BatchMergeService.ts      # [新規] 一括マージオーケストレーター
├── ui/
│   ├── types.ts                  # [拡張] BatchMerge関連型追加
│   ├── components/
│   │   ├── screens/
│   │   │   ├── BranchListScreen.tsx      # [拡張] 'p'キー追加
│   │   │   ├── BatchMergeProgressScreen.tsx  # [新規] 進捗表示画面
│   │   │   └── BatchMergeResultScreen.tsx    # [新規] 結果サマリー画面
│   │   └── parts/
│   │       ├── ProgressBar.tsx   # [新規] 進捗バー部品
│   │       └── MergeStatusList.tsx  # [新規] マージステータスリスト部品
│   └── hooks/
│       └── useBatchMerge.ts      # [新規] バッチマージロジックフック

tests/
├── unit/
│   ├── git.test.ts               # [拡張] マージ関数テスト追加
│   └── services/
│       └── BatchMergeService.test.ts  # [新規]
├── integration/
│   └── batch-merge.test.ts       # [新規] 一括マージ統合テスト
└── e2e/
    └── batch-merge-workflow.test.ts  # [新規] E2Eテスト
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 要件に基づいて技術スタックを決定し、既存のコードパターンを理解する

**出力**: `specs/SPEC-ee33ca26/research.md`

### 調査項目

1. **既存のコードベース分析**
   - ✅ 現在の技術スタック: TypeScript 5.8+ + Bun 1.0+ + Ink.js 6.3+
   - ✅ アーキテクチャパターン: Repository層 + Service層 + UI層
   - ✅ 統合ポイント: git.ts (git操作), worktree.ts (worktree管理), BranchListScreen (UI)

2. **技術的決定**
   - **決定1**: git merge実装方法（git.ts拡張 vs 新規モジュール）
   - **決定2**: 進捗表示のリアルタイム更新方法（polling vs event-driven）
   - **決定3**: ドライランモードの実装方法（一時worktree vs git merge-tree）
   - **決定4**: テスト戦略（モック vs 実際のgitリポジトリ）

3. **制約と依存関係**
   - **制約1**: Worktree設計思想（ブランチ切り替え禁止）
   - **制約2**: 既存git.ts、worktree.tsとの互換性維持
   - **制約3**: Ink.jsのCLI制約（ターミナルサイズ、更新頻度）

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-ee33ca26/data-model.md`
- `specs/SPEC-ee33ca26/quickstart.md`
- `specs/SPEC-ee33ca26/contracts/` （該当なし - 内部API）

### 1.1 データモデル設計

**ファイル**: `data-model.md`

主要なエンティティとその関係を定義：
- **BatchMergeConfig**: 設定（マージ元、対象ブランチ、オプション）
- **BatchMergeProgress**: 進捗状態（現在ブランチ、進捗率、経過時間）
- **BranchMergeStatus**: 各ブランチのマージ結果（成功/スキップ/失敗）
- **BatchMergeResult**: 最終結果（全ステータス、サマリー統計）

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの簡潔なガイド：
- セットアップ手順（依存関係なし - 既存環境で動作）
- 開発ワークフロー（TDDサイクル）
- よくある操作（テスト実行、ビルド、デバッグ）
- トラブルシューティング（コンフリクト処理、worktree問題）

### 1.3 契約/インターフェース（該当する場合）

**ディレクトリ**: `contracts/`

**該当なし** - 本機能は内部APIのみで、外部公開APIはありません。
内部API設計はdata-model.mdに記載します。

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-ee33ca26/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装：
1. **P1**: 基本一括マージ + リアルタイム進捗表示 - 最初に実装
2. **P2**: ドライランモード - P1の後
3. **P3**: 自動プッシュ - 最後に

### 独立したデリバリー

各ユーザーストーリーは独立して実装・テスト・デプロイ可能：
- **P1完了** → MVP（基本的な一括マージ機能）
- **P2追加** → リスク低減版（ドライラン追加）
- **P3追加** → 完全自動化版（プッシュ自動化）

### TDD実装フロー

1. テスト作成（Red）
2. ユーザー承認（仕様確認）
3. テスト実行（Fail確認）
4. 実装（Green）
5. リファクタリング（Refactor）
6. コミット＆プッシュ

## テスト戦略

### ユニットテスト

**対象**:
- git.ts の新規マージ関数（mergeFromBranch, hasMergeConflict, abortMerge）
- BatchMergeService の全メソッド
- Reactフック useBatchMerge

**アプローチ**:
- execaをモック（happy path + error cases）
- 境界値テスト（0ブランチ、1ブランチ、多数ブランチ）

### 統合テスト

**対象**:
- 一括マージフロー全体（fetch → worktree作成 → merge → push）
- コンフリクト処理フロー
- キャンセル処理フロー

**アプローチ**:
- テスト用gitリポジトリを動的作成
- 実際のgitコマンド実行
- 後処理でクリーンアップ

### エンドツーエンドテスト

**対象**:
- UI操作を含む全フロー（'p'キー押下 → 結果表示）
- 進捗表示の更新
- エラーメッセージ表示

**アプローチ**:
- ink-testing-library使用
- ユーザー操作をシミュレート
- 画面出力を検証

### パフォーマンステスト

**対象**:
- 20ブランチの処理時間測定
- メモリ使用量モニタリング

**基準**:
- 1ブランチあたり平均10秒以内
- メモリ使用量100MB以下

## リスクと緩和策

### 技術的リスク

1. **リスク1: git merge中のコンフリクト検出が不完全**
   - **緩和策**: git statusとgit diffで複数確認、統合テストで全パターン検証

2. **リスク2: Ink.jsのリアルタイム更新でちらつき発生**
   - **緩和策**: 更新頻度を500ms程度に制限、React.memoで不要な再描画防止

3. **リスク3: 多数ブランチでの処理時間超過**
   - **緩和策**: 進捗表示で透明性確保、ドライランで事前推定可能に

### 依存関係リスク

1. **依存関係1: execaのバージョン変更によるAPI変更**
   - **緩和策**: package.jsonで固定バージョン指定、テストでカバレッジ確保

2. **依存関係2: Bunのgit互換性問題**
   - **緩和策**: 実gitリポジトリでの統合テスト、CIで継続検証

## 次のステップ

1. ✅ セットアップ完了: plan.md作成
2. ✅ フェーズ0完了: research.md生成
3. ✅ フェーズ1完了: data-model.md, quickstart.md生成、agent context更新
4. ⏭️ `/speckit.tasks` を実行してタスクを生成
5. ⏭️ `/speckit.analyze` で品質分析
6. ⏭️ `/speckit.implement` で実装を開始
