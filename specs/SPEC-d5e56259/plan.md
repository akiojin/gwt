# 実装計画: Web UI機能の追加

**仕様ID**: `SPEC-d5e56259` | **日付**: 2025-11-10 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-d5e56259/spec.md` からの機能仕様

## 概要

claude-worktreeに`serve`サブコマンドを追加し、ローカルWebサーバーを起動してブラウザからWorktree管理とAI Tool起動を可能にする。既存のCLI機能を維持しつつ、Web UIをオプションとして提供する。PTY（疑似端末）とWebSocketを使用して、ブラウザ内でターミナル風のインタラクティブな操作を実現する。

## 技術コンテキスト

**言語/バージョン**: TypeScript 5.8.x, Node.js 18+ / Bun 1.0+
**主要な依存関係**:
- バックエンド: Fastify 5, @fastify/websocket, @fastify/static, node-pty 1.0+
- フロントエンド: Vite 6, React 19, xterm.js 5.x, @tanstack/react-query, Zustand
**ストレージ**: ファイルシステム（既存の`~/.claude-worktree/tools.json`, セッション履歴）
**テスト**: Vitest 4.0.8（既存）, @testing-library/react 16.3.0, Playwright（E2E）
**ターゲットプラットフォーム**: Linux/macOS/Windows（ローカル開発環境）
**プロジェクトタイプ**: Web - バックエンド（Fastify）+ フロントエンド（React SPA）
**パフォーマンス目標**:
- サーバー起動: 5秒以内
- ブランチ一覧表示: 3秒以内（1000ブランチ）
- ターミナルI/O遅延: 100ms以内
**制約**:
- 既存CLI機能を破壊しない（デュアルUI対応）
- ローカル環境のみ（リモートアクセス不可）
- PTYが利用可能な環境（Windows/macOS/Linux）
**スケール/範囲**:
- 単一ユーザー（ローカル開発者）
- 最大10個の同時AIツールセッション
- 最大1000ブランチの表示対応

## 原則チェック

*ゲート: フェーズ0の調査前に合格する必要があります。フェーズ1の設計後に再チェック。*

### CLAUDE.md 開発指針との整合性

#### ✅ 合格項目

1. **シンプルさの追求**: Web UI実装は既存のReact（Ink）構造を最大限活用し、複雑な新規アーキテクチャを避ける
2. **ユーザビリティ重視**: ブラウザでの直感的な操作を提供し、CLIコマンドを覚える必要性を軽減
3. **既存ファイルの優先**: src/ui/ を src/cli/ui/ に移動し、コアロジックを src/core/ に抽出して再利用
4. **Spec Kit準拠**: 現在、spec.md作成済み、plan.md作成中（TDD実装前にSpec承認が必要）
5. **bun使用**: ローカル検証・実行はbunを使用（既存方針に準拠）

#### ⚠️ 確認必要項目

1. **Spec Kit承認**: このplan.md完了後、ユーザー承認を得てからTDD開始（CLAUDE.md要件）
2. **コミットメッセージ**: Conventional Commits形式で記述（semantic-release対応）
3. **テスト完了確認**: エラーが解消された時点で完了とする

#### 🎯 適用される原則

- **設計ガイドライン**: 設計ドキュメント（plan.md, research.md, data-model.md）にソースコードは書かない
- **完了条件**: エラーが発生している状態で完了としない
- **開発ワークフロー**: 作業完了後、日本語コミットログでコミット＆プッシュ
- **リリースワークフロー**: feature/webui → develop へAuto Merge

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-d5e56259/
├── plan.md              # このファイル（/speckit.plan コマンド出力）
├── research.md          # フェーズ0出力（/speckit.plan コマンド）
├── data-model.md        # フェーズ1出力（/speckit.plan コマンド）
├── quickstart.md        # フェーズ1出力（/speckit.plan コマンド）
├── contracts/           # フェーズ1出力（/speckit.plan コマンド）
│   ├── rest-api.yaml    # REST API仕様（OpenAPI）
│   └── websocket.md     # WebSocketプロトコル仕様
└── tasks.md             # フェーズ2出力（/speckit.tasks コマンド）
```

### ソースコード（リポジトリルート）

```text
src/
├── cli/                 # CLI UI（既存のInk実装を移動）
│   ├── ui/              # 既存の src/ui/ から移動
│   └── index.ts         # CLIエントリーポイント
├── web/                 # Web UI（新規）
│   ├── server/          # Fastifyサーバー
│   │   ├── index.ts     # サーバーエントリーポイント
│   │   ├── routes/      # REST APIルート
│   │   ├── websocket/   # WebSocketハンドラー
│   │   └── pty/         # PTYマネージャー
│   └── client/          # Vite + React
│       ├── src/
│       │   ├── components/  # UIコンポーネント
│       │   ├── hooks/       # カスタムhooks
│       │   ├── pages/       # ページコンポーネント
│       │   └── lib/         # ユーティリティ
│       ├── public/
│       └── vite.config.ts
├── core/                # 共通ビジネスロジック（既存から抽出）
│   ├── git.ts           # Git操作（既存）
│   ├── worktree.ts      # Worktree管理（既存）
│   ├── claude.ts        # Claude Code起動（既存）
│   ├── codex.ts         # Codex CLI起動（既存）
│   ├── launcher.ts      # カスタムツール起動（既存）
│   ├── services/        # ビジネスロジック（既存）
│   └── repositories/    # データアクセス（既存）
├── types/               # 共通型定義
└── index.ts             # エントリーポイント（CLI/serve分岐）

tests/
├── cli/                 # CLI用テスト
├── web/                 # Web UI用テスト
└── e2e/                 # E2Eテスト（Playwright）
```

## フェーズ0: 調査（技術スタック選定）

**目的**: 要件に基づいて技術スタックを決定し、既存のコードパターンを理解する

**出力**: `specs/SPEC-d5e56259/research.md`

### 調査項目

1. **既存のコードベース分析**
   - 現在の技術スタック: TypeScript 5.8.x, React 19, Ink 6, Bun 1.0+, Vitest
   - 既存のパターン: Ink UIコンポーネント、hooks（useGitData, useScreenState）、services/repositories層
   - 統合ポイント: git.ts, worktree.ts, claude.ts, codex.ts, launcher.ts

2. **技術的決定**
   - **WebSocketライブラリ**: @fastify/websocket（Fastify統合、軽量）
   - **PTYライブラリ**: node-pty（業界標準、VS Code使用実績）
   - **ターミナルエミュレーター**: xterm.js（ANSI完全対応、VS Code Web版使用実績）
   - **フロントエンドビルドツール**: Vite 6（高速、React 19対応）
   - **状態管理**: TanStack Query（サーバー状態）+ Zustand（クライアント状態）
   - **UIコンポーネント**: shadcn/ui（Tailwind CSS、アクセシビリティ対応）

3. **制約と依存関係**
   - **既存CLI互換性**: src/index.ts でCLI/serve分岐、コアロジックは共通化
   - **PTY制約**: Windowsでは一部制限あり（調査必要）
   - **パフォーマンス**: WebSocket接続数上限、PTYプロセス数上限（システムリソース依存）

## フェーズ1: 設計（アーキテクチャと契約）

**目的**: 実装前に技術設計を定義する

**出力**:
- `specs/SPEC-d5e56259/data-model.md`
- `specs/SPEC-d5e56259/quickstart.md`
- `specs/SPEC-d5e56259/contracts/`

### 1.1 データモデル設計

**ファイル**: `data-model.md`

主要なエンティティとその関係を定義：
- **Worktree**: ブランチ名、パス、作成日時、状態
- **Branch**: ブランチ名、タイプ（local/remote）、コミットハッシュ、マージ状態、Worktreeパス
- **AIToolSession**: セッションID、ツールタイプ、モード、Worktreeパス、PTY PID、WebSocket ID、開始/終了時刻、出力ログ
- **CustomAITool**: ツール名、コマンド、実行タイプ、デフォルト引数、環境変数

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの簡潔なガイド：
- セットアップ手順: `bun install`, `bun run build`
- 開発ワークフロー: CLI開発（`bun run start`）, Web UI開発（`bun run dev:web`）
- よくある操作: サーバー起動（`bunx . serve`）、ビルド（`bun run build:web`）
- トラブルシューティング: ポート競合、PTY初期化エラー、WebSocket接続エラー

### 1.3 契約/インターフェース

**ディレクトリ**: `contracts/`

#### REST API（rest-api.yaml）

- `GET /api/health` - ヘルスチェック
- `GET /api/branches` - ブランチ一覧取得
- `GET /api/branches/:branchName` - 特定ブランチの詳細情報取得（URLエンコード対応、例: `/api/branches/feature%2Fwebui`）
- `GET /api/worktrees` - Worktree一覧取得
- `POST /api/worktrees` - Worktree作成
- `DELETE /api/worktrees/:path` - Worktree削除
- `GET /api/sessions` - セッション履歴取得
- `POST /api/sessions/start` - AIツールセッション開始
- `GET /api/config` - 設定取得
- `PUT /api/config` - 設定更新

#### フロントエンドルーティング

- `/` - ブランチ一覧ページ（ホーム）
- `/:branchName` - ブランチ詳細ページ（例: `/feature-webui`、スラッシュを含む場合は`/feature%2Fwebui`）
  - ブランチ情報表示
  - Worktree作成ボタン
  - AI Tool起動ボタン
  - Worktree削除ボタン（Worktreeが存在する場合）

#### WebSocket（websocket.md）

- エンドポイント: `/ws/terminal/:sessionId`
- クライアント → サーバー:
  - `{ type: 'input', data: string }` - ターミナル入力
  - `{ type: 'resize', cols: number, rows: number }` - リサイズ
- サーバー → クライアント:
  - `{ type: 'output', data: string }` - ターミナル出力
  - `{ type: 'exit', code: number }` - プロセス終了

## フェーズ2: タスク生成

**次のステップ**: `/speckit.tasks` コマンドを実行

**入力**: このプラン + 仕様書 + 設計ドキュメント

**出力**: `specs/SPEC-d5e56259/tasks.md` - 実装のための実行可能なタスクリスト

## 実装戦略

### 優先順位付け

ユーザーストーリーの優先度に基づいて実装：

1. **P1 - ストーリー1**: Webサーバーの起動とアクセス
   - `claude-worktree serve`コマンド実装
   - Fastifyサーバー基本セットアップ
   - 静的ファイル配信
   - MVP: ブラウザでアクセス可能

2. **P1 - ストーリー2**: ブランチ一覧とWorktree作成
   - REST API実装（branches, worktrees）
   - フロントエンドルーティング: `/` （ブランチ一覧）、`/:branchName` （ブランチ詳細）
   - フロントエンド: ブランチ一覧画面（リンク付き）、ブランチ詳細画面（Worktree作成ボタン）
   - URLエンコード/デコード処理（スラッシュを含むブランチ名対応）
   - MVP: ブラウザからブランチ詳細ページでWorktree作成可能

3. **P1 - ストーリー3**: AI Tool起動とターミナル統合
   - PTYマネージャー実装
   - WebSocketサーバー実装
   - xterm.js統合
   - MVP: ブラウザからClaude Code起動・操作可能

4. **P2 - ストーリー4**: Worktree削除と管理
   - Worktree管理画面
   - クリーンアップ機能

5. **P2 - ストーリー5**: 設定管理
   - 設定管理画面
   - tools.json編集UI

6. **P3 - ストーリー6**: Git操作ビジュアル化
   - ブランチツリー表示
   - 差分可視化

### 独立したデリバリー

各ユーザーストーリーは独立して実装・テスト・デプロイ可能：
- **ストーリー1完了**: サーバー起動可能なMVP → デプロイ可能
- **ストーリー2追加**: Worktree管理機能追加 → 拡張MVP
- **ストーリー3追加**: AI Tool統合完了 → コア機能完成
- **ストーリー4-6追加**: UX向上機能 → 完全版

## テスト戦略

### ユニットテスト（Vitest）

- **PTYマネージャー**: spawn, write, resize, kill操作
- **WebSocketハンドラー**: メッセージ送受信、エラーハンドリング
- **REST APIルート**: 各エンドポイントの正常系・異常系
- **Reactコンポーネント**: @testing-library/react使用

### 統合テスト（Vitest）

- **CLI/Web分岐**: index.ts のコマンド引数パース
- **PTY + WebSocket**: エンドツーエンドの入出力転送
- **REST API + Git操作**: ブランチ取得、Worktree作成・削除

### E2Eテスト（Playwright）

- **ストーリー1**: サーバー起動 → ブラウザアクセス → ページ表示
- **ストーリー2**: ブランチ選択 → Worktree作成 → 成功メッセージ
- **ストーリー3**: AI Tool起動 → ターミナル表示 → 入出力動作
- **ストーリー4**: Worktree削除 → 確認ダイアログ → 削除完了

### パフォーマンステスト

- ブランチ一覧表示（1000ブランチ）: 3秒以内
- ターミナルI/O遅延測定: 100ms以内
- WebSocket同時接続数: 最大10セッション

## リスクと緩和策

### 技術的リスク

1. **PTYのWindows互換性**: node-ptyはWindowsでConPTY使用、制限あり
   - **緩和策**: Windows固有のテストケース追加、ドキュメントで制限明記

2. **WebSocket接続安定性**: ネットワーク切断時のセッション復元
   - **緩和策**: 再接続ロジック実装、セッションをメモリに保持（タイムアウト付き）

3. **xterm.jsのANSI対応**: 一部のANSI escape codesが未対応の可能性
   - **緩和策**: xterm.js最新版使用、主要ANSI codesの事前テスト

4. **大量ブランチのパフォーマンス**: 1000以上のブランチで遅延
   - **緩和策**: フロントエンドで仮想スクロール（react-window）、ページネーション

### 依存関係リスク

1. **node-ptyのメンテナンス状況**: VS Code依存のため安定、ただし更新頻度低い
   - **緩和策**: 代替ライブラリ（node-pty-prebuilt-multiarch）を調査、必要時切り替え

2. **Fastify + WebSocketの統合**: @fastify/websocketの互換性
   - **緩和策**: Fastify 5とWebSocketプラグインのバージョン固定、統合テスト実施

## 次のステップ

1. ⏭️ フェーズ0実行: research.md生成（調査エージェント起動）
2. ⏭️ フェーズ1実行: data-model.md, contracts/, quickstart.md生成
3. ⏭️ エージェントコンテキスト更新: `.specify/scripts/bash/update-agent-context.sh claude`
4. ⏭️ 原則チェック再評価: 設計完了後のCLAUDE.md準拠確認
5. ⏭️ `/speckit.tasks` 実行: tasks.md生成
6. ⏭️ `/speckit.implement` 実行: TDD開始
