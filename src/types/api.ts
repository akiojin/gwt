/**
 * Web UI API型定義
 *
 * データモデル仕様: specs/SPEC-d5e56259/data-model.md
 * REST API仕様: specs/SPEC-d5e56259/contracts/rest-api.yaml
 */

/**
 * Branch - Gitブランチ
 */
export interface Branch {
  name: string;
  type: "local" | "remote";
  commitHash: string;
  commitMessage?: string | null;
  author?: string | null;
  commitDate?: string | null; // ISO8601
  mergeStatus: "unmerged" | "merged" | "unknown";
  worktreePath?: string | null;
  baseBranch?: string | null;
  divergence?: {
    ahead: number;
    behind: number;
    upToDate: boolean;
  } | null;
}

/**
 * Worktree - Gitワークツリー
 */
export interface Worktree {
  path: string;
  branchName: string;
  head: string;
  isLocked: boolean;
  isPrunable: boolean;
  createdAt?: string | null; // ISO8601
  lastAccessedAt?: string | null; // ISO8601
}

/**
 * AIToolSession - AI Tool実行セッション
 */
export interface AIToolSession {
  sessionId: string; // UUID v4
  toolType: "claude-code" | "codex-cli" | "custom";
  toolName?: string | null;
  mode: "normal" | "continue" | "resume";
  worktreePath: string;
  ptyPid?: number | null;
  websocketId?: string | null;
  status: "pending" | "running" | "completed" | "failed";
  startedAt: string; // ISO8601
  endedAt?: string | null; // ISO8601
  exitCode?: number | null;
  errorMessage?: string | null;
}

/**
 * CustomAITool - カスタムAI Tool設定
 */
export interface CustomAITool {
  id: string; // UUID v4
  name: string;
  command: string;
  executionType: "path" | "bunx" | "command";
  defaultArgs?: string[] | null;
  env?: Record<string, string> | null;
  description?: string | null;
  createdAt: string; // ISO8601
  updatedAt: string; // ISO8601
}

/**
 * REST API Response wrappers
 */
export interface SuccessResponse<T = unknown> {
  success: true;
  data: T;
}

export interface ErrorResponse {
  success: false;
  error: string;
  details?: string | null;
}

export type ApiResponse<T> = SuccessResponse<T> | ErrorResponse;

/**
 * API Endpoints
 */
export interface HealthResponse {
  success: true;
  status: string;
  timestamp: string; // ISO8601
}

export type BranchListResponse = SuccessResponse<Branch[]>;
export type BranchResponse = SuccessResponse<Branch>;
export type WorktreeListResponse = SuccessResponse<Worktree[]>;
export type WorktreeResponse = SuccessResponse<Worktree>;
export type SessionListResponse = SuccessResponse<AIToolSession[]>;
export type SessionResponse = SuccessResponse<AIToolSession>;
export type ConfigResponse = SuccessResponse<{ tools: CustomAITool[] }>;

/**
 * API Request bodies
 */
export interface CreateWorktreeRequest {
  branchName: string;
  createBranch?: boolean;
}

export interface StartSessionRequest {
  toolType: "claude-code" | "codex-cli" | "custom";
  toolName?: string | null;
  mode: "normal" | "continue" | "resume";
  worktreePath: string;
  skipPermissions?: boolean;
  bypassApprovals?: boolean;
}

export interface UpdateConfigRequest {
  tools: CustomAITool[];
}

export interface CleanupResponse {
  success: true;
  deleted: string[];
}

/**
 * WebSocket Messages
 */
export interface WebSocketMessage {
  type: string;
  data?: unknown;
  timestamp?: string; // ISO8601
}

export interface InputMessage extends WebSocketMessage {
  type: "input";
  data: string;
}

export interface ResizeMessage extends WebSocketMessage {
  type: "resize";
  data: {
    cols: number;
    rows: number;
  };
}

export interface PingMessage extends WebSocketMessage {
  type: "ping";
}

export interface OutputMessage extends WebSocketMessage {
  type: "output";
  data: string;
}

export interface ExitMessage extends WebSocketMessage {
  type: "exit";
  data: {
    code: number;
    signal?: string;
  };
}

export interface ErrorMessage extends WebSocketMessage {
  type: "error";
  data: {
    message: string;
    code?: string;
  };
}

export interface PongMessage extends WebSocketMessage {
  type: "pong";
}

export type ClientMessage = InputMessage | ResizeMessage | PingMessage;
export type ServerMessage =
  | OutputMessage
  | ExitMessage
  | ErrorMessage
  | PongMessage;
