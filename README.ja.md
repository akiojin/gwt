# gwt

gwt は Git worktree 管理とコーディングエージェント起動
（Claude Code / Codex / Gemini / OpenCode）を行うデスクトップ GUI アプリです。

## 配布

配布は GitHub Releases のみです。

主な成果物:

- macOS: `.dmg`, `.pkg`
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

## ディレクトリ構成

- `crates/gwt-core/`: コア（Git/worktree/設定/ログ/Docker/PTY）
- `crates/gwt-tauri/`: Tauri v2 バックエンド（commands + state）
- `gwt-gui/`: Svelte 5 フロントエンド（UI + xterm.js）
- `installers/`: インストーラー定義（例: WiX）

## ライセンス

MIT
