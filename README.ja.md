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

### 音声認識の精度評価

ローカル音声データセットで WER/CER を計測できます。

```bash
cp tests/voice_eval/manifest.template.json tests/voice_eval/manifest.json
scripts/voice-eval.sh
```

詳細は `tests/voice_eval/README.md` を参照してください。
バージョン管理するベンチマークスナップショットは `docs/voice-eval-benchmarks.md` を参照してください。

### 音声入力ランタイム（Qwen3-ASR）

音声入力はローカル Python ランタイム経由で Qwen3-ASR を実行します。

- 必須: Python 3.11 以上（`PATH` 上、または `GWT_VOICE_PYTHON` で指定）
- 手動導入不要: `qwen_asr` パッケージ
- 初回利用時に gwt が `~/.gwt/runtime/voice-venv` を自動作成し、必要依存を自動インストール
- その後、選択品質に対応する Qwen モデルを Hugging Face キャッシュへ必要時に取得

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
