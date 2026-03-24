### REDで先に追加するテスト

| ID | テスト | 対応FR |
|----|--------|--------|
| T-001 | managed skills から `gwt-project-index` が session available skills に含まれることを検証 | FR-001, FR-003 |
| T-002 | skill description に SPEC / Issue 検索用途が含まれることを検証 | FR-002 |
| T-003 | index 不在時の復旧案内またはコマンド引数が実装と一致することを検証 | FR-004 |

### 既存テストで回帰確認

| ID | テスト群 | 対応FR |
|----|----------|--------|
| T-010 | `skill_registration` 系既存テスト | FR-001, FR-003 |
| T-011 | command / asset 配布系の既存テスト | FR-005 |
