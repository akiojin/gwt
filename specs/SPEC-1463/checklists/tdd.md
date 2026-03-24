### REDで先に追加するテスト

| ID | テスト | 対応FR |
|----|--------|--------|
| T-001 | `OPENAI_API_KEY` 未設定 + active profile `ai.api_key` 非空で Codex 認証 true | FR-001, FR-002 |
| T-002 | env と `ai.api_key` 両方空で Codex 認証 false | FR-001, FR-005 |
| T-003 | Launch env で `OPENAI_API_KEY` 未設定時に `ai.api_key` が注入される | FR-003 |
| T-004 | `profile.env.OPENAI_API_KEY` がある場合は上書きされない | FR-004 |

### 既存テストで回帰確認

| ID | テスト群 | 対応FR |
|----|----------|--------|
| T-010 | `merge_profile_env` 系既存テスト | FR-004 |
| T-011 | `settings-config.spec.ts` 既存 Profiles シナリオ | FR-005 |
