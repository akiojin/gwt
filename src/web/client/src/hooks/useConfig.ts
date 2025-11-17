/**
 * Config Hooks
 */

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { configApi } from "../lib/api";
import type { ConfigPayload } from "../../../../types/api.js";

/**
 * カスタムAIツール設定を取得
 */
export function useConfig() {
  return useQuery<ConfigPayload>({
    queryKey: ["config"],
    queryFn: configApi.get,
  });
}

/**
 * カスタムAIツール設定を更新
 */
export function useUpdateConfig() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: configApi.update,
    onSuccess: (data: ConfigPayload) => {
      queryClient.setQueryData(["config"], data);
    },
  });
}
