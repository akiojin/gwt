# Quickstart: OpenTUI 移行の開発手順

## セットアップ

1. 依存関係の取得: `bun install`
2. Zig のインストールと PATH 設定（OpenTUI ビルド用）
   - macOS: Homebrew などのパッケージマネージャで Zig をインストール
   - Linux: 公式バイナリを取得して任意ディレクトリに展開し PATH へ追加
   - Windows: 公式 ZIP を展開し、`C:\\tools\\zig` などに配置 → 展開先を PATH に追加
3. Zig の動作確認: `zig version`
4. 既存 CLI のビルド: `bun run build:cli`

## 開発ワークフロー

- CLI の起動: `bun run start` または `bunx .`
- UI テストの実行: `bun run test`（Vitest）
- E2E テストの実行: `bun run test:e2e`（Playwright）
- 型チェック: `bun run type-check`

## Windows ネイティブの注意点

- Windows 環境での実行確認を必須とする（WSL ではなくネイティブ）。
- Zig のインストール後、`zig version` が実行できる状態にする。
- 端末は Windows Terminal を推奨する。
- シェルは PowerShell 7+（`pwsh`）を使用する。
- `bun -v` と `zig version` の両方が PowerShell 7+ で通ることを確認する。

## トラブルシューティング

- Zig が見つからない場合: PATH 設定を確認する。
- UI が崩れる場合: 端末の幅/高さ、フォント設定を確認する。
- パフォーマンステストが不安定な場合: 競合プロセスを停止し、同条件で再測定する。
