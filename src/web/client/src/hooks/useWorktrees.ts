/**
 * Worktree関連のReact Hook
 */

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { worktreeApi } from "../lib/api";
import type { Worktree, CreateWorktreeRequest } from "../../../../types/api.js";

/**
 * Worktree一覧を取得
 */
export function useWorktrees() {
  return useQuery<Worktree[]>({
    queryKey: ["worktrees"],
    queryFn: worktreeApi.list,
  });
}

/**
 * Worktreeを作成
 */
export function useCreateWorktree() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: CreateWorktreeRequest) => worktreeApi.create(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["worktrees"] });
      queryClient.invalidateQueries({ queryKey: ["branches"] });
    },
  });
}

/**
 * Worktreeを削除
 */
export function useDeleteWorktree() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (path: string) => worktreeApi.delete(path),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["worktrees"] });
      queryClient.invalidateQueries({ queryKey: ["branches"] });
    },
  });
}
