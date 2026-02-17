# TDD記録: Window復元時の無限生成防止

## 対象

- `crates/gwt-tauri/src/state.rs`
- `gwt-gui/src/lib/windowSessionRestoreLeader.ts`

## RED（失敗条件の定義）

1. 復元リーダー取得
   - `main` 以外で取得できてしまう挙動を失敗条件と定義
   - 有効期限内の他ラベル保持中に `main` が奪取できてしまう挙動を失敗条件と定義
2. 復元リーダー解放
   - 不一致ラベルで解放されてしまう挙動を失敗条件と定義
3. フロント呼び出し契約
   - non-main で backend command を実行してしまう挙動を失敗条件と定義
   - command失敗時に false/ignore へフォールバックしない挙動を失敗条件と定義

## GREEN（実装）

1. `AppState` に復元リーダーロックを追加し、main限定 + TTL + 一致解放の仕様を実装
2. Tauri command で `try_acquire/release` を公開
3. フロントを commandベースに切替し、復元処理を `await` で直列化
4. 現在Windowラベルは `get_current_window_label` command を唯一の経路に統一

## REFACTOR

- フロントの `localStorage` ベース実装を廃止し、責務を backend lock に集約
- テストを commandラッパ契約中心に整理し、将来の復元ロジック変更に対して壊れにくい構成へ更新

## 実行ログ（最終）

- `cargo clippy --all-targets --all-features -- -D warnings` ✅
- `cargo test -p gwt-tauri -- --nocapture` ✅
- `pnpm test src/lib/windowSessionRestoreLeader.test.ts` ✅
- `pnpm exec svelte-check --tsconfig ./tsconfig.json` ✅
