import type {
  LaunchAgentRequest,
  LaunchFinishedPayload,
  LaunchProgressPayload,
} from "./types";

export type LaunchStepId =
  | "fetch"
  | "validate"
  | "paths"
  | "conflicts"
  | "create"
  | "skills"
  | "deps";

export function applyLaunchProgressRuntime(args: {
  payload: LaunchProgressPayload;
  launchStatus: string;
  launchStepIds: readonly LaunchStepId[];
  currentLaunchStep: LaunchStepId;
  setLaunchStep: (step: LaunchStepId) => void;
  setLaunchDetail: (detail: string) => void;
}): void {
  if (args.launchStatus !== "running") return;
  const next = args.payload.step as LaunchStepId;
  if (args.launchStepIds.includes(next) && next !== args.currentLaunchStep) {
    args.setLaunchStep(next);
  }
  args.setLaunchDetail((args.payload.detail ?? "").toString());
}

export function closeLaunchModalRuntime(args: {
  clearBufferedLaunchEvents: () => void;
  setLaunchProgressOpen: (open: boolean) => void;
  setLaunchJobId: (jobId: string) => void;
  setPendingLaunchRequest: (request: LaunchAgentRequest | null) => void;
  setLaunchStatus: (status: "running" | "ok" | "error") => void;
  setLaunchStep: (step: LaunchStepId) => void;
  setLaunchDetail: (detail: string) => void;
  setLaunchError: (error: string | null) => void;
}): void {
  args.setLaunchProgressOpen(false);
  args.setLaunchJobId("");
  args.setPendingLaunchRequest(null);
  args.clearBufferedLaunchEvents();
  args.setLaunchStatus("running");
  args.setLaunchStep("fetch");
  args.setLaunchDetail("");
  args.setLaunchError(null);
}

export function applyLaunchFinishedRuntime(args: {
  payload: LaunchFinishedPayload;
  pendingLaunchRequest: LaunchAgentRequest | null;
  parseE1004BranchName: (message: string) => string | null;
      setPendingLaunchRequest: (request: LaunchAgentRequest | null) => void;
  setLaunchStatus: (status: "running" | "ok" | "error") => void;
  setLaunchError: (error: string | null) => void;
  onCancelled: () => void;
  onSuccess: (paneId: string) => void;
}): void {
  if (args.payload.status === "cancelled") {
    args.onCancelled();
    return;
  }
  if (args.payload.status === "ok" && args.payload.paneId) {
    args.setLaunchStatus("ok");
    args.onSuccess(args.payload.paneId);
    args.onCancelled();
    return;
  }
  const error = args.payload.error || "Launch failed.";
  const recoveredBranch = args.parseE1004BranchName(error);
  if (recoveredBranch && args.pendingLaunchRequest) {
    args.setPendingLaunchRequest({
      ...args.pendingLaunchRequest,
      branch: recoveredBranch,
    });
  }
  args.setLaunchStatus("error");
  args.setLaunchError(error);
}

export function flushBufferedLaunchEventsRuntime(args: {
  launchJobId: string;
  bufferedLaunchProgressEvents: LaunchProgressPayload[];
  bufferedLaunchFinishedEvents: LaunchFinishedPayload[];
  clearBufferedLaunchEvents: () => void;
  applyLaunchProgressPayload: (payload: LaunchProgressPayload) => void;
  applyLaunchFinishedPayload: (payload: LaunchFinishedPayload) => void;
  getLaunchJobId: () => string;
}): void {
  if (!args.launchJobId) {
    args.clearBufferedLaunchEvents();
    return;
  }
  const activeJobId = args.launchJobId;

  for (const payload of args.bufferedLaunchProgressEvents) {
    if (payload.jobId !== activeJobId) continue;
    args.applyLaunchProgressPayload(payload);
  }

  for (const payload of args.bufferedLaunchFinishedEvents) {
    if (payload.jobId !== activeJobId) continue;
    args.applyLaunchFinishedPayload(payload);
    if (!args.getLaunchJobId() || args.getLaunchJobId() !== activeJobId) break;
  }

  args.clearBufferedLaunchEvents();
}

export function buildUseExistingBranchRetryRequest(
  pendingLaunchRequest: LaunchAgentRequest | null,
): LaunchAgentRequest | null {
  if (!pendingLaunchRequest) return null;
  const retryRequest: LaunchAgentRequest = { ...pendingLaunchRequest };
  delete retryRequest.createBranch;
  return retryRequest;
}
