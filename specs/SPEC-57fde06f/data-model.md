# Data Model: releaseブランチ経由の自動リリース＆Auto Mergeフロー

| Entity | Description | Key Fields | Relationships |
| --- | --- | --- | --- |
| `ReleaseBranchState` | develop から同期された release ブランチの最新情報 | `headSha`, `sourceSha` (develop), `lastSemanticReleaseTag`, `status` (clean/dirty) | `/release` コマンドが develop HEAD を読み取り release に書き込む。`ReleasePullRequest` が参照。
| `ReleasePullRequest` | release→main の単一 PR。Auto Merge と Required チェックの状態を保持 | `number`, `title`, `autoMerge` (enabled/disabled), `requiredChecks` (array), `lastUpdated` | `ReleaseBranchState` から生成。`RequiredCheck` を内包し main Branch Protection と連動。
| `RequiredCheck` | Auto Merge を許可するために必須の CI ジョブ | `name` (e.g. `lint`, `test`, `semantic-release`), `status`, `url` | GitHub Actions workflow run と 1:1 対応し、`ReleasePullRequest.requiredChecks` 配列に含まれる。
| `BranchProtectionConfig` | main ブランチの保護条件 | `directPushAllowed` (bool), `autoMergeAllowed`, `requiredChecks` | Release PR の Auto Merge 条件を決定。`ReleasePullRequest` 作成時に前提として確認。
| `ReleaseAutomationCommand` | `/release` 実行時の入力/出力スキーマ | `runBy`, `timestamp`, `developSha`, `releaseSha`, `prUrl`, `ghWorkflowUrl` | CLI 実行ログに保存され、スクリプト・ドキュメント双方で参照。

## State Transitions

1. **Sync Stage**
   - develop でリリース対象コミットを作成 → `/release` が `git fetch origin release` → `git push origin develop:release` を実行。
   - `ReleaseBranchState.headSha` が `developSha` と一致すると state=`clean`。

2. **CI Stage**
   - release ブランチ push → `release.yml` が起動 → `lint`/`test`/`semantic-release` ジョブが走り、`RequiredCheck.status` を success/failure に更新。
   - semantic-release 成功時にタグと GitHub Release、npm publish が行われ、`ReleaseBranchState.lastSemanticReleaseTag` が更新。

3. **PR Stage**
   - `/release` が release→main PR を作成/更新し `autoMerge=enabled`。PR 本文にリリースノートと CI リンクを記載。
   - Required チェック成功で Auto Merge が main にコミットを取り込み、PR は close 状態になる。

4. **Post-Merge Stage**
   - main が release と同じ SHA になる → develop に main を逆マージするかどうかは既存ポリシーに従う。
   - `ReleaseBranchState.status` は `waiting`（次回リリース待ち）へ遷移。

## Validation Rules

- release ブランチには必ず 1 件の Release PR が紐づく（複数存在した場合は `/release` が古いものを close する）。
- `RequiredCheck` は Branch Protection に登録されたジョブ名と一致していなければならない。
- semantic-release がタグを作成したら release→main PR の本文にタグとリリースノートリンクを含める。
- main への直接 push を検出した場合は CI でエラーを出し、ドキュメントに従い release フローへ誘導する。
