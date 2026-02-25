# タスク: SPEC-1238

## タスク一覧

- [x] T1: 仕様書作成（spec.md / plan.md / tasks.md）
- [x] T2: テスト追加（TDD - RED → GREEN）
- [x] T3: `update.rs` 実装
- [x] T4: `release.yml` ワークフロー修正
- [x] T5: 検証（cargo test / cargo clippy）

## 依存関係

- T2 → T3（テストファースト）
- T3, T4 は独立（並行可能）
- T5 は T3, T4 完了後
