# 実装計画: Claude Code / Codex CLI 対応の対話型Gitワークツリーマネージャー

**仕様ID**: `SPEC-473b3d47` | **日付**: 2025-10-24 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-473b3d47/spec.md` からの機能仕様

**注**: この計画は既存実装（v0.6.1）のアーキテクチャとパターンを文書化したものです。

## 概要

`@akiojin/claude-worktree`は、Claude Code / Codex CLI と統合された対話型Gitワークツリーマネージャーです。TypeScriptで実装され、Bun 1.0.0+ 環境で動作します（必要に応じてNode.js 18+を併用可能）。主要な機能は以下の8つのユーザーストーリーに分類されます：

1. **対話型ブランチ選択とワークツリー自動作成** (P1)
2. **スマートブランチ作成ワークフロー** (P1)
3. **セッション管理と継続機能** (P2)
4. **マージ済みPRの自動クリーンアップ** (P2)
5. **ワークツリー管理とライフサイクル操作** (P2)
6. **リリース管理とGit Flowサポート** (P3)
7. **AIツール統合と実行モード管理** (P1)
8. **変更管理と開発セッション終了処理** (P2)

アーキテクチャは**レイヤードアーキテクチャ**を採用し、UI層、サービス層、リポジトリ層に分離されています。

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.8.3 / Bun 1.0.0+
**ランタイム**: Bun 1.0.0+（開発・実行）※必要に応じてNode.js 18.0.0+を併用可能
**主要な依存関係**:
- `@inquirer/prompts` (^6.0.1) - 対話型プロンプトとユーザー入力
- `chalk` (^5.4.1) - コンソール出力の色付け
- `execa` (^9.6.0) - 外部コマンド実行（Git, GitHub CLI, AIツール）
- `string-width` (^7.2.0) - テーブル表示の幅計算

**ストレージ**:
- ファイルシステム（`.config/claude-worktree-session.json`）- セッション管理
- Git（ワークツリーメタデータ）- Git標準機能

**テスト**: 現在テストフレームワークは未実装（実装推奨: Vitest または Jest）

**ターゲットプラットフォーム**: Linux / macOS / Windows（WSL）

**プロジェクトタイプ**: CLIツール（単一パッケージ、NPMグローバルインストール対応）

**パフォーマンス目標**:
- ブランチ選択からAIツール起動まで5秒以内
- 新規ブランチ作成からワークツリー作成まで10秒以内
- セッション継続（-cオプション）3秒以内

**制約**:
- Git 2.5+（worktree機能要件）
- Bun 1.0.0+必須
- AIツール（Claude Code / Codex CLI）いずれか1つ以上インストール必要
- GitHub統合はGitHub CLI（gh）必須

**スケール/範囲**:
- 想定ブランチ数: 100ブランチ程度
- 想定ワークツリー数: 10-20同時管理
- 単一リポジトリ専用（モノレポ対応なし）

## 原則チェック

*ゲート: 既存実装のレビューとリファクタリング前に確認*

### プロジェクト原則（CLAUDE.mdより）

✅ **シンプルさの極限追求**
- 現状: 実装はシンプルで理解しやすい構造
- 改善点: 一部の関数が長い（特にindex.ts）→ リファクタリング推奨

✅ **ユーザビリティと開発者体験の品質**
- 現状: 直感的なUI、エラーメッセージが明確
- 改善点: ヘルプメッセージとドキュメントの充実

✅ **CLI操作の直感性と効率性**
- 現状: 対話型メニュー、キーボードナビゲーション対応
- 改善点: ショートカットキーの追加検討

⚠️ **完了条件（エラーゼロ）**
- 現状: 基本的なエラーハンドリングあり
- 改善点: エッジケースでのエラーハンドリング強化

⚠️ **テストファースト**
- **ゲート違反**: テストが存在しない
- 正当化: 既存実装のため、今後テストを追加する必要あり
- アクション: Phase 2でテスト戦略とテストタスクを定義

⚠️ **コードクオリティ（markdownlint, commitlint）**
- 現状: 設定ファイルは存在（.markdownlint.json）
- 改善点: CIでのlint自動実行

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-473b3d47/
├── spec.md              # 機能仕様書（完了）
├── plan.md              # このファイル（/speckit.plan出力）
├── data-model.md        # Phase 1出力（/speckit.plan）
├── quickstart.md        # Phase 1出力（/speckit.plan）
├── tasks.md             # Phase 2出力（/speckit.tasks - 別コマンド）
└── checklists/
    └── requirements.md  # 仕様品質チェックリスト（完了）
```

### ソースコード（リポジトリルート）

```text
src/
├── index.ts                    # メインエントリーポイント、アプリケーションループ
├── git.ts                      # Git操作（ブランチ、コミット、stash）
├── worktree.ts                 # ワークツリー作成・管理・削除
├── claude.ts                   # Claude Code起動・統合
├── codex.ts                    # Codex CLI起動・統合
├── github.ts                   # GitHub CLI統合（PR検出）
├── claude-history.ts           # Claude Code履歴管理（将来機能）
├── utils.ts                    # ユーティリティ、エラーハンドリング
├── config/
│   ├── index.ts                # セッション永続化
│   └── constants.ts            # 定数定義
├── ui/
│   ├── prompts.ts              # 対話型プロンプト定義
│   ├── display.ts              # コンソール出力フォーマット
│   ├── table.ts                # ブランチテーブル生成
│   └── types.ts                # TypeScript型定義
├── services/                   # サービス層（未使用、将来のリファクタリング用）
│   ├── git.service.ts
│   ├── worktree.service.ts
│   └── github.service.ts
└── repositories/               # リポジトリ層（未使用、将来のリファクタリング用）
    ├── git.repository.ts
    ├── worktree.repository.ts
    └── github.repository.ts

bin/
└── claude-worktree.js          # 実行可能ラッパー

dist/                            # TypeScriptコンパイル出力
└── [compiled .js files]
```

**注**: services/とrepositories/ディレクトリは存在しますが、現在使用されていません。将来のリファクタリングでレイヤードアーキテクチャを完全実装する準備として配置されています。

## 既存アーキテクチャの詳細

### アーキテクチャパターン

**現在のアーキテクチャ**: **モノリシックな手続き型 + 部分的レイヤー分離**

1. **メインループ**: `index.ts`
   - アプリケーションのエントリーポイント
   - ユーザー入力処理とメニューナビゲーション
   - すべての主要フローを調整（1000行超）

2. **機能モジュール**: 独立した機能ファイル
   - `git.ts`: Gitコマンドのラッパー関数群
   - `worktree.ts`: ワークツリー操作
   - `claude.ts` / `codex.ts`: AIツール起動
   - `github.ts`: GitHub CLI統合

3. **UI層**: `ui/` ディレクトリ
   - 対話型プロンプト、テーブル表示、メッセージ出力を分離
   - 良い分離が実現されている

4. **未使用のレイヤー**: `services/` と `repositories/`
   - ファイルは存在するが、まだインポート・使用されていない
   - 将来のリファクタリングの準備

### データフロー

```text
User Input
    ↓
index.ts (Main Loop)
    ↓
ui/prompts.ts (User Interaction)
    ↓
index.ts (Handler Functions)
    ↓
git.ts / worktree.ts / github.ts (Git Operations)
    ↓
execa (External Commands: git, gh, claude, codex)
    ↓
ui/display.ts (Output Formatting)
    ↓
Console Output
```

### 技術的決定

#### 決定1: execaによる外部コマンド実行

**決定**: `execa`ライブラリを使用してすべての外部コマンド（git, gh, claude, codex）を実行

**理由**:
- Promiseベースで非同期処理がシンプル
- TypeScript型定義が充実
- エラーハンドリングが優れている

#### 決定2: @inquirer/promptsによる対話型UI

**決定**: `@inquirer/prompts`を使用した対話型メニューとプロンプト

**理由**:
- CLIで最も広く使われているUIライブラリ
- 豊富なプロンプトタイプ
- TypeScript完全サポート

#### 決定3: Git Flowに準拠したブランチ管理

**決定**: feature/hotfix/releaseの3種類のブランチタイプをサポート

**理由**:
- 業界標準のブランチングモデル
- チーム開発での一貫性

## 次のステップ

1. ✅ Phase 0完了: 既存アーキテクチャの分析
2. ⏭️ Phase 1: data-model.mdとquickstart.mdの生成
3. ⏭️ Phase 2: `/speckit.tasks`でタスクリスト生成
