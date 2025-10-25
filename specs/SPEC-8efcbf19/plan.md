# 実装計画: Docker/root環境でのClaude Code自動承認機能

**仕様ID**: `SPEC-8efcbf19` | **日付**: 2025-10-25 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-8efcbf19/spec.md` からの機能仕様

**注**: このテンプレートは `/speckit.plan` コマンドによって記入されます。実行ワークフローについては `.specify/templates/commands/plan.md` を参照してください。

## 概要

Docker/root環境でClaude Codeの`--dangerously-skip-permissions`フラグを動作させるため、rootユーザー検出時にIS_SANDBOX=1環境変数を自動設定する機能を追加します。この機能により、Docker環境でのPermission prompt削減とセキュリティ警告の表示を実現します。

**主要要件**:
- rootユーザー検出（process.getuid() === 0）
- skipPermissions=true時のIS_SANDBOX=1環境変数設定
- セキュリティ警告メッセージの表示
- 非rootユーザーでの既存動作維持

**技術的アプローチ**:
- 既存のlaunchClaudeCode関数（src/claude.ts）を修正
- execa環境変数設定機能を使用
- rootユーザー検出にはprocess.getuid() APIを使用（POSIXシステム）

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.8+ / Bun 1.0+
**主要な依存関係**: execa (9.6+), chalk (5.4+), @inquirer/prompts (6.0+)
**ストレージ**: N/A（環境変数のみ）
**テスト**: Vitest 2.1+ (unit/integration/e2e)
**ターゲットプラットフォーム**: Linux/macOS (POSIX準拠システム)、Docker環境
**プロジェクトタイプ**: CLI Tool (単一プロジェクト)
**パフォーマンス目標**: N/A（起動時の1回のみ実行、<10ms目標）
**制約**:
- process.getuid()はPOSIXシステムのみ対応（Windows非対応）
- IS_SANDBOX=1は非公式環境変数（Claude Code側の将来変更リスクあり）
- 既存のlaunchClaudeCode関数を非破壊的に拡張する必要あり
**スケール/範囲**: 単一関数の修正（src/claude.ts内、約15行の追加）

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

**ステータス**: ✅ **PASS** (原則ファイル未定義のため、デフォルト合格)

**評価**:
- `.specify/memory/constitution.md`はテンプレートのままで、プロジェクト固有の原則が未定義
- 一般的なベストプラクティスに従う:
  - ✅ 既存コードの非破壊的修正（後方互換性維持）
  - ✅ セキュリティリスクの明示（警告メッセージ表示）
  - ✅ テスタビリティ（ユニットテスト可能な設計）
  - ✅ シンプルさ優先（最小限の変更で機能実現）

**推奨事項**: プロジェクト原則を`.specify/memory/constitution.md`に定義することで、将来の機能開発時の一貫性を向上できます。

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-8efcbf19/
├── spec.md              # 機能仕様（完了済み）
├── plan.md              # このファイル（/speckit.plan コマンド出力）
├── research.md          # フェーズ0出力（/speckit.plan コマンド）
├── data-model.md        # フェーズ1出力（N/A - データモデルなし）
├── quickstart.md        # フェーズ1出力（/speckit.plan コマンド）
└── tasks.md             # フェーズ2出力（/speckit.tasks コマンド）
```

### ソースコード（リポジトリルート）

```text
src/
├── claude.ts            # 修正対象ファイル（launchClaudeCode関数）
├── index.ts             # メインエントリーポイント（Claude Code起動呼び出し）
├── ui/
│   └── prompts.ts       # UI関連（skipPermissions選択プロンプト）
└── types.ts             # 型定義

tests/
├── unit/
│   └── claude.test.ts   # launchClaudeCode関数のユニットテスト（新規作成）
└── integration/
    └── root-sandbox.test.ts  # root環境統合テスト（新規作成）
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 要件に基づいて技術スタックを決定し、既存のコードパターンを理解する

**出力**: `specs/SPEC-8efcbf19/research.md`

### 調査項目

1. **既存のコードベース分析**
   - ✅ 技術スタック確認: TypeScript 5.8+, Bun 1.0+, Vitest 2.1+
   - ✅ launchClaudeCode関数の現在の実装（src/claude.ts:17-134）
   - ✅ execa環境変数設定の使用パターン
   - ✅ 既存のエラーハンドリングパターン（ClaudeError）

2. **技術的決定**
   - **決定1**: rootユーザー検出方法 → `process.getuid() === 0`を使用（POSIX標準API）
   - **決定2**: 環境変数設定方法 → execaの`env`オプションを使用
   - **決定3**: 警告メッセージ表示 → 既存のchalkパターンに従う（yellow/blue）
   - **決定4**: エラーハンドリング → try-catchでprocess.getuid()の非存在をハンドリング

3. **制約と依存関係**
   - **制約1**: POSIXシステムのみ対応（Windows非対応は許容）
   - **制約2**: IS_SANDBOX=1は非公式環境変数（将来変更リスクあり）
   - **依存関係1**: Claude Code CLIのIS_SANDBOX=1サポート（コミュニティ確認済み）

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-8efcbf19/data-model.md` (N/A - データモデルなし)
- `specs/SPEC-8efcbf19/quickstart.md`

### 1.1 データモデル設計

**ファイル**: `data-model.md` - **N/A**

この機能にはデータモデルが不要：
- 環境変数の設定のみ（永続化なし）
- エンティティや状態管理なし
- 既存関数のロジック追加のみ

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの簡潔なガイド：
- 機能概要とユースケース
- 実装箇所（src/claude.ts）
- テスト方法（Docker環境での検証）
- トラブルシューティング（Windows環境、IS_SANDBOX=1動作しない場合）

### 1.3 契約/インターフェース

**ディレクトリ**: `contracts/` - **N/A**

この機能には外部APIや契約がない：
- 内部関数の修正のみ
- 外部インターフェースの変更なし

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/[SPEC-xxxxxxxx]/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装：

1. **P1: Docker/root環境での自動承認実行**
   - rootユーザー検出ロジック
   - IS_SANDBOX=1環境変数設定
   - 基本動作確認

2. **P2: セキュリティ警告の表示**
   - 警告メッセージの追加
   - 既存のchalkパターンに従った実装

3. **P3: 非rootユーザーでの既存動作維持**
   - エラーハンドリング（try-catch）
   - 後方互換性の検証

### 独立したデリバリー

各ユーザーストーリーは独立して実装・テスト・デプロイ可能：

- **P1完了**: Docker/root環境でClaude Codeが起動可能 → MVP達成
- **P2追加**: ユーザーへの警告表示でセキュリティ意識向上 → UX改善
- **P3追加**: 非root環境での完全な後方互換性 → 完全な機能

## テスト戦略

### ユニットテスト

**範囲**: launchClaudeCode関数のロジック

**テストファイル**: `tests/unit/claude.test.ts`

**テストケース**:
1. rootユーザー検出が正常に動作する
2. skipPermissions=true + root環境でIS_SANDBOX=1が設定される
3. skipPermissions=false + root環境でIS_SANDBOX=1が設定されない
4. 非root環境でIS_SANDBOX=1が設定されない
5. process.getuid()が存在しない環境でエラーが発生しない
6. 警告メッセージが適切に表示される

**モック**:
- `process.getuid()`: rootユーザーと非rootユーザーをシミュレート
- `execa`: 環境変数設定を検証
- `console.log`: 警告メッセージ表示を検証

### 統合テスト

**範囲**: Docker環境での実際の動作確認

**テストファイル**: `tests/integration/root-sandbox.test.ts`

**テストケース**:
1. Docker環境（root）でClaude Codeが正常に起動する
2. 非root環境で既存の動作が維持される
3. IS_SANDBOX=1が実際にClaude Codeに渡される

**環境**:
- Docker container (node:22イメージ)
- 実際のexeca呼び出し

### エンドツーエンドテスト

**範囲**: 実際のユーザーフロー

**手動テスト手順**:
1. Docker環境でclaude-worktreeを起動
2. "Skip permission checks?"でYesを選択
3. Claude Codeが正常に起動することを確認
4. Permission promptが表示されないことを確認

**自動化**: E2Eテストは手動検証を優先（Docker環境のセットアップが複雑なため）

### カバレッジ目標

- ユニットテスト: 95%以上（新規コードのみ）
- 統合テスト: 主要な受け入れシナリオをカバー
- E2Eテスト: 手動検証で実施

## リスクと緩和策

### 技術的リスク

1. **IS_SANDBOX=1が将来のClaude Codeバージョンで動作しなくなる**
   - **確率**: 中（非公式環境変数のため）
   - **影響**: 高（機能が完全に動作しなくなる）
   - **緩和策**:
     - README.mdに既知の制限として明記
     - GitHub Issue #3490を定期的に監視
     - 動作しなくなった場合のフォールバック（エラーメッセージ表示）
     - コミュニティとの連携でAnthropicへのフィードバック

2. **process.getuid()がすべての環境で利用可能でない**
   - **確率**: 高（Windows環境では確実に発生）
   - **影響**: 低（フォールバックで既存動作維持）
   - **緩和策**:
     - try-catchで例外をハンドリング
     - Windows環境では既存の動作を維持
     - ドキュメントに対応プラットフォームを明記

3. **root環境以外での誤動作**
   - **確率**: 低（テストでカバー）
   - **影響**: 中（既存ユーザーへの影響）
   - **緩和策**:
     - ユニットテストで非root環境をカバー
     - 後方互換性テストを実施
     - 段階的なリリース（カナリアリリース）

### 依存関係リスク

1. **Claude Code CLIのアップデート**
   - **確率**: 中（定期的にアップデートされる）
   - **影響**: 高（IS_SANDBOX=1サポートが削除される可能性）
   - **緩和策**:
     - CI/CDで定期的に動作確認
     - Claude Codeのリリースノートを監視
     - ユーザーへの事前通知体制

2. **execa APIの変更**
   - **確率**: 低（安定したAPIを使用）
   - **影響**: 中（環境変数設定が動作しなくなる）
   - **緩和策**:
     - セマンティックバージョニングで互換性維持
     - ロックファイル（bun.lockb）で固定
     - アップデート前のテスト実施

## 次のステップ

1. ✅ **フェーズ0完了**: 調査と技術スタック決定
   - research.md作成完了
   - 技術的決定（rootユーザー検出、環境変数設定、警告メッセージ）
   - 制約と依存関係の明確化

2. ✅ **フェーズ1完了**: 設計とアーキテクチャ定義
   - quickstart.md作成完了
   - data-model.md: N/A（データモデル不要）
   - contracts/: N/A（外部API不要）

3. ⏭️ **フェーズ2**: `/speckit.tasks` を実行してタスクを生成
   - 実装タスクの詳細化
   - 依存関係の明確化
   - 見積もりと優先順位付け

4. ⏭️ **実装**: `/speckit.implement` で実装を開始
   - src/claude.tsの修正
   - テストの追加（unit/integration）
   - ドキュメントの更新

5. ⏭️ **検証とリリース**
   - Docker環境での動作確認
   - 非root環境での後方互換性確認
   - コミット＆プッシュ
