# 機能仕様: OS環境変数の自動継承

**仕様ID**: `SPEC-os-env-01`
**作成日**: 2026-02-10
**ステータス**: 確定
**カテゴリ**: Core / Terminal / Environment

**入力**: ユーザー説明: "bashなどに設定されている環境変数が適用されていない"

## 背景

gwt GUI は Tauri v2 デスクトップアプリとして動作し、エージェント（Claude Code / Codex / Gemini等）を PTY 上で起動する。
しかし、GUI アプリは Dock/Spotlight 等から起動されるため、ユーザーのシェルプロファイル（`.bashrc`/`.zshrc`等）が読み込まれず、以下の問題が発生する:

- **PATH不足**: brew/nvm/pyenv/conda 等で追加されたパスが含まれず、コマンドが見つからない
- **APIキー欠落**: `ANTHROPIC_API_KEY` 等のシェルで `export` された秘密情報がエージェントに渡らない
- **その他の変数**: ユーザー独自の環境変数が一切反映されない

現在の実装では、環境変数は `~/.gwt/profiles.toml` のプロファイルシステムでのみ管理されており、OS環境変数は自動継承されない。

## 設計判断

### 三層マージ構造

環境変数は以下の優先順位で三層マージされる（後勝ち）:

1. **OS環境変数**（ベース）: ログインシェルから取得
2. **プロファイル環境変数**: `profiles.toml` の `env` で上書き + `disabled_env` で削除
3. **リクエストオーバーライド**: `launch_agent` の `env_overrides` + gwt コンテキスト変数 + GLMプロバイダー設定で最終上書き

> **注**: GLMプロバイダー設定（ANTHROPIC_BASE_URL等）は常にOS環境変数を上書きする（現行動作維持）。

### OS環境変数の取得方法

- **Unix**: ログインシェルを起動してNUL区切りで環境変数を取得
  - `$SHELL` を自動検出し、ユーザーのデフォルトシェルを使用
  - `$SHELL` 未設定時は `/bin/sh` にフォールバック
  - シェル種別に応じたコマンド構築:
    - **bash/zsh/sh/その他POSIXシェル**: `$SHELL -l -c 'env -0'`
    - **fish**: `fish -l -c 'env -0'`
    - **nushell**: `nu -l -c '$env | to json'`（JSON出力をパース）
  - タイムアウト: 5秒
  - エラー/タイムアウト時: `std::env::vars()` にフォールバック + warnログ + GUIトースト通知（初回のみ）
- **Windows**: `std::env::vars()` のみ使用（レジストリにシステム環境変数があるためログインシェル不要）

### 取得タイミング

- Tauriアプリ起動時に1回、非同期で取得（UIの初期表示はブロックしない）
- 取得完了まではエージェント起動を待機させ、GUIに "Loading environment..." ステータスを表示
- アプリ起動後の `.bashrc` 変更にはアプリ再起動が必要

### EnvSnapshot の廃止

`runner.rs` の `EnvSnapshot`（PATH/HOME/BUN_INSTALL等の個別キャプチャ）は、OS環境変数がベースになることで不要になるため廃止し、シンプル化する。

### PTYへの環境変数渡し

取得した全環境変数をフィルタなしでPTYに渡す。セキュリティフィルタリングは不要。

### Docker転送ポリシー

現行の `ENV_PASSTHROUGH_PREFIXES` allowlist方式を維持する。OS環境変数がベースになっても、Docker転送時はallowlistでフィルタする。

### フォールバック時の通知

ログインシェルからの取得が失敗した場合:

- `warn!` ログに記録
- 初回のみGUIにトースト通知を表示: "Shell environment not loaded. Using process environment."

### デバッグ機能

メニューに `Debug: Show Captured Environment` を追加し、取得した環境変数一覧とソース（login shell / std::env fallback）を確認できるようにする。

## ユーザーシナリオとテスト

### ユーザーストーリー 1 - シェル環境変数がエージェントに反映される (優先度: P0)

開発者として、`.bashrc`/`.zshrc` で `export` した環境変数が、gwt から起動したエージェントにそのまま反映されることを期待する。

**受け入れシナリオ**:

1. **前提条件** `.bashrc` に `export MY_API_KEY=secret123` が設定されている、**操作** gwt からエージェントを起動、**期待結果** エージェント内で `echo $MY_API_KEY` が `secret123` を返す
2. **前提条件** `.zshrc` で nvm/pyenv により PATH が拡張されている、**操作** gwt からエージェントを起動、**期待結果** `node`/`python` コマンドが正常に見つかる
3. **前提条件** profiles.toml で `MY_VAR=override` が設定されている、**操作** エージェントを起動、**期待結果** OS側の `MY_VAR` よりprofiles.toml の値が優先される

### ユーザーストーリー 2 - プロファイルでOS環境変数を無効化できる (優先度: P1)

開発者として、特定のOS環境変数をエージェントに渡したくない場合に、`disabled_env` で除外できることを期待する。

**受け入れシナリオ**:

1. **前提条件** OS に `DANGEROUS_VAR=value` がある、profiles.toml の `disabled_env` に `DANGEROUS_VAR` を追加、**操作** エージェントを起動、**期待結果** `DANGEROUS_VAR` はエージェント環境に存在しない

### ユーザーストーリー 3 - シェルが壊れていてもアプリは起動する (優先度: P1)

開発者として、シェルプロファイルが壊れていても、gwt アプリ自体は正常に起動することを期待する。

**受け入れシナリオ**:

1. **前提条件** `$SHELL` が存在しないパスを指す、**操作** gwt を起動、**期待結果** `std::env::vars()` にフォールバックし、アプリは正常起動。トースト通知が表示される
2. **前提条件** ログインシェルが5秒以上応答しない、**操作** gwt を起動、**期待結果** タイムアウト後に `std::env::vars()` にフォールバック。トースト通知が表示される
3. **前提条件** `$SHELL` が未設定、**操作** gwt を起動、**期待結果** `/bin/sh -l -c 'env -0'` で試行し、成功すればその結果を使用

### ユーザーストーリー 4 - 環境変数のデバッグ確認ができる (優先度: P2)

開発者として、gwt が取得した環境変数の一覧とソースを確認し、問題を診断できることを期待する。

**受け入れシナリオ**:

1. **前提条件** gwt が起動済み、**操作** メニューから "Debug: Show Captured Environment" を実行、**期待結果** 取得した環境変数一覧とソース情報が表示される

### エッジケース

- 環境変数の値に改行を含む場合でも正しくパースされること（`env -0` のNUL区切りで対応）
- fishシェルユーザーでも環境変数が正しく取得されること
- nushellユーザーでも環境変数が正しく取得されること（JSON出力経由）
- MOTD/バナーがシェル起動時に出力されても、環境変数パースに影響しないこと（stdout のNULパースがバナーテキストを無視）
- `$SHELL` 未設定時に `/bin/sh` にフォールバックすること
- 非同期取得中にアプリUIは正常に表示され、エージェント起動が「Loading environment...」で待機状態になること
- GLMプロバイダー設定（ANTHROPIC_BASE_URL等）がOS環境変数の同名変数を上書きすること

## 要件

### 機能要件

- **FR-500**: システムは、Tauriアプリ起動時にユーザーのログインシェルから環境変数を自動取得しなければ**ならない**
- **FR-501**: 環境変数は `OS環境変数 → profiles.toml(env上書き + disabled_env削除) → env_overrides/GLM設定` の三層マージで適用しなければ**ならない**
- **FR-502**: Unix環境では `$SHELL -l -c 'env -0'` でNUL区切りの環境変数を取得しなければ**ならない**
- **FR-503**: fishシェルでは `fish -l -c 'env -0'` の構文で環境変数を取得しなければ**ならない**
- **FR-504**: nushellでは `nu -l -c '$env | to json'` でJSON形式の環境変数を取得しなければ**ならない**
- **FR-505**: ログインシェルの起動が5秒以内にタイムアウトしなければ**ならない**
- **FR-506**: タイムアウト/エラー時は `std::env::vars()` にフォールバックしなければ**ならない**
- **FR-507**: フォールバック時はログ記録 + 初回GUIトースト通知を行わなければ**ならない**
- **FR-508**: `$SHELL` 未設定時は `/bin/sh -l -c 'env -0'` にフォールバックしなければ**ならない**
- **FR-509**: Windows環境では `std::env::vars()` のみで環境変数を取得しなければ**ならない**
- **FR-510**: エージェント起動は環境変数取得完了後にのみ許可し、待機中は「Loading environment...」を表示しなければ**ならない**
- **FR-511**: `runner.rs` の `EnvSnapshot` を廃止し、OS環境変数ベースのコマンド解決に移行しなければ**ならない**
- **FR-512**: PTYへの環境変数渡しにおいてフィルタリングは行わず、全環境変数を渡さなければ**ならない**
- **FR-513**: Docker環境への環境変数転送は既存のallowlist方式を維持しなければ**ならない**
- **FR-514**: GLMプロバイダー設定（ANTHROPIC_BASE_URL等）はOS環境変数の同名変数を上書きしなければ**ならない**
- **FR-515**: メニューに「Debug: Show Captured Environment」を追加し、環境変数一覧とソース情報を表示しなければ**ならない**

### 非機能要件

- **NFR-500**: 環境変数の取得はアプリ起動をブロックせず、非同期で実行しなければ**ならない**（UIの初期表示は即座に行われる）
- **NFR-501**: テストは unit test（モックベース）+ 手動確認で行う

## 影響範囲

### 変更対象ファイル

- `crates/gwt-core/src/config/os_env.rs` - 新規: ログインシェル環境変数取得モジュール
- `crates/gwt-core/src/terminal/runner.rs` - EnvSnapshot廃止・OS環境変数ベースに移行
- `crates/gwt-tauri/src/commands/terminal.rs` - `launch_agent` の環境変数マージロジック変更
- `crates/gwt-tauri/src/lib.rs` or `main.rs` - 起動時の非同期環境変数取得フック + グローバルステート
- `gwt-gui/` - 「Loading environment...」ステータス表示 + Debug メニュー追加

### 変更しないファイル

- `crates/gwt-core/src/docker/manager.rs` - Docker allowlistは現行維持
- `crates/gwt-core/src/config/profile.rs` - プロファイル構造は変更なし（disabled_env は既存）

## 範囲外

- GUIでの環境変数編集UI
- 環境変数の手動リフレッシュボタン（将来検討）
- Docker allowlistの拡張
