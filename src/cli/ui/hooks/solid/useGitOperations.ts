import { createMemo, createSignal, type Accessor } from "solid-js";
import type { AsyncState } from "../../core/types.js";
import type { WorktreeConfig } from "../../types.js";
import {
  createWorktree,
  generateWorktreePath,
  removeWorktree,
} from "../../../../worktree.js";
import { deleteBranch, getRepositoryRoot } from "../../../../git.js";

export interface UseGitOperationsResult {
  state: Accessor<AsyncState<unknown>>;
  isLoading: Accessor<boolean>;
  error: Accessor<Error | null>;
  run: <T>(operation: () => Promise<T>) => Promise<T>;
  reset: () => void;
  getRepositoryRoot: () => Promise<string>;
  generateWorktreePath: (
    repoRoot: string,
    branchName: string,
  ) => Promise<string>;
  createWorktree: (config: WorktreeConfig) => Promise<void>;
  removeWorktree: (worktreePath: string, force?: boolean) => Promise<void>;
  deleteBranch: (branchName: string, force?: boolean) => Promise<void>;
}

const toError = (err: unknown): Error =>
  err instanceof Error ? err : new Error(String(err));

/**
 * Solid hook to perform git/worktree operations with shared async state.
 */
export function useGitOperations(): UseGitOperationsResult {
  const [state, setState] = createSignal<AsyncState<unknown>>({
    status: "idle",
  });

  const isLoading = createMemo(() => state().status === "loading");
  const error = createMemo(() => {
    const current = state();
    return current.status === "error" ? current.error : null;
  });

  const run = async <T>(operation: () => Promise<T>): Promise<T> => {
    setState({ status: "loading" });
    try {
      const result = await operation();
      setState({ status: "success", data: result });
      return result;
    } catch (err) {
      const errorValue = toError(err);
      setState({ status: "error", error: errorValue });
      throw errorValue;
    }
  };

  const reset = () => setState({ status: "idle" });

  const runGetRepositoryRoot = () => run(() => getRepositoryRoot());
  const runGenerateWorktreePath = (repoRoot: string, branchName: string) =>
    run(() => generateWorktreePath(repoRoot, branchName));
  const runCreateWorktree = (config: WorktreeConfig) =>
    run(() => createWorktree(config));
  const runRemoveWorktree = (worktreePath: string, force = false) =>
    run(() => removeWorktree(worktreePath, force));
  const runDeleteBranch = (branchName: string, force = false) =>
    run(() => deleteBranch(branchName, force));

  return {
    state,
    isLoading,
    error,
    run,
    reset,
    getRepositoryRoot: runGetRepositoryRoot,
    generateWorktreePath: runGenerateWorktreePath,
    createWorktree: runCreateWorktree,
    removeWorktree: runRemoveWorktree,
    deleteBranch: runDeleteBranch,
  };
}
