import type {
  LaunchAgentRequest,
  LaunchProgressPayload,
  LaunchFinishedPayload,
} from "./types";

export type LaunchStepId =
  | "fetch"
  | "validate"
  | "paths"
  | "conflicts"
  | "create"
  | "skills"
  | "deps";

export const LAUNCH_STEP_IDS: LaunchStepId[] = [
  "fetch",
  "validate",
  "paths",
  "conflicts",
  "create",
  "skills",
  "deps",
];

export const LAUNCH_EVENT_BUFFER_LIMIT = 64;

export interface LaunchState {
  progressOpen: boolean;
  jobId: string;
  step: LaunchStepId;
  detail: string;
  status: "running" | "ok" | "error" | "cancelled";
  error: string | null;
  pendingRequest: LaunchAgentRequest | null;
  jobStartPending: boolean;
  bufferedProgressEvents: LaunchProgressPayload[];
  bufferedFinishedEvents: LaunchFinishedPayload[];
  docsEditorAutoClosePaneIds: string[];
}

export function createLaunchState(): LaunchState {
  return {
    progressOpen: false,
    jobId: "",
    step: "fetch",
    detail: "",
    status: "running",
    error: null,
    pendingRequest: null,
    jobStartPending: false,
    bufferedProgressEvents: [],
    bufferedFinishedEvents: [],
    docsEditorAutoClosePaneIds: [],
  };
}

export function bufferLaunchProgressEventRuntime(
  state: LaunchState,
  payload: LaunchProgressPayload,
): void {
  if (state.bufferedProgressEvents.length >= LAUNCH_EVENT_BUFFER_LIMIT) {
    state.bufferedProgressEvents.shift();
  }
  state.bufferedProgressEvents.push(payload);
}

export function bufferLaunchFinishedEventRuntime(
  state: LaunchState,
  payload: LaunchFinishedPayload,
): void {
  if (state.bufferedFinishedEvents.length >= LAUNCH_EVENT_BUFFER_LIMIT) {
    state.bufferedFinishedEvents.shift();
  }
  state.bufferedFinishedEvents.push(payload);
}

export function resetLaunchStateRuntime(state: LaunchState): void {
  state.progressOpen = false;
  state.jobId = "";
  state.step = "fetch";
  state.detail = "";
  state.status = "running";
  state.error = null;
  state.pendingRequest = null;
  state.jobStartPending = false;
  state.bufferedProgressEvents = [];
  state.bufferedFinishedEvents = [];
  state.docsEditorAutoClosePaneIds = [];
}
