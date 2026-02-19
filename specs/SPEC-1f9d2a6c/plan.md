# 実装計画: Window復元時の無限生成防止

**仕様ID**: `SPEC-1f9d2a6c` | **日付**: 2026-02-17 | **仕様書**: `specs/SPEC-1f9d2a6c/spec.md`

## 目的

- 起動時のWindow復元で、同一Windowが連鎖的に生成される不具合を根本的に解消する
- 復元リーダー制御をフロント依存からバックエンド原子制御へ移し、単一リーダー保証を強化する
- 回帰防止テストを追加し、同種の復元暴走を自動検知できる状態にする

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **復元関連実装**:
  - `gwt-gui/src/App.svelte`
  - `gwt-gui/src/lib/windowSessionRestoreLeader.ts`
  - `crates/gwt-tauri/src/state.rs`
  - `crates/gwt-tauri/src/commands/window.rs`

## 実装方針

### Phase 1: バックエンド主導の復元リーダーロック

1. `AppState` に `window_session_restore_leader`（`Mutex<Option<...>>`）を追加
2. `main` のみ取得可能な `try_acquire_window_session_restore_leader` を実装
3. TTL付きロック更新・期限切れ再取得を実装
4. `release_window_session_restore_leader` を実装し、ラベル一致時のみ解放

### Phase 2: Tauri command公開

1. `commands/window.rs` に下記 command を追加
   - `try_acquire_window_restore_leader`
   - `release_window_restore_leader`
2. `app.rs` の `generate_handler!` に command を登録

### Phase 3: フロント復元フローの安全化

1. `windowSessionRestoreLeader.ts` を commandラッパへ移行
2. `App.svelte` の復元処理を `await tryAcquire...` / `await release...` へ切替
3. 現在Windowラベル取得を `get_current_window_label` のみに統一し、内部メタデータ fallback を削除

### Phase 4: テスト・検証

1. バックエンド:
   - 取得条件（main限定）
   - アクティブ他ラベルでの拒否
   - 期限切れ再取得
   - 解放条件
2. フロント:
   - non-main で command未実行
   - command呼び出し引数の妥当性
   - 失敗時フォールバック

## リスクと軽減策

| ID | リスク | 影響 | 軽減策 |
| --- | --- | --- | --- |
| RISK-001 | release失敗でロック残留 | 次回復元が抑止される | TTLで自動回復 |
| RISK-002 | ラベル取得失敗 | 復元漏れ | 復元best-effortで安全中断 |
| RISK-003 | フロント/バックの契約不一致 | 復元失敗 | command呼び出し単体テストを追加 |

## 検証コマンド

- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test -p gwt-tauri -- --nocapture`
- `cd gwt-gui && pnpm test src/lib/windowSessionRestoreLeader.test.ts`
- `cd gwt-gui && pnpm exec svelte-check --tsconfig ./tsconfig.json`
