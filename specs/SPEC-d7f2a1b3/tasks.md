# タスク: SPEC-d7f2a1b3

## タスク一覧

- [x] T1: `WorktreeInfo` シリアライズの snake_case 検証テストを追加
- [x] T2: `WorktreesChangedPayload` シリアライズの snake_case 検証テストを追加
- [x] T3: `WorktreeInfo` から `#[serde(rename_all = "camelCase")]` を削除
- [x] T4: `WorktreesChangedPayload` から `#[serde(rename_all = "camelCase")]` を削除
- [x] T5: `cargo test` / `cargo clippy` / `npx vitest run` で全検証パス
