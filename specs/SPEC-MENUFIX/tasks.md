# タスク: SPEC-MENUFIX

## タスク一覧

- [x] T-1: バックエンド -- EventTarget importの追加 (app.rs)
- [x] T-2: バックエンド -- emit() を emit_to() に変更 (app.rs)
- [x] T-3: フロントエンド -- listen() をウィンドウスコープに変更 (App.svelte)
- [x] T-4: cargo clippy による検証
- [x] T-5: svelte-check による検証
- [ ] T-6: 手動テスト -- 複数ウィンドウでのメニューアクション動作確認

## 依存関係

- T-4 は T-1, T-2 に依存
- T-5 は T-3 に依存
- T-6 は T-4, T-5 に依存
