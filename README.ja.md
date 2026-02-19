# gwt

gwt は Git worktree の管理と、ブランチ単位での
`Claude Code` / `Codex` / `Gemini` / `OpenCode` 起動を行うデスクトップ GUI アプリです。

## インストール

### macOS

インストーラーを実行します。

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash
```

特定バージョンを指定してインストール:

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/install.sh | bash -s -- --version 6.30.3
```

配布アセット:

- macOS: `.dmg`, `.pkg`

### Windows

GitHub Releases から `.msi` をダウンロードして実行します。

### Linux

以下をダウンロードして通常の方法で実行します。

- `.deb`
- `.AppImage`

### アンインストール（macOS）

```bash
curl -fsSL https://raw.githubusercontent.com/akiojin/gwt/main/installers/macos/uninstall.sh | bash
```

## 使い始め方

1. gwt を起動します。
2. **Open Project...** から Git リポジトリを開きます。
3. サイドバーで対象ブランチを選択します。
4. ブランチ操作欄から次を行います。
   - worktree の作成/一覧/クリーンアップ
   - エージェント起動
5. Agent や要約機能を使う場合は、**Settings** で AI プロファイルを設定します。

## 自動アップデート

gwt は GitHub Releases を参照して自動アップデートを確認します。

- 起動時に自動で更新チェックを行います。
- 失敗した場合は数回再試行します。
- 更新が見つかると通知されます。
- メニューの **Help → Check for Updates...** から手動チェックできます。

更新可能なインストーラー/バイナリが検出できる場合は、アプリ側から更新を適用できます。
自動適用できない場合は、リリースページから手動ダウンロードが必要と案内されます。

## キーボードショートカット

| ショートカット (macOS) | ショートカット (Windows/Linux) | 操作 |
|---|---|---|
| Cmd+N | Ctrl+N | 新しいウィンドウ |
| Cmd+O | Ctrl+O | プロジェクトを開く |
| Cmd+C | Ctrl+C | コピー |
| Cmd+V | Ctrl+V | ペースト |
| Cmd+Shift+C | Ctrl+Shift+C | 画面テキストのコピー |
| Cmd+Shift+K | Ctrl+Shift+K | Worktree のクリーンアップ |
| Cmd+, | Ctrl+, | 設定 |
| Cmd+Shift+[ | Ctrl+Shift+[ | 前のタブ |
| Cmd+Shift+] | Ctrl+Shift+] | 次のタブ |
| Cmd+` | Ctrl+` | 次のウィンドウ |
| Cmd+Shift+` | Ctrl+Shift+` | 前のウィンドウ |
| Cmd+M | --- | 最小化（macOS のみ） |

## 必要環境変数と前提

### 必須

- `PATH` に `git` があること（Git コマンドが使える状態）

### 任意

- AI 利用時の認証情報（または Settings のプロファイル設定でも可）:
  - `ANTHROPIC_API_KEY` または `ANTHROPIC_AUTH_TOKEN`
  - `OPENAI_API_KEY`
  - `GOOGLE_API_KEY` または `GEMINI_API_KEY`
- `bunx` / `npx`（ローカル起動のフォールバックに利用）

### 任意（高度設定）

- `GWT_AGENT_AUTO_INSTALL_DEPS` (`true` / `false`)
- `GWT_DOCKER_FORCE_HOST` (`true` / `false`)

## ライセンス

MIT
