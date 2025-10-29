# 実装計画: カスタムAIツール対応機能

**仕様ID**: `SPEC-30f6d724` | **日付**: 2025-10-28 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-30f6d724/spec.md` からの機能仕様

## 概要

Claude Code/Codex CLI以外のカスタムAIツールを`~/.claude-worktree/tools.json`に定義して利用可能にする機能。ユーザーは設定ファイルでツールの実行方法（絶対パス、bunx、コマンド名）、デフォルト引数、モード別引数、権限スキップ引数、環境変数を定義でき、UIから選択して起動できる。既存のビルトインツール（Claude Code, Codex CLI）との統合を保ちながら、拡張性を提供する。

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.8.3 (target: ES2022)
**ランタイム**: Bun >=1.0.0
**主要な依存関係**:
- React 19.2.0 (UI)
- Ink 6.3.1 (CLI UI framework)
- execa 9.6.0 (プロセス実行)
- chalk 5.4.1 (色付き出力)

**ストレージ**: ファイルシステム (JSON設定ファイル、~/.claude-worktree/tools.json)
**テスト**: Vitest 2.1.8, @testing-library/react 16.3.0, ink-testing-library 4.0.0
**ターゲットプラットフォーム**: macOS/Linux/Windows (Node.js互換環境)
**プロジェクトタイプ**: CLIアプリケーション (単一プロジェクト)
**パフォーマンス目標**: 設定読み込み <100ms, ツール起動 <500ms
**制約**:
- 既存のビルトインツール（Claude Code, Codex CLI）との後方互換性を100%維持
- bunx実行環境が必須
- JSON手動編集ベース（GUI不要）

**スケール/範囲**: カスタムツール ~10種類、設定ファイルサイズ <10KB

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

### シンプルさの極限追求

✅ **合格**: 設定ファイルは単純なJSON形式、新規ファイルは最小限（tools.ts, launcher.ts のみ）、既存コード変更は必要最小限

### ユーザビリティと開発者体験の品質

✅ **合格**: UIは既存パターンを踏襲、設定ファイルは明確なスキーマ、エラーメッセージは具体的

### 設計文書とソースコードの分離

✅ **合格**: このplan.mdおよびspec.mdは設計のみ、ソースコードは含まない

### Spec Kit SDD/TDD絶対遵守

✅ **合格**: spec.md承認済み、plan.md作成中、tasks.md作成後にTDD実施予定

### 既存ファイル優先メンテナンス

✅ **合格**: 新規ファイルは2つのみ、既存の設定システム・UI・起動ロジックは拡張で対応

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-30f6d724/
├── spec.md              # 機能仕様（完成）
├── plan.md              # このファイル（作成中）
├── research.md          # フェーズ0出力（次のステップ）
├── data-model.md        # フェーズ1出力
├── quickstart.md        # フェーズ1出力
├── contracts/           # フェーズ1出力（TypeScript型定義）
│   └── types.ts         # ToolsConfig, CustomAITool型定義
└── tasks.md             # フェーズ2出力（/speckit.tasks コマンド）
```

### ソースコード（リポジトリルート）

```text
src/
├── config/
│   ├── index.ts         # 既存: AppConfig, SessionData管理
│   └── tools.ts         # 新規: カスタムツール設定管理
├── launcher.ts          # 新規: 汎用AIツール起動機能
├── claude.ts            # 既存: 修正（launcherを使用）
├── codex.ts             # 既存: 修正（launcherを使用）
├── index.ts             # 既存: 修正（カスタムツール対応）
└── ui/
    └── components/
        └── screens/
            ├── AIToolSelectorScreen.tsx  # 既存: 修正（動的ツール一覧）
            └── ExecutionModeSelectorScreen.tsx  # 既存: 確認（変更なし）

tests/
├── unit/
│   ├── config/
│   │   └── tools.test.ts         # 新規: tools.ts のテスト
│   └── launcher.test.ts          # 新規: launcher.ts のテスト
└── integration/
    └── custom-tool-launch.test.ts  # 新規: E2Eテスト
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 要件に基づいて技術スタックを決定し、既存のコードパターンを理解する

**出力**: `specs/SPEC-30f6d724/research.md`

### 調査項目

1. **既存のコードベース分析**
   - 現在の設定管理パターン (`src/config/index.ts`)
   - 既存のAIツール起動ロジック (`src/claude.ts`, `src/codex.ts`)
   - UIコンポーネントの構造 (`src/ui/components/screens/AIToolSelectorScreen.tsx`)
   - セッション管理の実装 (`SessionData`, `saveSession`, `loadSession`)
   - プロセス実行パターン (execa使用方法)

2. **技術的決定**
   - **設定ファイル形式**: JSON（既存のconfig.jsonと統一）
   - **設定ファイルパス**: `~/.claude-worktree/tools.json`（既存の`~/.config/claude-worktree/`との一貫性）
   - **型定義**: TypeScriptインターフェース（ToolsConfig, CustomAITool）
   - **バリデーション**: Zod不使用、シンプルな手動検証（依存関係最小化）
   - **コマンド解決**: which/where コマンド経由（セキュリティ考慮）

3. **制約と依存関係**
   - bunx実行環境（Bun 1.0+）
   - 既存のexecaライブラリ（バージョン9.6.0）
   - Reactコンポーネントパターン（Ink 6.3.1）
   - ファイルシステムAPI（Node.js fs/promises）

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-30f6d724/data-model.md`
- `specs/SPEC-30f6d724/quickstart.md`
- `specs/SPEC-30f6d724/contracts/types.ts`

### 1.1 データモデル設計

**ファイル**: `data-model.md`

主要なエンティティとその関係を定義：

1. **ToolsConfig**
   - version: string
   - customTools: CustomAITool[]
   - 関係: 1対多（1つのToolsConfigが複数のCustomAIToolを持つ）

2. **CustomAITool**
   - id: string（一意識別子）
   - displayName: string
   - icon?: string
   - type: 'path' | 'bunx' | 'command'
   - command: string
   - defaultArgs?: string[]
   - modeArgs: ModeArgs
   - permissionSkipArgs?: string[]
   - env?: Record<string, string>

3. **ModeArgs**
   - normal?: string[]
   - continue?: string[]
   - resume?: string[]

4. **SessionData（拡張）**
   - 既存フィールド: lastWorktreePath, lastBranch, timestamp, repositoryRoot
   - 新規フィールド: lastUsedTool?: string

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの簡潔なガイド：

1. **セットアップ手順**
   - `~/.claude-worktree/tools.json` の作成方法
   - 設定例（3つの実行タイプ）

2. **開発ワークフロー**
   - カスタムツールの追加手順
   - テストの実行方法
   - デバッグ方法（DEBUG_CONFIG=true）

3. **よくある操作**
   - 新しいツールの登録
   - モード別引数の設定
   - 環境変数の設定

4. **トラブルシューティング**
   - JSON構文エラー
   - コマンドが見つからない
   - 権限エラー

### 1.3 契約/インターフェース

**ディレクトリ**: `contracts/`

TypeScript型定義を定義：

**ファイル**: `contracts/types.ts`

```typescript
// カスタムツール設定の型定義
export interface ToolsConfig {
  version: string;
  customTools: CustomAITool[];
}

export type ToolExecutionType = 'path' | 'bunx' | 'command';

export interface CustomAITool {
  id: string;
  displayName: string;
  icon?: string;
  type: ToolExecutionType;
  command: string;
  defaultArgs?: string[];
  modeArgs: ModeArgs;
  permissionSkipArgs?: string[];
  env?: Record<string, string>;
}

export interface ModeArgs {
  normal?: string[];
  continue?: string[];
  resume?: string[];
}

// 統合型（ビルトイン + カスタム）
export interface AIToolConfig {
  id: string;
  displayName: string;
  icon?: string;
  isBuiltin: boolean;
  customConfig?: CustomAITool;
}
```

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-30f6d724/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装：

1. **P1: カスタムAIツールの設定**
   - `src/config/tools.ts` 実装
   - 設定読み込み・検証機能
   - テスト作成

2. **P1: カスタムツールの選択と起動**
   - `src/launcher.ts` 実装
   - 3つの実行タイプ（path/bunx/command）対応
   - `AIToolSelectorScreen` 動的ツール一覧対応
   - テスト作成

3. **P2: 実行モードと権限スキップ**
   - モード別引数の結合ロジック
   - 権限スキップ引数の追加
   - テスト作成

4. **P3: 環境変数の設定**
   - env フィールドの処理
   - プロセス起動時の環境変数設定
   - テスト作成

5. **P3: セッション管理とツール情報の保存**
   - `SessionData` 拡張
   - ツールID保存・復元
   - テスト作成

### 独立したデリバリー

各ユーザーストーリーは独立して実装・テスト・デプロイ可能：

- **P1-1完了** → 設定ファイルが読み込め、検証できる（デプロイ可能なMVP）
- **P1-2追加** → カスタムツールが起動できる（拡張MVP）
- **P2追加** → 実行モードと権限スキップが使える（機能拡張）
- **P3追加** → 環境変数とセッション管理が使える（完全な機能）

## テスト戦略

### ユニットテスト

**範囲**: 各モジュールの個別機能

- **`src/config/tools.ts`**
  - `loadToolsConfig()`: ファイル存在/不存在、JSON構文エラー、検証エラー
  - `validateToolConfig()`: 必須フィールド、type値、id重複
  - `getToolById()`: 存在/不存在ケース
  - `getAllTools()`: ビルトイン + カスタムの統合

- **`src/launcher.ts`**
  - `launchCustomAITool()`: 3つの実行タイプ別テスト
  - 引数結合ロジック（defaultArgs + modeArgs + permissionSkipArgs + extraArgs）
  - 環境変数設定
  - エラーハンドリング（コマンド不在、権限エラー）

### 統合テスト

**範囲**: コンポーネント間の連携

- **設定読み込み → ツール一覧表示**
  - `getAllTools()` → `AIToolSelectorScreen` の表示確認

- **ツール選択 → 起動**
  - `AIToolSelectorScreen` → `launchCustomAITool()` の連携
  - 実行モード選択 → 引数結合の確認

- **セッション保存 → 復元**
  - ツール使用 → `saveSession()` → `loadSession()` → ツールID復元

### エンドツーエンドテスト

**範囲**: ユーザーシナリオ全体

- **シナリオ1**: カスタムツール登録 → 起動（normalモード）
- **シナリオ2**: カスタムツール登録 → 起動（continueモード、権限スキップあり）
- **シナリオ3**: 設定ファイル不在 → ビルトインツールのみ表示
- **シナリオ4**: JSON構文エラー → エラーメッセージ表示

### パフォーマンステスト

- 設定読み込み時間: <100ms（10個のカスタムツール）
- ツール起動時間: <500ms（bunx実行含む）

## リスクと緩和策

### 技術的リスク

1. **bunx実行の遅延**
   - **説明**: bunxでのパッケージ実行が初回ダウンロード時に遅い
   - **緩和策**: ローディングインジケーター表示、事前にパッケージインストールを推奨

2. **コマンドパス解決の失敗**
   - **説明**: PATH環境変数から探すコマンドが見つからない
   - **緩和策**: which/whereコマンドで事前確認、明確なエラーメッセージ表示

3. **JSON構文エラーのユーザビリティ**
   - **説明**: 手動編集のためJSON構文エラーが発生しやすい
   - **緩和策**: 詳細なエラーメッセージ（行番号、エラー内容）、サンプル設定ファイル提供

### 依存関係リスク

1. **既存コードとの互換性**
   - **説明**: Claude Code/Codex CLIの既存動作に影響
   - **緩和策**: 既存テストの100%パス、リグレッションテスト実施

2. **セッションデータスキーマ変更**
   - **説明**: SessionDataへのフィールド追加が既存セッションに影響
   - **緩和策**: オプショナルフィールド（lastUsedTool?: string）、後方互換性維持

## 次のステップ

1. ⏭️ フェーズ0: `research.md` を作成（既存コード分析、技術決定の詳細化）
2. ⏭️ フェーズ1: `data-model.md`, `quickstart.md`, `contracts/types.ts` を作成
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
