# git.ts 分割計画

## 現状

`src/git.ts` は1,263行のモノリシックファイルで、以下の問題があります：

- 単一責任の原則違反
- テスタビリティの低下
- 関心の分離不足

## 分割案

### 1. git/repository.ts

リポジトリ基本操作

```typescript
export { isGitRepository, getRepositoryRoot, getWorktreeRoot, isInWorktree };
```

### 2. git/branch.ts

ブランチ操作

```typescript
export {
  getCurrentBranch,
  getLocalBranches,
  getRemoteBranches,
  getAllBranches,
  createBranch,
  branchExists,
  deleteBranch,
  deleteRemoteBranch,
  getCurrentBranchName,
  getBranchType,
};
```

### 3. git/changes.ts

変更操作

```typescript
export {
  hasUncommittedChanges,
  getChangedFilesCount,
  showStatus,
  stashChanges,
  discardAllChanges,
  commitChanges,
  getUncommittedChangesCount,
  getLatestCommitMessage,
};
```

### 4. git/sync.ts

同期操作

```typescript
export {
  hasUnpushedCommits,
  hasUnpushedCommitsInRepo,
  getUnpushedCommitsCount,
  pushBranchToRemote,
  fetchAllRemotes,
  pullFastForward,
  checkRemoteBranchExists,
};
```

### 5. git/divergence.ts

Divergence計算

```typescript
export {
  getBranchDivergenceStatuses,
  branchHasUniqueCommitsComparedToBase,
  collectUpstreamMap,
};
```

### 6. git/merge.ts

マージ操作

```typescript
export { mergeFromBranch, hasMergeConflict, abortMerge, getMergeStatus, resetToHead };
```

### 7. git/version.ts

バージョン操作

```typescript
export { getCurrentVersion, calculateNewVersion, executeNpmVersionInWorktree };
```

### 8. git/index.ts

再エクスポート（後方互換性維持）

```typescript
export * from "./repository.js";
export * from "./branch.js";
export * from "./changes.js";
export * from "./sync.js";
export * from "./divergence.js";
export * from "./merge.js";
export * from "./version.js";
export { GitError, BranchInfo, BranchDivergenceStatus } from "./types.js";
```

## 移行戦略

1. **Phase 1**: 型定義を `git/types.ts` に分離
2. **Phase 2**: 各モジュールを段階的に分離
3. **Phase 3**: `git/index.ts` で再エクスポートして後方互換性を維持
4. **Phase 4**: インポートを新しいパスに段階的に移行

## 注意事項

- 循環依存を避けるため、共通の型定義は `git/types.ts` に集約
- `GitError` クラスは `git/error.ts` に分離
- 既存のインポートパスは `git/index.ts` で互換性を維持
