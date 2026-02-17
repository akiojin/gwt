import type { WorkflowRunInfo } from "./types";

export function workflowStatusIcon(run: WorkflowRunInfo): string {
  if (run.status !== "completed") {
    return run.status === "in_progress" ? "\u25C9" : "\u25CB";
  }
  switch (run.conclusion) {
    case "success":
      return "\u2713";
    case "failure":
      return "\u2717";
    case "neutral":
    case "skipped":
      return "\u2014";
    default:
      return "?";
  }
}

export function workflowStatusClass(run: WorkflowRunInfo): string {
  if (run.status === "in_progress") return "running";
  if (run.status === "queued") return "pending";
  if (run.status !== "completed") return "pending";
  switch (run.conclusion) {
    case "success":
      return "pass";
    case "failure":
      return "fail";
    default:
      return "neutral";
  }
}
