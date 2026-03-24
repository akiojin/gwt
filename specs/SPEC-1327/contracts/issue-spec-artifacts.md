# Contract: Issue-first Spec Artifacts

**仕様ID**: `SPEC-8ad13230` | **日付**: 2026-02-17

## Built-in tool contract

### upsert_spec_issue_artifact

- input:
  - `issue_number: number`
  - `kind: "contract" | "checklist"`
  - `artifact_name: string`
  - `content: string`
  - `expected_etag?: string`
- output:
  - `commentId`, `kind`, `artifactName`, `content`, `updatedAt`, `etag`

### list_spec_issue_artifacts

- input:
  - `issue_number: number`
  - `kind?: "contract" | "checklist"`
- output:
  - `SpecIssueArtifactComment[]`

### delete_spec_issue_artifact

- input:
  - `issue_number: number`
  - `kind: "contract" | "checklist"`
  - `artifact_name: string`
  - `expected_etag?: string`
- output:
  - `{ "deleted": boolean }`

## MCP tool contract

### spec_issue_artifact_upsert

- built-in `upsert_spec_issue_artifact` と同等。

### spec_issue_artifact_list

- built-in `list_spec_issue_artifacts` と同等。

### spec_issue_artifact_delete

- built-in `delete_spec_issue_artifact` と同等。

## Backward compatibility

- `spec_contract_comment_append` は `kind=contract` の upsert エイリアスとして維持する。
