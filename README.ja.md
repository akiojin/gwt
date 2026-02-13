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

### アンインストール（macOS）

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/uninstall.sh | bash
```

### ダウンロード

配布は GitHub Releases のみです。

主な成果物:

- macOS: `.dmg`
- Windows: `.msi`
- Linux: `.AppImage`, `.deb`

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
