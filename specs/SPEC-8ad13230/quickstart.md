# クイックスタート: SPEC-8ad13230

**仕様ID**: `SPEC-8ad13230` | **日付**: 2026-02-17

## 1. 主要コマンド（ローカル検証）

```bash
cargo check
cargo test -p gwt-core issue_spec -- --nocapture
cargo test -p gwt-tauri commands::issue_spec::tests -- --nocapture
cargo test -p gwt-tauri agent_master::tests -- --nocapture
cargo test -p gwt-tauri agent_tools::tests -- --nocapture
python3 -m py_compile scripts/gwt_issue_spec_mcp.py
```

## 2. 内蔵ツール（Master Agent）

- `upsert_spec_issue`
- `get_spec_issue`
- `upsert_spec_issue_artifact`
- `list_spec_issue_artifacts`
- `delete_spec_issue_artifact`
- `sync_spec_issue_project`

## 3. MCP ツール

- `spec_issue_upsert`
- `spec_issue_get`
- `spec_issue_artifact_upsert`
- `spec_issue_artifact_list`
- `spec_issue_artifact_delete`
- `spec_project_sync`

## 4. 期待フロー

1. `spec_issue_upsert` で bundle section を生成/更新する。
2. `spec_issue_artifact_upsert` で `contracts/*` と `checklists/*` を登録する。
3. `spec_project_sync` で Project V2 の phase を同期する。
