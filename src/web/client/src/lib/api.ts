/**
 * REST API Client
 *
 * バックエンドのREST APIと通信するクライアント。
 * 仕様: specs/SPEC-d5e56259/contracts/rest-api.yaml
 */

import type {
  Branch,
  Worktree,
  AIToolSession,
  HealthResponse,
  CreateWorktreeRequest,
  StartSessionRequest,
  UpdateConfigRequest,
  BranchSyncRequest,
  BranchSyncResult,
  ConfigPayload,
} from "../../../../types/api.js";

const API_BASE = "/api";

/**
 * APIエラー
 */
export class ApiError extends Error {
  constructor(
    message: string,
    public statusCode: number,
    public details?: string | null,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

/**
 * fetchラッパー - エラーハンドリングとJSON解析
 */
async function apiFetch<T>(url: string, options?: RequestInit): Promise<T> {
  const response = await fetch(url, {
    ...options,
    headers: {
      "Content-Type": "application/json",
      ...options?.headers,
    },
  });

  const data = await response.json();

  if (!response.ok || !data.success) {
    throw new ApiError(
      data.error || "API request failed",
      response.status,
      data.details,
    );
  }

  return data.data;
}

/**
 * ヘルスチェック
 */
export async function checkHealth(): Promise<HealthResponse> {
  const response = await fetch(`${API_BASE}/health`);
  return response.json();
}

/**
 * ブランチAPI
 */
export const branchApi = {
  /**
   * すべてのブランチ一覧を取得
   */
  list: async (): Promise<Branch[]> => {
    return apiFetch<Branch[]>(`${API_BASE}/branches`);
  },

  /**
   * 特定のブランチ情報を取得
   */
  get: async (branchName: string): Promise<Branch> => {
    const encoded = encodeURIComponent(branchName);
    return apiFetch<Branch>(`${API_BASE}/branches/${encoded}`);
  },

  sync: async (
    branchName: string,
    payload: BranchSyncRequest,
  ): Promise<BranchSyncResult> => {
    const encoded = encodeURIComponent(branchName);
    return apiFetch<BranchSyncResult>(`${API_BASE}/branches/${encoded}/sync`, {
      method: "POST",
      body: JSON.stringify(payload),
    });
  },
};

/**
 * WorktreeAPI
 */
export const worktreeApi = {
  /**
   * すべてのWorktree一覧を取得
   */
  list: async (): Promise<Worktree[]> => {
    return apiFetch<Worktree[]>(`${API_BASE}/worktrees`);
  },

  /**
   * 新しいWorktreeを作成
   */
  create: async (request: CreateWorktreeRequest): Promise<Worktree> => {
    return apiFetch<Worktree>(`${API_BASE}/worktrees`, {
      method: "POST",
      body: JSON.stringify(request),
    });
  },

  /**
   * Worktreeを削除
   */
  delete: async (path: string): Promise<void> => {
    const url = new URL(`${API_BASE}/worktrees/delete`, window.location.origin);
    url.searchParams.set("path", path);
    await apiFetch<void>(url.toString(), {
      method: "DELETE",
    });
  },
};

/**
 * セッションAPI
 */
export const sessionApi = {
  /**
   * すべてのセッション一覧を取得
   */
  list: async (): Promise<AIToolSession[]> => {
    return apiFetch<AIToolSession[]>(`${API_BASE}/sessions`);
  },

  /**
   * 新しいセッションを開始
   */
  start: async (request: StartSessionRequest): Promise<AIToolSession> => {
    return apiFetch<AIToolSession>(`${API_BASE}/sessions`, {
      method: "POST",
      body: JSON.stringify(request),
    });
  },

  /**
   * 特定のセッション情報を取得
   */
  get: async (sessionId: string): Promise<AIToolSession> => {
    return apiFetch<AIToolSession>(`${API_BASE}/sessions/${sessionId}`);
  },

  /**
   * セッションを終了
   */
  delete: async (sessionId: string): Promise<void> => {
    await apiFetch<void>(`${API_BASE}/sessions/${sessionId}`, {
      method: "DELETE",
    });
  },
};

/**
 * 設定API
 */
export const configApi = {
  /**
   * カスタムAI Tool設定を取得
   */
  get: async (): Promise<ConfigPayload> => {
    return apiFetch<ConfigPayload>(`${API_BASE}/config`);
  },

  /**
   * カスタムAI Tool設定を更新
   */
  update: async (request: UpdateConfigRequest): Promise<ConfigPayload> => {
    return apiFetch<ConfigPayload>(`${API_BASE}/config`, {
      method: "PUT",
      body: JSON.stringify(request),
    });
  },
};
