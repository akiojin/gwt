### REDで先に追加するテスト

| ID | テスト | 対応FR |
|----|--------|--------|
| T-001 | managed skills から `gwt-issue-spec-ops` が session available skills に含まれることを検証 | FR-001, FR-003 |
| T-002 | skill description に SPEC / Issue / TDD 用途が含まれることを検証 | FR-002 |
| T-003 | SPEC作成要求時に `gwt-issue-spec-ops` が汎用 spec スキルより優先されることを検証 | FR-004 |
| T-004 | `gwt-spec` Issue テンプレート構造が repo 運用と一致することを検証 | FR-005 |

### 既存テストで回帰確認

| ID | テスト群 | 対応FR |
|----|----------|--------|
| T-010 | `skill_registration` 系既存テスト | FR-001, FR-003 |
| T-011 | command / asset 配布系の既存テスト | FR-005 |
