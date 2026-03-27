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

TUI のキーバインドは全て `Ctrl+G` プレフィックスを使用します。

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

## 必要環境変数と前提

### 必須

- `PATH` に `git` があること（Git コマンドが使える状態）

### 任意

- AI 利用時の認証情報（または Settings のプロファイル設定でも可）:
  - `ANTHROPIC_API_KEY` または `ANTHROPIC_AUTH_TOKEN`
  - `OPENAI_API_KEY`
  - `GOOGLE_API_KEY` または `GEMINI_API_KEY`
- `bunx` / `npx`（ローカル起動のフォールバックに利用）

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

通常ログは `~/.gwt/logs/` 配下に JSON Lines 形式で保存されます。パフォーマンスプロファイリングは **Settings > Profiling** で有効化できます。
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
│   └── gwt-tui/        # ratatui TUI フロントエンド
├── specs/              # ローカル SPEC 管理（SPEC-{N}/）
└── package.json        # 開発用スクリプト
```

## ライセンス

MIT
