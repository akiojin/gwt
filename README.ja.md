# gwt

gwt は Git worktree 管理とコーディングエージェント起動
（Claude Code / Codex / Gemini / OpenCode）を行うデスクトップ GUI アプリです。

## インストール

### macOS（シェルインストーラー）

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
```

バージョン指定:

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version 6.30.3
```

### macOS（ローカル `.pkg` インストーラー）

ローカル `.pkg` を作成:

```bash
cargo tauri build
./installers/macos/build-pkg.sh
```

ローカル `.pkg` からインストール:

```bash
./installers/macos/install.sh --pkg ./target/release/bundle/pkg/gwt-macos-$(uname -m).pkg
```

または、上記を1コマンドで実行:

```bash
./installers/macos/install-local.sh
```

### アンインストール（macOS）

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/uninstall.sh | bash
```

### ダウンロード

配布は GitHub Releases のみです。

主な成果物:

- macOS: `.dmg`, `.pkg`
- Windows: `.msi`
- Linux: `.AppImage`, `.deb`

## キーボードショートカット

| ショートカット (macOS) | ショートカット (Windows/Linux) | 操作 |
|---|---|---|
| Cmd+N | Ctrl+N | 新しいウィンドウ |
| Cmd+O | Ctrl+O | プロジェクトを開く |
| Cmd+C | Ctrl+C | コピー |
| Cmd+V | Ctrl+V | ペースト |
| Cmd+Shift+K | Ctrl+Shift+K | Worktree のクリーンアップ |
| Cmd+, | Ctrl+, | 環境設定 |
| Cmd+Shift+[ | Ctrl+Shift+[ | 前のタブ |
| Cmd+Shift+] | Ctrl+Shift+] | 次のタブ |
| Cmd+` | Ctrl+` | 次のウィンドウ |
| Cmd+Shift+` | Ctrl+Shift+` | 前のウィンドウ |
| Cmd+M | --- | 最小化（macOS のみ） |

## 開発

前提:

- Rust（stable）
- Node.js 22
- pnpm（Corepack 経由）
- Tauri の OS 依存パッケージ（プラットフォーム別）

開発起動:

```bash
cd gwt-gui
pnpm install --frozen-lockfile

cd ..
cargo tauri dev
```

ビルド:

```bash
cd gwt-gui
pnpm install --frozen-lockfile

cd ..
cargo tauri build
```

Playwright E2E（WebView UI スモーク）:

```bash
cd gwt-gui
pnpm install --frozen-lockfile
pnpm exec playwright install chromium
pnpm run test:e2e
```

CI では `.github/workflows/test.yml` の `E2E (Playwright)` ジョブで同じ Playwright テストを実行します。

## AI 設定

Agent Mode やセッション要約を使うには AI 設定が必要です。

手順:

- `Settings` を開く
- `Profiles` でプロファイルを選択
- `AI Settings` を有効化
- `Endpoint` と `Model` を設定（ローカル LLM の場合は API Key 省略可）
- `Save` をクリック

## ディレクトリ構成

- `crates/gwt-core/`: コア（Git/worktree/設定/ログ/Docker/PTY）
- `crates/gwt-tauri/`: Tauri v2 バックエンド（commands + state）
- `gwt-gui/`: Svelte 5 フロントエンド（UI + xterm.js）
- `installers/`: インストーラー定義（例: WiX）

## ライセンス

MIT
