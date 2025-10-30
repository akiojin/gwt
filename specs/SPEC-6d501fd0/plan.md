# 実装計画: Ink UI内蔵仮想ターミナル機能

**仕様ID**: `SPEC-6d501fd0` | **日付**: 2025-01-26 | **仕様書**: [spec.md](spec.md)
**入力**: `/specs/SPEC-6d501fd0/spec.md` からの機能仕様

**注**: このテンプレートは `/speckit.plan` コマンドによって記入されます。実行ワークフローについては `.specify/templates/commands/plan.md` を参照してください。

## 概要

既存のInk UIベースのCLIアプリケーション内に、AIツール（Claude Code/Codex CLI）を実行するための仮想ターミナル画面を追加します。現在、AIツール起動時にInk UIが終了してしまい、コンテキスト情報（ブランチ名、ツール名、実行モード）が失われる問題を解決します。

**主要要件**:
- TerminalScreenをInk UIフロー内に追加
- ヘッダーにブランチ名、ツール名、モード、worktreeパスを表示
- AIツールとの双方向通信を実現
- Ctrl+C（中断）、Ctrl+Z（一時停止/再開）、Ctrl+S（ログ保存）、F11（全画面）のサポート
- AIツール終了後、自動的にブランチ一覧画面に戻る

**技術的アプローチ**:
- PTY（疑似端末）を使用してAIツールを起動
- Ink UIのuseInputフックでrawモードの入力を処理
- プロセス制御用のPTYマネージャーを実装
- ログ保存機能を追加

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.8+ (ES Modules)
**ランタイム**: Bun 1.0+（Node.js互換）
**主要な依存関係**:
- Ink 6.3.1（React for CLIs）
- React 19.2.0
- execa 9.6.0（プロセス実行）
- node-pty（要追加 - 疑似端末）
**ストレージ**: ファイルシステム（ログ保存用 - `.logs/` ディレクトリ）
**テスト**: vitest, ink-testing-library
**ターゲットプラットフォーム**: macOS, Linux, Windows（Bun対応環境）
**プロジェクトタイプ**: 単一CLIアプリケーション
**パフォーマンス目標**:
- キー入力レイテンシ < 100ms
- 出力表示レイテンシ < 200ms
- 起動時間 < 5秒
**制約**:
- 既存のInk UIアーキテクチャとの統合
- 既存の画面遷移フローを維持
- すべてのプラットフォームで動作
**スケール/範囲**: 単一ユーザー、1つのターミナルセッション

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

**注**: constitution.md がテンプレートのままのため、プロジェクト原則は未定義です。以下の一般的なベストプラクティスを適用します：

- ✅ **TDDアプローチ**: すべての新機能はテストファーストで実装
- ✅ **既存コードパターンの踏襲**: Ink UIの既存Screen実装パターンに従う
- ✅ **シンプルさ**: 必要最小限の抽象化、YAGNI原則
- ✅ **ユーザー価値優先**: P1（コンテキスト情報付きAIツール実行）を最初に実装

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-6d501fd0/
├── spec.md              # 機能仕様（完了）
├── plan.md              # このファイル（/speckit.plan コマンド出力）
├── research.md          # フェーズ0出力（/speckit.plan コマンド）
├── data-model.md        # フェーズ1出力（/speckit.plan コマンド）
├── quickstart.md        # フェーズ1出力（/speckit.plan コマンド）
├── contracts/           # フェーズ1出力（N/A - UIコンポーネントのため不要）
└── tasks.md             # フェーズ2出力（/speckit.tasks コマンド）
```

### ソースコード（リポジトリルート）

```text
src/
├── ui/
│   ├── components/
│   │   ├── screens/
│   │   │   ├── TerminalScreen.tsx          # 新規: ターミナル画面コンポーネント
│   │   │   ├── BranchListScreen.tsx        # 既存
│   │   │   └── ...
│   │   ├── parts/
│   │   │   ├── TerminalOutput.tsx          # 新規: ターミナル出力表示コンポーネント
│   │   │   └── ...
│   │   └── App.tsx                         # 修正: TerminalScreen統合
│   ├── hooks/
│   │   ├── usePtyProcess.ts                # 新規: PTYプロセス管理フック
│   │   └── ...
│   └── types.ts                            # 修正: TerminalSession型定義追加
├── pty/
│   ├── PtyManager.ts                       # 新規: PTY管理クラス
│   └── types.ts                            # 新規: PTY関連型定義
├── claude.ts                               # 修正: PTY対応
├── codex.ts                                # 修正: PTY対応
└── index.ts                                # 修正: フロー変更

tests/ または src/**/__tests__/
└── （対応するテストファイル）
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 要件に基づいて技術スタックを決定し、既存のコードパターンを理解する

**出力**: `specs/SPEC-6d501fd0/research.md`

### 調査項目

1. **既存のコードベース分析**
   - 現在のInk UI画面実装パターン（BranchListScreen, AIToolSelectorScreen等）
   - 現在のAIツール起動方法（launchClaudeCode, launchCodexCLI関数）
   - 画面遷移フロー（useScreenStateフック）
   - 既存のテストパターン（vitest, ink-testing-library）

2. **技術的決定**
   - **PTY（疑似端末）ライブラリの選択**
     - 候補: node-pty（公式推奨）
     - Bun環境での互換性確認
     - Windows/macOS/Linuxのクロスプラットフォーム対応
   - **Ink UIでのraw入力処理**
     - useInputフックの制限と拡張方法
     - キーボード入力をPTYにパススルーする方法
   - **プロセス制御（SIGSTOP/SIGCONT）**
     - Ctrl+Zでの一時停止/再開の実装
     - プラットフォーム依存性の確認
   - **ログ保存機能**
     - ストリーミング出力のバッファリング方法
     - ファイル保存時のエラーハンドリング

3. **制約と依存関係**
   - 既存のexecaベースの起動ロジックをPTYベースに移行
   - 既存の画面遷移フローを維持
   - 既存のテスト戦略（TDD）を継続

### 調査タスク

- [ ] node-ptyのBun互換性を検証
- [ ] Ink UIでのPTY出力表示パターンを調査
- [ ] 既存Screenコンポーネントの実装パターンを分析
- [ ] プラットフォーム別のPTY動作を確認
- [ ] ログバッファリングのベストプラクティスを調査

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-6d501fd0/data-model.md`
- `specs/SPEC-6d501fd0/quickstart.md`
- `specs/SPEC-6d501fd0/contracts/` （UIコンポーネントのため不要）

### 1.1 データモデル設計

**ファイル**: `data-model.md`

主要なエンティティとその関係を定義：

**TerminalSession**:
- branch: string
- tool: 'claude-code' | 'codex-cli'
- mode: 'normal' | 'continue' | 'resume'
- worktreePath: string
- startTime: Date
- endTime?: Date

**TerminalOutput**:
- sessionId: string
- timestamp: Date
- content: string
- isError: boolean

**PtyProcess**:
- pid: number
- ptyInstance: IPty
- status: 'running' | 'paused' | 'stopped'

**LogFile**:
- filePath: string
- savedAt: Date
- size: number

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの簡潔なガイド：
- セットアップ手順（`bun install`、`bun run build`）
- 開発ワークフロー（TDD、テスト実行）
- TerminalScreenの統合方法
- PTYマネージャーの使用例
- トラブルシューティング（PTYビルドエラー、プラットフォーム固有の問題）

### 1.3 契約/インターフェース（該当する場合）

**ディレクトリ**: `contracts/`

**注**: UIコンポーネントのため、外部APIやイベントスキーマは不要です。内部コンポーネントインターフェースはTypeScriptの型定義で管理します。

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-6d501fd0/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装：
1. **P1**: コンテキスト情報付きAIツール実行
   - TerminalScreen基本実装
   - PTYマネージャー基本機能
   - ヘッダー表示
   - 双方向通信
2. **P2**: AIツール実行の制御とログ保存
   - Ctrl+C（中断）
   - Ctrl+Z（一時停止/再開）
   - Ctrl+S（ログ保存）
3. **P3**: 全画面表示による視認性向上
   - F11（全画面切替）

### 独立したデリバリー

各ユーザーストーリーは独立して実装・テスト・デプロイ可能：
- P1完了 → 基本的なターミナル機能が使用可能
- P2完了 → 高度な制御機能が追加
- P3完了 → UI体験の向上

## テスト戦略

### ユニットテスト

- **PtyManager.ts**: PTYのライフサイクル管理（spawn, kill, pause, resume）
- **usePtyProcess.ts**: React フックのロジック
- **TerminalScreen.tsx**: コンポーネントのレンダリングとイベントハンドリング
- **TerminalOutput.tsx**: 出力表示ロジック

### 統合テスト

- **TerminalScreen統合**: 画面遷移フロー（BranchList → AIToolSelector → ExecutionModeSelector → TerminalScreen → BranchList）
- **PTY + AIツール統合**: 実際のClaude Code/Codex CLI起動と双方向通信

### エンドツーエンドテスト

- **受け入れシナリオ**: spec.mdに定義された各ユーザーストーリーの受け入れシナリオをテスト
- **エッジケース**: AIツール起動失敗、PTY利用不可、ログ保存失敗など

### パフォーマンステスト

- キー入力レイテンシ測定（< 100ms）
- 出力表示レイテンシ測定（< 200ms）
- ログ保存速度測定（1MB < 1秒）

## リスクと緩和策

### 技術的リスク

1. **node-ptyのBun互換性**
   - **説明**: node-ptyがBun環境で正常に動作しない可能性
   - **緩和策**:
     - 事前に互換性を検証（Phase 0）
     - 動作しない場合、execaベースのフォールバック実装を用意
     - Windows環境でのビルド問題に備えてドキュメント整備

2. **PTYのクロスプラットフォーム動作**
   - **説明**: Windows/macOS/Linuxで動作が異なる可能性
   - **緩和策**:
     - 各プラットフォームでのテストを実施
     - プラットフォーム固有の処理を条件分岐で実装
     - エラーハンドリングを充実

3. **Ink UIでのraw入力処理**
   - **説明**: useInputフックがrawモードをサポートしていない可能性
   - **緩和策**:
     - useInputの制限を事前に調査
     - 必要に応じてInkの低レベルAPIを使用
     - 代替実装の検討

### 依存関係リスク

1. **node-ptyの保守状況**
   - **説明**: node-ptyがアクティブに保守されているか
   - **緩和策**:
     - ライブラリの最終更新日とissue状況を確認
     - 代替ライブラリ（node-pty-prebuilt等）を調査

2. **既存画面遷移フローへの影響**
   - **説明**: TerminalScreen追加により既存フローが破壊される可能性
   - **緩和策**:
     - 既存テストを全て実行して回帰を確認
     - TerminalScreenを既存フローに最小限の変更で統合

## 次のステップ

1. ⏭️ フェーズ0: 調査と技術スタック決定（research.md生成）
2. ⏭️ フェーズ1: 設計とアーキテクチャ定義（data-model.md, quickstart.md生成）
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
