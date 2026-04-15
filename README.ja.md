# gwt

gwt は Git worktree の管理と、ブランチ単位での
`Claude Code` / `Codex` / `Gemini` / `OpenCode` 起動を行うターミナル (TUI) ツールです。

## インストール

[GitHub Releases](https://github.com/akiojin/gwt/releases) からお使いの
プラットフォーム向けバイナリをダウンロードし、`PATH` に配置してください。

### macOS

インストーラーを実行します。

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
```

特定バージョンを指定してインストール:

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version 6.30.3
```

### Windows

GitHub Releases からバイナリをダウンロードして `PATH` に配置します。

### Linux

GitHub Releases からバイナリをダウンロードして `PATH` に配置します。

### アンインストール（macOS）

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/uninstall.sh | bash
```

## 使い方

カレントディレクトリで TUI を起動します。

```bash
gwt
```

### ターミナル要件

- 256 色対応ターミナル推奨（最新のターミナルはほぼ対応済み）
- 最小 80x24 のターミナルサイズ

## 使い始め方

1. Git リポジトリ内で `gwt` を実行します。
2. サイドバーでブランチと worktree を閲覧します。
3. ブランチ操作欄から次を行います。
   - worktree の作成/一覧/クリーンアップ
   - エージェント起動
4. Agent や要約機能を使う場合は、**Settings** で AI プロファイルを設定します。

## キーバインド

TUI の主要なキーバインドは `Ctrl+G` プレフィックスを使用します。terminal
text はドラッグ範囲選択した時点で copy されます。

| キーバインド | 操作 |
|---|---|
| `Ctrl+G`, `c` | 新しいシェルタブ |
| `Ctrl+G`, `n` | 新しいエージェントタブ |
| `Ctrl+G`, `1`-`9` | タブ N に切替 |
| `Ctrl+G`, `]` | 次のタブ |
| `Ctrl+G`, `[` | 前のタブ |
| `Ctrl+G`, `x` | 現在のタブを閉じる |
| `Ctrl+G`, `w` | Worktree 一覧 |
| `Ctrl+G`, `s` | 設定 |
| `Ctrl+G`, `?` | ヘルプ / キーバインド一覧 |
| `Ctrl+G`, `q` | 終了 |

terminal text はドラッグで copy できます。ホスト terminal が shortcut を
転送する環境では、次も使えます。

- macOS: `Cmd+C`
- Linux / Windows: `Ctrl+Shift+C`

シェルやエージェント端末で日本語 IME の候補選択を調査する場合は、
`GWT_INPUT_TRACE_PATH=/tmp/gwt-input-trace.jsonl` を付けて gwt を起動してください。
JSONL トレースには、生の `crossterm` キーイベント、keybind 判定、PTY に
転送したバイト列が記録され、実行中に入力モードを切り替える必要はありません。

その routed trace と terminal の生入力を比較したい場合は、同じ端末で
`cargo run -p gwt-tui --example keytest -- --mode raw` を実行してください。
probe は既定で `/tmp/gwt-crossterm-events.jsonl` に全
`crossterm::event::Event` を記録し、必要なら位置引数で出力先を
上書きできます。

描画起因の IME 退行を切り分けるため、同じ probe には `--mode redraw` と
`--mode ratatui` もあります。`redraw` は同じ committed-text surface を
direct `crossterm` で周期再描画し、`ratatui` は同じ surface を同じ tick
で ratatui 経由に切り替えます。モード比較時の再描画間隔は
`--tick-ms <N>` で変更できます。

また gwt は起動時に minimal な kitty keyboard enhancement
(`DISAMBIGUATE_ESCAPE_CODES | REPORT_ALL_KEYS_AS_ESCAPE_CODES |
REPORT_ALTERNATE_KEYS | REPORT_EVENT_TYPES`) を要求し、終了時に pop します。
非対応端末では fail-open で従来挙動を維持します。互換端末で発生する
繰り返しキーイベントも通常の key press と同じ入力経路に残るため、IME の
候補ページ送り時にイベントが途中で消えにくくなります。さらに terminal pane
が focus を持つ間は、overlay など明示的に周期 UI が必要な場合を除いて、
idle な 100 ms tick では TUI を再描画しないため、バックグラウンド redraw に
よる IME 候補操作の中断を抑えます。一方で PTY output は即座に redraw を要求
するため、確定文字や通常の shell 出力が次のキー入力まで遅延しません。

## 必要環境変数と前提

### 必須

- `PATH` に `git` があること（Git コマンドが使える状態）

### 任意

- AI 利用時の認証情報（または Settings のプロファイル設定でも可）:
  - `ANTHROPIC_API_KEY` または `ANTHROPIC_AUTH_TOKEN`
  - `OPENAI_API_KEY`
  - `GOOGLE_API_KEY` または `GEMINI_API_KEY`
- `bunx` / `npx`（ローカル起動のフォールバックに利用）
- gwt が shared project-index runtime を bootstrap / repair するとき
  （起動時やリポジトリ初期化時など）には `PATH` 上に Python 3.9+ が必要です。
  gwt が `~/.gwt/runtime/chroma-venv` を自動作成し、その後は managed runtime を再利用します。
  Windows では Command Prompt / PowerShell で `python` または `py -3` が通る状態にしてください。
- ベクトル索引データ (Issue / SPEC / ソースファイル) は `~/.gwt/index/<repo-hash>/` 配下に
  保存されます。Issue および SPEC はリポジトリ単位で共有、ソースファイルは Worktree 単位です。
  TUI は Worktree ごとにファイルシステム watcher を常駐させ、Issue 索引は起動時に
  15 分 TTL で非同期リフレッシュします。初回検索時に `intfloat/multilingual-e5-base`
  埋め込みモデル (約 440MB) を `~/.cache/huggingface/` にダウンロードします。
  SPEC は `gwt-spec` ラベル付き GitHub Issue として格納され、`~/.gwt/cache/issues/<repo-hash>/` に
  キャッシュされます。読み取りは `gwt issue spec <n>`、書き込みは
  `gwt issue spec <n> --edit <section> -f <file>` を使用してください。

### Hook 設定ファイルの扱い

- gwt は `.claude/settings.local.json` をローカル端末向け設定として再生成し、このファイルの Git 除外も管理します。
- gwt は `.codex/hooks.json` を作成またはマージしますが、`.gitignore` や `info/exclude` には追加しません。
- `.codex/hooks.json` を version 管理するかどうかは各リポジトリの判断です。既存ファイルがある場合、gwt は gwt 管理 hook だけを置き換え、ユーザー hook と無関係な top-level 設定は保持します。

### GitHub Token（PAT）要件

gwt は GitHub 操作に `gh` CLI を使用します。以下で認証してください:

```bash
gh auth login
```

#### Fine-grained PAT 推奨権限

| 権限 | アクセス | 用途 |
|---|---|---|
| **Contents** | Read and Write | リポジトリ参照、ブランチ操作、リリース |
| **Pull requests** | Read and Write | PR 作成・編集・マージ・レビュー |
| **Issues** | Read and Write | Issue 作成・編集・コメント |
| **Metadata** | Read | 暗黙付与 |

#### 読み取り専用の最小権限

閲覧のみ（PR 作成やブランチ管理なし）の場合:

| 権限 | アクセス |
|---|---|
| **Contents** | Read |
| **Pull requests** | Read |
| **Issues** | Read |
| **Metadata** | Read |

### 任意（高度設定）

- `GWT_AGENT_AUTO_INSTALL_DEPS` (`true` / `false`)
- `GWT_DOCKER_FORCE_HOST` (`true` / `false`)

### ログとプロファイリング

通常ログはプロジェクトごとに `~/.gwt/logs/<repo-hash>/gwt.log.YYYY-MM-DD` へ JSON Lines 形式で保存されます。パフォーマンスプロファイリングは **Settings > Profiling** で有効化できます。
ログ仕様の詳細は [#1758](https://github.com/akiojin/gwt/issues/1758) を参照してください。

## 開発

### ビルド

```bash
cargo build -p gwt-tui
```

### 実行（開発）

```bash
cargo run -p gwt-tui
```

### Canvas terminal PoC の実行

```bash
cargo run -p poc-terminal
```

この PoC はローカル HTTP/WebSocket server を起動し、その URL を読み込む desktop
WebView を開きます。起動すると stderr に `http://127.0.0.1:<port>/` のような browser URL
を出力するので、native app の起動中は同じ URL を通常の browser でも開けます。

canvas では `Shell` / `Claude` / `Codex` の terminal window と、read-only の
`File Tree` window、そして `Branches` window を扱えます。`Branches` は
single click で選択、double click で中央表示の Launch Agent panel を開き、
quick start / new branch / agent / model / runtime / permissions を選べます。
`Settings` / `Memo` / `Profile` / `Logs` / `Issue` / `SPEC` / `Board` / `PR`
は mock window として追加できます。

`Tile` で表示中の window をグリッド状に整列し、`Stack` で title bar を残した
まま重ねて表示できます。
`Cmd/Ctrl+Shift+Right` と `Cmd/Ctrl+Shift+Left` で focus window を順送り /
逆送りでき、keyboard で切り替えた window は canvas の中央に寄ります。

terminal 描画は runtime 時に CDN から `xterm.js` を読み込むため、初回起動時は
ネットワーク接続が必要です。

### macOS 向け PoC `.app` bundle の生成

まず一度だけ `cargo-bundle` を install します。

```bash
cargo install cargo-bundle
```

その後、ローカル用の `.app` bundle を生成します。

```bash
cargo bundle -p poc-terminal --format osx
```

生成された `.app` は既定で `target/debug/bundle/osx/` に出力されます。Finder
からその `.app` を開けます。PoC の icon は専用 asset が未提供のため、現時点では
汎用 app icon です。

### テスト

```bash
cargo test -p gwt-core -p gwt-tui
```

### Lint

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

### フォーマット

```bash
cargo fmt
```

## プロジェクト構成

```text
├── Cargo.toml          # ワークスペース設定
├── crates/
│   ├── gwt-core/       # コアライブラリ（Git操作・PTY管理・設定）
│   ├── gwt-github/     # GitHub Issue SOT for SPEC 管理 (SPEC-12)
│   └── gwt-tui/        # ratatui TUI フロントエンド + CLI (`gwt issue spec ...`)
├── poc/
│   └── terminal/       # canvas 風 floating terminal GUI PoC
└── package.json        # 開発用スクリプト
```

## ライセンス

MIT
