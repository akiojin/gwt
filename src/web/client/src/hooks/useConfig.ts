import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { configApi } from "../lib/api";
import type {
  ConfigPayload,
  UpdateConfigRequest,
} from "../../../../types/api.js";

const QUERY_KEY = ["config"] as const;

export function useConfig() {
  return useQuery<ConfigPayload>({
    queryKey: QUERY_KEY,
    queryFn: configApi.get,
  });
}

export function useUpdateConfig() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (payload: UpdateConfigRequest) => configApi.update(payload),
    onSuccess: (data: ConfigPayload) => {
      queryClient.setQueryData(QUERY_KEY, data);
    },
  });
}
