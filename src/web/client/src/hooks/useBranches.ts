/**
 * ブランチ関連のReact Hook
 */

import { useQuery } from "@tanstack/react-query";
import { branchApi } from "../lib/api";
import type { Branch } from "../../../../types/api.js";

/**
 * ブランチ一覧を取得
 */
export function useBranches() {
  return useQuery<Branch[]>({
    queryKey: ["branches"],
    queryFn: branchApi.list,
  });
}

/**
 * 特定のブランチ情報を取得
 */
export function useBranch(branchName: string) {
  return useQuery<Branch>({
    queryKey: ["branches", branchName],
    queryFn: () => branchApi.get(branchName),
    enabled: !!branchName,
  });
}
