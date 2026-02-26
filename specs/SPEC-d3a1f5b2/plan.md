# 実装計画: Cmd+Q 二重確認によるアプリ終了

**仕様ID**: `SPEC-d3a1f5b2` | **日付**: 2026-02-26 | **仕様書**: `specs/SPEC-d3a1f5b2/spec.md`

## 目的

- Cmd+Q / Alt+F4 の誤操作でアプリが即座に終了することを防止する
- Chrome 風の二重確認トースト（「Press ⌘Q again to quit」）を導入する
- 従来のエージェント実行中ダイアログを廃止し、トースト方式に統一する

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **テスト**: cargo test / vitest
- **前提**: Tauri の `RunEvent::ExitRequested` で `api.prevent_exit()` が正しく動作すること

## 実装方針

### Phase 1: バックエンド - 終了シーケンス状態管理

1. **AppState に quit_confirm タイマー状態を追加** (`crates/gwt-tauri/src/state.rs`)
   - `quit_confirm_requested_at: Mutex<Option<Instant>>` を追加
   - 1回目の ExitRequested で現在時刻を記録
   - 2回目の ExitRequested で3秒以内なら `request_quit()` → `app.exit(0)`

2. **ExitRequested ハンドラの書き換え** (`crates/gwt-tauri/src/app.rs`)
   - macOS 固有のエージェント警告ダイアログロジックを削除
   - 全プラットフォーム統一の二重確認ロジックに置換
   - 1回目: `api.prevent_exit()` + ウィンドウにイベント emit (`quit-confirm-show`)
   - 2回目（3秒以内）: `request_quit()` + `app.exit(0)`
   - ウィンドウ非表示時: ウィンドウを再表示してからイベント emit

3. **リセット用コマンド追加** (`crates/gwt-tauri/src/commands/`)
   - `cancel_quit_confirm` コマンド: フロントエンドから呼ばれ、quit_confirm 状態をリセット

### Phase 2: フロントエンド - トーストコンポーネント

1. **QuitConfirmToast コンポーネント作成** (`gwt-gui/src/lib/components/QuitConfirmToast.svelte`)
   - `quit-confirm-show` イベントリスナー
   - ウィンドウ上部中央に固定配置
   - gwt デザイントークンを使用（`--bg-surface`, `--text-primary`, `--border-color`）
   - 3秒タイマーで自動非表示 + バックエンドにリセット通知
   - フェードイン/フェードアウトアニメーション

2. **他操作検知によるリセット** (`gwt-gui/src/lib/components/QuitConfirmToast.svelte`)
   - トースト表示中に `mousedown`, `keydown`（Cmd+Q 以外）を検知
   - 検知したら即座にトーストを非表示 + バックエンドにリセット通知

3. **App.svelte への組み込み** (`gwt-gui/src/App.svelte`)
   - QuitConfirmToast コンポーネントをルートレベルに配置

### Phase 3: クリーンアップ

1. **従来のエージェント警告ダイアログの削除**
   - `app.rs` の macOS 限定ダイアログロジック削除
   - `exit_confirm_inflight` フィールドの廃止（不要になるため）
   - `try_begin_exit_confirm` / `end_exit_confirm` 関数の削除

## テスト

### バックエンド

- `quit_confirm_requested_at` の状態管理テスト（設定/リセット/タイムアウト判定）
- ExitRequested ハンドラの二重確認ロジックテスト（3秒以内/以降）
- `cancel_quit_confirm` コマンドのリセットテスト

### フロントエンド

- QuitConfirmToast コンポーネントのレンダリングテスト
- イベント受信でトーストが表示されるテスト
- 3秒後の自動非表示テスト
- マウスクリック/キー入力でのリセットテスト
- プラットフォーム別メッセージ表示テスト（⌘Q vs Alt+F4）
