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
  hasUnpushedCommits: boolean;
  worktreePath?: string | null;
  baseBranch?: string | null;
  divergence?: {
    ahead: number;
    behind: number;
    upToDate: boolean;
  } | null;
  prInfo?: {
    number: number;
    title: string;
    state: "open" | "merged" | "closed";
    mergedAt?: string | null;
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
  isProtected: boolean;
  createdAt?: string | null; // ISO8601
  lastAccessedAt?: string | null; // ISO8601
  divergence?: Branch["divergence"];
  prInfo?: Branch["prInfo"];
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
export interface EnvironmentVariable {
  key: string;
  value: string;
  lastUpdated?: string | null;
}

export interface EnvironmentHistoryEntry {
  key: string;
  action: "add" | "update" | "delete" | "import";
  timestamp: string;
  source: "ui" | "os" | "cli";
}

export interface CustomAITool {
  id: string; // UUID v4 or slug
  displayName: string;
  icon?: string | null;
  command: string;
  executionType: "path" | "bunx" | "command";
  defaultArgs?: string[] | null;
  modeArgs: {
    normal?: string[];
    continue?: string[];
    resume?: string[];
  };
  permissionSkipArgs?: string[] | null;
  env?: EnvironmentVariable[] | null;
  description?: string | null;
  createdAt?: string | null; // ISO8601
  updatedAt?: string | null; // ISO8601
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
export type BranchSyncResponse = SuccessResponse<BranchSyncResult>;
export type WorktreeListResponse = SuccessResponse<Worktree[]>;
export type WorktreeResponse = SuccessResponse<Worktree>;
export type SessionListResponse = SuccessResponse<AIToolSession[]>;
export type SessionResponse = SuccessResponse<AIToolSession>;
export interface ConfigPayload {
  version: string;
  updatedAt?: string | null;
  env?: EnvironmentVariable[] | null;
  history?: EnvironmentHistoryEntry[] | null;
  tools: CustomAITool[];
}

export type ConfigResponse = SuccessResponse<ConfigPayload>;

/**
 * API Request bodies
 */
export interface CreateWorktreeRequest {
  branchName: string;
  createBranch?: boolean;
}

export interface BranchSyncRequest {
  worktreePath: string;
}

export interface StartSessionRequest {
  toolType: "claude-code" | "codex-cli" | "custom";
  toolName?: string | null;
  mode: "normal" | "continue" | "resume";
  worktreePath: string;
  skipPermissions?: boolean;
  bypassApprovals?: boolean;
  extraArgs?: string[];
  customToolId?: string | null;
}

export type UpdateConfigRequest = ConfigPayload;

export interface CleanupResponse {
  success: true;
  deleted: string[];
}

export interface BranchSyncResult {
  branch: Branch;
  divergence?: Branch["divergence"];
  fetchStatus: "success";
  pullStatus: "success" | "failed";
  warnings?: string[];
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
