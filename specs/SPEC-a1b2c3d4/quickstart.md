# クイックスタート: SPEC-a1b2c3d4

## 前提条件

- Rust stable toolchain
- Node.js + pnpm
- Tauri CLI (`cargo tauri`)

## ビルド確認

```bash
# バックエンドビルド
cargo build

# フロントエンドビルド
cd gwt-gui && pnpm install && pnpm build

# 統合ビルド
cargo tauri build
```

## テスト実行

```bash
# バックエンドテスト
cargo test

# フロントエンドテスト
cd gwt-gui && pnpm test

# Lint
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json
```

## 動作確認手順

### 統計記録の確認

1. `cargo tauri dev` でアプリ起動
2. エージェントを起動する
3. `~/.gwt/stats.toml` の内容を確認
4. 起動回数がインクリメントされていること

### ステータスバーの確認

1. アプリ起動後、ステータスバー右側に CPU/MEM が表示される
2. 1秒間隔で値が更新される
3. 70%以上で黄色、90%以上で赤に変化

### About ダイアログの確認

1. メニュー → About で General タブが開く
2. ステータスバーの CPU/MEM をクリックで System タブが開く
3. Statistics タブでエージェント起動回数テーブルが表示される
4. リポジトリフィルタドロップダウンが動作する

## 新規ファイル一覧

| ファイル | 説明 |
|---|---|
| `crates/gwt-core/src/config/stats.rs` | 統計データ load/save/increment |
| `crates/gwt-core/src/system_info.rs` | SystemMonitor（sysinfo + nvml-wrapper） |
| `crates/gwt-tauri/src/commands/system.rs` | get_system_info / get_stats コマンド |
| `gwt-gui/src/lib/systemMonitor.ts` | フロントエンドポーリング |
| `gwt-gui/src/lib/components/AboutDialog.svelte` | About 3タブコンポーネント |

## 変更ファイル一覧

| ファイル | 変更内容 |
|---|---|
| `crates/gwt-core/Cargo.toml` | sysinfo, nvml-wrapper 依存追加 |
| `crates/gwt-core/src/lib.rs` | system_info モジュール追加 |
| `crates/gwt-core/src/config/mod.rs` | stats モジュール追加 |
| `crates/gwt-tauri/src/state.rs` | AppState に SystemMonitor 追加 |
| `crates/gwt-tauri/src/commands/mod.rs` | system コマンド登録 |
| `crates/gwt-tauri/src/commands/terminal.rs` | 起動/WT作成時の統計記録フック |
| `gwt-gui/src/App.svelte` | AboutDialog 置き換え、systemMonitor 統合 |
| `gwt-gui/src/lib/components/StatusBar.svelte` | CPU/MEM 表示追加 |
| `gwt-gui/src/styles/global.css` | ステータスバー高さ 28px |
