import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { configApi } from "../lib/api";
import type { CustomAITool } from "../../../../types/api.js";

const QUERY_KEY = ["config"] as const;

export function useConfig() {
  return useQuery({
    queryKey: QUERY_KEY,
    queryFn: configApi.get,
  });
}

export function useUpdateConfig() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (tools: CustomAITool[]) => configApi.update({ tools }),
    onSuccess: (data) => {
      queryClient.setQueryData(QUERY_KEY, data);
    },
  });
}
