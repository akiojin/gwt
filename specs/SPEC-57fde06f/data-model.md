# Data Model: releaseブランチ経由の自動リリース＆Auto Mergeフロー

| Entity | Description | Key Fields | Relationships |
| --- | --- | --- | --- |
| `ReleaseBranchState` | develop と release/vX.Y.Z の整合状態 | `developSha`, `releaseSha`, `status`(in-sync/outdated), `lastVersion` | `/release` コマンドが更新し、`ReleaseWorkflowRun` が参照する。
| `ReleaseWorkflowRun` | `release.yml` の実行結果 | `runId`, `version`, `semanticReleaseStatus`, `logsUrl` | 成功時に release ブランチを main へマージし、タグ/リリース情報を残す。
| `PublishWorkflowRun` | `publish.yml` の実行結果 | `runId`, `npmPublished`, `backmergeStatus`, `logsUrl` | `release.yml` 後に走り、`develop` へのバックマージと npm publish を管理。
| `BranchProtectionConfig` | main ブランチの保護条件 | `directPushAllowed`, `requiredChecks`, `enforceAdmins` | release automation の前提。CI が失敗すると main 更新をブロック。
| `ReleaseCommandInvocation` | `/release` または helper script の実行情報 | `invokedBy`, `timestamp`, `developSha`, `releaseBranch`, `workflowRunUrl` | `create-release.yml` を起動するたびに記録され、監査に利用。

## State Transitions

1. **Sync Stage**
   - `/release` または helper script が `create-release.yml` を起動し、semantic-release dry-run で次バージョンを決定して `release/vX.Y.Z` を push。
   - `ReleaseBranchState.releaseSha` が `developSha` と一致した時点で `status=in-sync`。

2. **Release Stage**
   - `release.yml` が起動し、lint/test/semantic-release を実行。成功すると version を記録し、release ブランチを main へマージして削除する。
   - 失敗した場合は main が更新されず、`ReleaseWorkflowRun.semanticReleaseStatus=failed` のまま保持される。

3. **Publish Stage**
   - main への push が `publish.yml` を起動。`npm publish`（任意）と `main` → `develop` のバックマージを実施し、結果を `PublishWorkflowRun` に記録。

4. **Idle Stage**
   - すべて完了後、`ReleaseBranchState.status` は `waiting` に戻り、次の `/release` を待つ。

## Validation Rules

- release/vX.Y.Z ブランチは 1 リリースにつき 1 度だけ生成され、`release.yml` 成功後に必ず削除する。
- `ReleaseWorkflowRun.version` が存在する場合、Git タグ `v${version}` と GitHub Release が必須。
- `PublishWorkflowRun.backmergeStatus` が `failed` の場合は develop を手動で同期し、workflow を再実行するまで新規リリースを行わない。
- main への直接 push を検出した場合は Branch Protection が拒否し、CLAUDE.md で案内している release フローに従う。
