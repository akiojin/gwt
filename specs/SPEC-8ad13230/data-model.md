# データモデル: SPEC-8ad13230

**仕様ID**: `SPEC-8ad13230` | **日付**: 2026-02-17

## 1. SpecIssueSections

- `spec: String`
- `plan: String`
- `tasks: String`
- `tdd: String`
- `research: String`
- `data_model: String`
- `quickstart: String`
- `contracts: String`（概要/運用ルール）
- `checklists: String`（概要/運用ルール）

## 2. SpecIssueArtifactComment

- `comment_id: String`（GitHub Node ID）
- `issue_number: u64`
- `kind: contract | checklist`
- `artifact_name: String`（例: `api.md`, `requirements.md`）
- `content: String`
- `updated_at: String`
- `etag: String`（`updated_at:content_length`）
- `url: Option<String>`

## 3. 命名規則

- contracts: `contract:<artifact_name>`
- checklists: `checklist:<artifact_name>`
- marker: `<!-- GWT_SPEC_ARTIFACT:<kind>:<artifact_name> -->`

## 4. 競合制御

- 更新/削除は `expected_etag` を受け付ける。
- etag 不一致時はエラーを返し、呼び出し側で再取得を要求する。
