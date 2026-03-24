### REDで先に追加するテスト

| ID | テスト | 対応FR |
|----|--------|--------|
| T-001 | `default` 不在 config を保存・読込すると `default` が補完される | FR-001 |
| T-002 | `profiles.default.ai = None` を保存・読込すると AI 設定が補完される | FR-002 |
| T-003 | 補完後も `profiles.default.ai.api_key == ""` が許容される | FR-003 |

### 既存テストで回帰確認

| ID | テスト群 | 対応FR |
|----|----------|--------|
| T-010 | `resolve_active_ai_settings_*` 既存テスト | FR-004 |
