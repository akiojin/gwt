# クイックスタート: シンプルターミナルタブ

**仕様ID**: `SPEC-f490dded` | **日付**: 2026-02-13

## 最小動作確認パス

以下の順序で実装すると最短で動作確認可能:

### Step 1: バックエンドの PTY 生成（最小）

1. `crates/gwt-core/src/terminal/manager.rs` に `spawn_shell()` メソッド追加
2. `crates/gwt-tauri/src/commands/terminal.rs` に `spawn_shell` コマンド追加
3. `crates/gwt-tauri/src/commands/mod.rs` にコマンド登録

### Step 2: メニューの追加

1. `crates/gwt-tauri/src/menu.rs` に "New Terminal" メニュー項目追加
2. `crates/gwt-tauri/src/app.rs` で `new-terminal` アクションをフロントエンドに転送

### Step 3: フロントエンドの最小タブ

1. `gwt-gui/src/lib/types.ts` に Tab.type = "terminal" を追加
2. `gwt-gui/src/App.svelte` に new-terminal ハンドラ追加（spawn_shell 呼出 → タブ作成）
3. `gwt-gui/src/lib/components/MainArea.svelte` に terminal タブのレンダリング追加

### Step 4: 動作確認

```bash
cargo tauri dev
```

1. Tools > New Terminal を選択
2. ターミナルタブが開き、シェルが操作可能であることを確認
3. `ls`, `cd /tmp` 等のコマンドが動作することを確認

## ビルド・テスト

```bash
# バックエンドテスト
cargo test -p gwt-core

# フロントエンドテスト
cd gwt-gui && npx vitest run

# Lint
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json
```
