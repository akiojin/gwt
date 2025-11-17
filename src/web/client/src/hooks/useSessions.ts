/**
 * セッション関連のReact Hook
 */

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { sessionApi } from "../lib/api";
import type {
  AIToolSession,
  StartSessionRequest,
} from "../../../../types/api.js";

/**
 * セッション一覧を取得
 */
export function useSessions() {
  return useQuery<AIToolSession[]>({
    queryKey: ["sessions"],
    queryFn: sessionApi.list,
  });
}

/**
 * 特定のセッション情報を取得
 */
export function useSession(sessionId: string) {
  return useQuery<AIToolSession>({
    queryKey: ["sessions", sessionId],
    queryFn: () => sessionApi.get(sessionId),
    enabled: !!sessionId,
  });
}

/**
 * セッションを開始
 */
export function useStartSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (request: StartSessionRequest) => sessionApi.start(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sessions"] });
    },
  });
}

/**
 * セッションを終了
 */
export function useDeleteSession() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (sessionId: string) => sessionApi.delete(sessionId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sessions"] });
    },
  });
}
