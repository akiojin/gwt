import type { MergedPullRequest, CleanupTarget } from "../../src/cli/ui/types";

/**
 * テスト用のマージ済みPRデータ
 */
export const mergedPullRequests: MergedPullRequest[] = [
  {
    number: 123,
    title: "Add user authentication feature",
    branch: "feature/user-auth",
    author: "Akira",
    mergedAt: "2025-01-01T10:00:00Z",
  },
  {
    number: 124,
    title: "Fix security vulnerability",
    branch: "hotfix/security-patch",
    author: "Sana",
    mergedAt: "2025-01-02T15:30:00Z",
  },
  {
    number: 125,
    title: "Update dashboard UI",
    branch: "feature/dashboard",
    author: "Noa",
    mergedAt: "2025-01-03T09:15:00Z",
  },
];

/**
 * テスト用のクリーンアップ対象データ
 */
export const cleanupTargets: CleanupTarget[] = [
  {
    branch: "feature/user-auth",
    worktreePath: "/path/to/worktree-feature-user-auth",
    pullRequest: {
      number: 123,
      title: "Add user authentication feature",
      branch: "feature/user-auth",
      mergedAt: "2025-01-01T10:00:00Z",
      author: "alice",
    },
    hasUncommittedChanges: false,
    hasUnpushedCommits: false,
    hasRemoteBranch: true,
    cleanupType: "worktree-and-branch",
    reasons: ["remote-synced"],
  },
  {
    branch: "hotfix/security-patch",
    worktreePath: null, // ワークツリーなし
    pullRequest: {
      number: 124,
      title: "Fix security vulnerability",
      branch: "hotfix/security-patch",
      mergedAt: "2025-01-02T15:30:00Z",
      author: "bob",
    },
    hasUncommittedChanges: false,
    hasUnpushedCommits: false,
    hasRemoteBranch: true,
    cleanupType: "branch-only",
    reasons: ["remote-synced"],
  },
  {
    branch: "feature/dashboard",
    worktreePath: "/path/to/worktree-feature-dashboard",
    pullRequest: {
      number: 125,
      title: "Update dashboard UI",
      branch: "feature/dashboard",
      mergedAt: "2025-01-03T09:15:00Z",
      author: "carol",
    },
    hasUncommittedChanges: false,
    hasUnpushedCommits: true, // 未プッシュコミットあり
    hasRemoteBranch: false,
    cleanupType: "worktree-and-branch",
    reasons: ["remote-synced"],
  },
];
