### REDで先に追加するテスト

| ID | テスト | 対応FR |
|----|--------|--------|
| T-001 | crashing persisted DB を copy した manual harness で quarantine + rebuild + retry が成立する | FR-004 |
| T-002 | files index の runtime probe / index / search が正常系で成立する | FR-002 |
| T-003 | issue index の `index-issues` / `search-issues` が関連 `gwt-spec` Issue を返す | FR-003 |
| T-004 | `gwt-project-search` が available skills に露出し、Issue/SPEC 検索用途を説明する | FR-005, FR-007 |
| T-005 | spec 統合要求時に `gwt-project-search` を先に使う workflow 要件を検証できる | FR-006, FR-008 |
| T-006 | `index-issues` / `search-issues` が shared crash-recovery wrapper 経由で復旧する | FR-010 |

### 既存テスト / harness での確認

| ID | テスト群 | 対応FR |
|----|----------|--------|
| T-010 | `cargo test -p gwt-tauri commands::project_index::tests` | FR-002, FR-004 |
| T-011 | `manual_crash_recovery_recovers_copied_persisted_index_from_env` | FR-004 |
| T-012 | Issue semantic search 実行確認 | FR-003, FR-006 |
| T-013 | files crash recovery 後も issues search が継続できる regression | FR-004, FR-010 |
