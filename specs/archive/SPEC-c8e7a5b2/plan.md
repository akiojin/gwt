# 実装計画: CLI起動時Web UIサーバー自動起動

**仕様ID**: `SPEC-c8e7a5b2` | **日付**: 2025-12-12 | **仕様書**: [spec.md](./spec.md)
**入力**: 既存実装の後付け仕様化・TDD追加

## 概要

- CLI起動時にWeb UIサーバーをバックグラウンドで非同期起動
- サーバー起動失敗時はCLI動作を継続（グレースフルデグレード）
- PORT環境変数でポートカスタマイズ可能

## 技術コンテキスト

- **言語/バージョン**: TypeScript (ESM) / Bun 1.x
- **主要な依存関係**: Fastify（Web UIサーバー）、pino（ロギング）
- **テスト**: vitest（単体テスト）、vi.mock()でモジュールモック
- **ターゲットプラットフォーム**: Bun/Node 環境（macOS/Linux/Windows）
- **プロジェクトタイプ**: 単一リポジトリ（CLI + Webサーバー）

## フェーズ0: 調査（完了）

既存実装の分析完了:

- src/index.ts:877-883 にWeb UIサーバー起動実装あり
- startWebServer()は動的import()で遅延ロード
- エラーはcatch()で捕捉しappLogger.warnで記録
- runInteractiveLoop()の前に起動、待機なし（fire-and-forget）

## フェーズ1: 設計（完了）

### アーキテクチャ決定

1. **非同期起動**: startWebServer().catch()パターンで非ブロッキング
2. **エラーハンドリング**: 警告ログのみ、CLI継続
3. **ポート設定**: process.env.PORT || 3000

### 実装箇所

実装箇所: `src/index.ts:877-883`

## テスト戦略

### ユニットテスト対象

- startWebServerが呼び出される
- printInfoで起動メッセージが表示される
- エラー時にappLogger.warnが呼び出される
- エラー時もrunInteractiveLoopが呼び出される
- PORT環境変数がメッセージに反映される
- Gitリポジトリ外ではサーバー起動しない

### モック対象

- `src/web/server/index.js` - startWebServer
- `src/logging/logger.js` - createLogger
- `src/git.js` - isGitRepository

## 次のステップ

1. ~~フェーズ0完了: 既存実装の調査~~
2. ~~フェーズ1完了: 設計とアーキテクチャ定義~~
3. テストファイル作成（tests/unit/index.webui-startup.test.ts）
4. テスト実行・検証
5. コミット＆プッシュ
