# Data Model

## Ref inventory
- `BranchRefEntry`: `id`, `canonicalName`, `hasLocal`, `hasRemote`, `localName?`, `remoteName?`, `upstream?`, `ahead`, `behind`, `divergenceStatus`, `isCurrent`, `isGone`
- `BranchBrowserFilterMode`: `local`, `remote`, `all`
- `RefInventorySnapshot`: `projectPath`, `generatedAt`, `entries`, `primaryBranch`

## Worktree instance
- `WorktreeInstance`: `id`, `path`, `branchName`, `commit`, `isCurrent`, `isProtected`, `isGone`, `displayName?`, `linkedIssue?`, `lastToolUsage?`, `safetyLevel`
- `BranchInventoryDetail`: `branchId`, `displayName?`, `linkedIssue?`, `lastToolUsage?`, `safetyLevel?`

## Local Git backend services
- `InventoryInvalidationReason`: `repoStateChanged`, `worktreeCreated`, `worktreeDeleted`, `branchMetadataChanged`, `manualRefresh`
- consumer request -> `RefInventorySnapshot | WorktreeInstance | BranchInventoryDetail`

## Resolution rules
- selected ref -> `ExistingWorktreeInstance | CreateWorktreeAction`
- execution session -> `worktreeInstanceId`
