export const AGENT_LAUNCH_DEFAULTS_STORAGE_KEY = "gwt.launchAgentDefaults.v1";

export type LaunchDefaultsSessionMode = "normal" | "continue" | "resume";
export type LaunchDefaultsRuntimeTarget = "host" | "docker";

export type LaunchDefaults = {
  selectedAgent: string;
  sessionMode: LaunchDefaultsSessionMode;
  modelByAgent: Record<string, string>;
  agentVersionByAgent: Record<string, string>;
  skipPermissions: boolean;
  reasoningLevel: string;
  resumeSessionId: string;
  showAdvanced: boolean;
  extraArgsText: string;
  envOverridesText: string;
  runtimeTarget: LaunchDefaultsRuntimeTarget;
  dockerService: string;
  dockerBuild: boolean;
  dockerRecreate: boolean;
  dockerKeep: boolean;
};

type StoredLaunchDefaults = {
  version: 1;
  data: LaunchDefaults;
};

function getStorageSafe(storage?: Storage | null): Storage | null {
  try {
    if (storage) return storage;
    if (typeof window === "undefined") return null;
    return window.localStorage;
  } catch {
    return null;
  }
}

function normalizeString(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function normalizeSessionMode(value: unknown): LaunchDefaultsSessionMode {
  const mode = normalizeString(value);
  if (mode === "continue" || mode === "resume") return mode;
  return "normal";
}

function normalizeRuntimeTarget(value: unknown): LaunchDefaultsRuntimeTarget {
  return normalizeString(value) === "docker" ? "docker" : "host";
}

function sanitizeStringRecord(value: unknown): Record<string, string> {
  if (!value || typeof value !== "object") return {};
  const raw = value as Record<string, unknown>;
  const next: Record<string, string> = {};
  for (const [k, v] of Object.entries(raw)) {
    const key = normalizeString(k);
    const val = normalizeString(v);
    if (!key || !val) continue;
    next[key] = val;
  }
  return next;
}

function sanitizeLaunchDefaults(value: unknown): LaunchDefaults {
  const raw = value && typeof value === "object" ? (value as Record<string, unknown>) : {};
  return {
    selectedAgent: normalizeString(raw.selectedAgent),
    sessionMode: normalizeSessionMode(raw.sessionMode),
    modelByAgent: sanitizeStringRecord(raw.modelByAgent),
    agentVersionByAgent: sanitizeStringRecord(raw.agentVersionByAgent),
    skipPermissions: raw.skipPermissions === true,
    reasoningLevel: normalizeString(raw.reasoningLevel),
    resumeSessionId: normalizeString(raw.resumeSessionId),
    showAdvanced: raw.showAdvanced === true,
    extraArgsText: normalizeString(raw.extraArgsText),
    envOverridesText: normalizeString(raw.envOverridesText),
    runtimeTarget: normalizeRuntimeTarget(raw.runtimeTarget),
    dockerService: normalizeString(raw.dockerService),
    dockerBuild: raw.dockerBuild === true,
    dockerRecreate: raw.dockerRecreate === true,
    dockerKeep: raw.dockerKeep === true,
  };
}

export function loadLaunchDefaults(storage?: Storage | null): LaunchDefaults | null {
  const store = getStorageSafe(storage);
  if (!store) return null;

  try {
    const raw = store.getItem(AGENT_LAUNCH_DEFAULTS_STORAGE_KEY);
    if (!raw) return null;

    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;
    const root = parsed as Partial<StoredLaunchDefaults>;
    if (root.version !== 1) return null;

    return sanitizeLaunchDefaults(root.data);
  } catch {
    return null;
  }
}

export function saveLaunchDefaults(defaults: LaunchDefaults, storage?: Storage | null): void {
  const store = getStorageSafe(storage);
  if (!store) return;

  const sanitized = sanitizeLaunchDefaults(defaults);
  const payload: StoredLaunchDefaults = { version: 1, data: sanitized };

  try {
    store.setItem(AGENT_LAUNCH_DEFAULTS_STORAGE_KEY, JSON.stringify(payload));
  } catch {
    // Best-effort persistence only.
  }
}
