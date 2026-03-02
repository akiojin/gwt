import {
  type ClassifyResult,
  type DockerContext,
  type IssueBranchLookupState,
} from "../types";

export type RuntimeTarget = "host" | "docker";
export type BranchPrefix = "feature/" | "bugfix/" | "hotfix/" | "release/" | "";

export function supportsModelFor(agentId: string): boolean {
  return (
    agentId === "codex" ||
    agentId === "claude" ||
    agentId === "gemini" ||
    agentId === "opencode"
  );
}

export function toErrorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) {
    const msg = (err as { message?: unknown }).message;
    if (typeof msg === "string") return msg;
  }
  return String(err);
}

export function parseExtraArgs(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

export function parseEnvOverrides(
  text: string,
): { env: Record<string, string>; error: string | null } {
  const env: Record<string, string> = {};
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i += 1) {
    const raw = lines[i].trim();
    if (!raw || raw.startsWith("#")) continue;
    const idx = raw.indexOf("=");
    if (idx <= 0) {
      return {
        env: {},
        error: `Invalid env override at line ${i + 1}. Use KEY=VALUE.`,
      };
    }
    const key = raw.slice(0, idx).trim();
    const value = raw.slice(idx + 1).trimStart();
    if (!key) {
      return { env: {}, error: `Invalid env override at line ${i + 1}. Key is required.` };
    }
    env[key] = value;
  }
  return { env, error: null };
}

export function buildNewBranchName(prefix: BranchPrefix, suffix: string): string {
  const s = suffix.trim();
  if (!s) return "";
  return `${prefix}${s}`;
}

export function splitBranchNamePrefix(
  input: string,
  prefixes: readonly BranchPrefix[],
): { prefix: BranchPrefix; suffix: string } | null {
  const trimmed = input.trim();
  for (const prefix of prefixes) {
    if (trimmed.startsWith(prefix)) {
      return { prefix, suffix: trimmed.slice(prefix.length) };
    }
  }
  return null;
}

export function dockerStatusHint(
  runtimeTarget: RuntimeTarget,
  dockerContext: DockerContext | null,
): string {
  if (runtimeTarget !== "docker") return "";
  const imgStatus = dockerContext?.images_exist;
  const ctrStatus = dockerContext?.container_status;
  if (imgStatus == null || ctrStatus == null) return "";
  const imgLabel = imgStatus ? "Images ready" : "No images";
  const ctrLabel =
    ctrStatus === "running"
      ? "Containers running"
      : ctrStatus === "stopped"
        ? "Containers stopped"
        : "No containers";
  const actions: string[] = [];
  if (!imgStatus) actions.push("build");
  if (ctrStatus === "not_found") actions.push("create");
  else if (ctrStatus === "stopped") actions.push("recreate");
  const suffix = actions.length > 0 ? ` - will ${actions.join(" and ")} automatically` : "";
  return `${imgLabel} / ${ctrLabel}${suffix}`;
}

export type DockerSelectionInput = {
  context: DockerContext | null;
  pendingRuntimePreference: RuntimeTarget | null;
  pendingDockerServicePreference: string;
  dockerService: string;
};

export type DockerSelectionResult = {
  runtimeTarget: RuntimeTarget;
  dockerService: string;
  pendingRuntimePreference: RuntimeTarget | null;
  pendingDockerServicePreference: string;
};

export function resolveDockerContextSelection(
  input: DockerSelectionInput,
): DockerSelectionResult {
  const ctx = input.context;
  const reset: Pick<DockerSelectionResult, "pendingRuntimePreference" | "pendingDockerServicePreference"> =
    {
      pendingRuntimePreference: null,
      pendingDockerServicePreference: "",
    };

  if (!ctx || ctx.force_host || ctx.file_type === "none") {
    return {
      runtimeTarget: "host",
      dockerService: "",
      ...reset,
    };
  }

  const services = ctx.compose_services ?? [];
  const composeLike =
    ctx.file_type === "compose" ||
    (ctx.file_type === "devcontainer" && services.length > 0);

  if (composeLike) {
    const canUseDocker = ctx.docker_available && ctx.compose_available;
    const runtimeTarget =
      input.pendingRuntimePreference === "host"
        ? "host"
        : input.pendingRuntimePreference === "docker" && canUseDocker
          ? "docker"
          : canUseDocker
            ? "docker"
            : "host";

    if (services.length === 0) {
      return {
        runtimeTarget,
        dockerService: "",
        ...reset,
      };
    }

    const preferredService = input.pendingDockerServicePreference.trim();
    const dockerService =
      preferredService && services.includes(preferredService)
        ? preferredService
        : services.includes(input.dockerService)
          ? input.dockerService
          : services[0];

    return {
      runtimeTarget,
      dockerService,
      ...reset,
    };
  }

  const runtimeTarget =
    input.pendingRuntimePreference === "host"
      ? "host"
      : input.pendingRuntimePreference === "docker" && ctx.docker_available
        ? "docker"
        : ctx.docker_available
          ? "docker"
          : "host";

  return {
    runtimeTarget,
    dockerService: "",
    ...reset,
  };
}

export function isIssueSelectable(
  issueNumber: number,
  issueBranchChecksInFlight: ReadonlySet<number>,
  issueBranchMap: ReadonlyMap<number, IssueBranchLookupState>,
): boolean {
  if (issueBranchChecksInFlight.has(issueNumber)) return false;
  if (!issueBranchMap.has(issueNumber)) return false;
  return issueBranchMap.get(issueNumber) === null;
}

export function canLaunchFromIssue(
  issueNumber: number | null | undefined,
  issueBranchChecksInFlight: ReadonlySet<number>,
  issueBranchMap: ReadonlyMap<number, IssueBranchLookupState>,
): boolean {
  if (issueNumber == null) return false;
  return isIssueSelectable(issueNumber, issueBranchChecksInFlight, issueBranchMap);
}

export function shouldLoadMoreIssues(
  scrollHeight: number,
  scrollTop: number,
  clientHeight: number,
  threshold: number,
  issuesHasNextPage: boolean,
  issuesLoading: boolean,
  issueRateLimited: boolean,
): boolean {
  const atBottom = scrollHeight - scrollTop - clientHeight < threshold;
  return atBottom && issuesHasNextPage && !issuesLoading && !issueRateLimited;
}

export function isStaleIssueClassifyRequest(
  reqId: number,
  classifyRequestId: number,
  selectedIssueNumber: number | null | undefined,
  issueNumber: number,
): boolean {
  return reqId !== classifyRequestId || selectedIssueNumber !== issueNumber;
}

export function classifyIssuePrefix(
  result: ClassifyResult,
  validPrefixes: readonly BranchPrefix[],
): BranchPrefix | "" {
  if (result.status !== "ok" || !result.prefix) return "";
  const prefix = `${result.prefix}/` as BranchPrefix;
  return validPrefixes.includes(prefix) ? prefix : "";
}
