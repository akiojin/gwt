# TDD: gwt 起動時に前回のWindowを復元する

**仕様ID**: `SPEC-8f9b2d01`
**作成日**: 2026-02-17
**対象**: `gwt-gui/src/App.svelte`, `gwt-gui/src/lib/windowSessions.ts`, `crates/gwt-tauri/src/commands/window.rs`

## テスト方針

- Unit: windowセッション永続化ロジックの正規化・重複排除・更新/削除をまず保証
- Unit/統合: `open_gwt_window`/`get_current_window_label` の戻り値差分を吸収する実装を確認
- Manual: マルチウィンドウ起動時の復元シナリオを実機で再現し、同時起動衝突を確認

## テストケース

1. `persistWindowSessions` はラベル重複を後勝ちで扱い、空/無効値を保存しない
2. `getWindowSession` はlabel trim後一致で取得できる
3. `upsertWindowSession` は既存ラベルの重複を更新扱いし、`removeWindowSession` が確実に削除する
4. `normalize_window_label(None or whitespace)` は `project-` プレフィックスを持つ fallback を返す
5. 起動時にリーダーロックが取得できる場合のみ他Windowの復元が実行される（競合防止）

## 受け入れ確認（手動）

- 2つ以上のWindowを開き、別プロジェクトを起動後、アプリを終了して再起動すると前回Window分が復元される
- うち1Windowをクラッシュ中断させるなどして保存データが不正化しても、起動は継続され復元は部分的にのみ実施される
