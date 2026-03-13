import type { OpenProjectResult, ProbePathResult } from "./types";
import {
  getWindowSession,
  removeWindowSession,
  upsertWindowSession,
} from "./windowSessions";

type InvokeFn = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

type StaleRestoreReason =
  | "emptyDir"
  | "invalid"
  | "notFound"
  | "notGwtProject";

export type RestoreCurrentWindowSessionResult =
  | { kind: "noSession" }
  | { kind: "opened"; result: OpenProjectResult }
  | { kind: "focusedExisting"; focusedWindowLabel: string | null }
  | { kind: "migrationRequired"; sourceRoot: string }
  | { kind: "stale"; reason: StaleRestoreReason }
  | { kind: "error"; message: string };

export type OpenAndNormalizeRestoredWindowSessionResult =
  | { kind: "noSession" }
  | { kind: "opened"; openedLabel: string }
  | { kind: "migrationRequired"; sourceRoot: string }
  | { kind: "stale"; reason: StaleRestoreReason }
  | { kind: "error"; message: string };

function normalizeText(value: unknown): string | null {
  const text = typeof value === "string" ? value.trim() : "";
  return text || null;
}

function toErrorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) {
    const message = (err as { message?: unknown }).message;
    if (typeof message === "string") return message;
  }
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

async function resolveInvoke(invokeFn?: InvokeFn): Promise<InvokeFn> {
  if (invokeFn) return invokeFn;
  const { invoke } = await import("$lib/tauriInvoke");
  return invoke;
}

function isStaleProbeKind(kind: string): kind is StaleRestoreReason {
  return (
    kind === "emptyDir" ||
    kind === "invalid" ||
    kind === "notFound" ||
    kind === "notGwtProject"
  );
}

function clearSession(label: string, storage?: Storage | null) {
  if (!label.trim()) return;
  removeWindowSession(label, storage);
}

export async function restoreCurrentWindowSession(
  label: string,
  invokeFn?: InvokeFn,
  storage?: Storage | null,
): Promise<RestoreCurrentWindowSessionResult> {
  const normalizedLabel = normalizeText(label);
  if (!normalizedLabel) {
    return { kind: "noSession" };
  }

  const session = getWindowSession(normalizedLabel, storage);
  if (!session?.projectPath) {
    clearSession(normalizedLabel, storage);
    return { kind: "noSession" };
  }

  const invoke = await resolveInvoke(invokeFn);

  try {
    const probe = await invoke<ProbePathResult>("probe_path", {
      path: session.projectPath,
    });

    if (probe.kind === "gwtProject" && normalizeText(probe.projectPath)) {
      const result = await invoke<OpenProjectResult>("open_project", {
        path: normalizeText(probe.projectPath),
      });

      if (result.action === "focusedExisting") {
        clearSession(normalizedLabel, storage);
        return {
          kind: "focusedExisting",
          focusedWindowLabel: normalizeText(result.focusedWindowLabel),
        };
      }

      return { kind: "opened", result };
    }

    if (
      probe.kind === "migrationRequired" &&
      normalizeText(probe.migrationSourceRoot)
    ) {
      clearSession(normalizedLabel, storage);
      return {
        kind: "migrationRequired",
        sourceRoot: normalizeText(probe.migrationSourceRoot)!,
      };
    }

    if (isStaleProbeKind(probe.kind)) {
      clearSession(normalizedLabel, storage);
      return { kind: "stale", reason: probe.kind };
    }

    clearSession(normalizedLabel, storage);
    return {
      kind: "error",
      message: probe.message || "Failed to restore project.",
    };
  } catch (err) {
    clearSession(normalizedLabel, storage);
    return { kind: "error", message: toErrorMessage(err) };
  }
}

export async function openAndNormalizeRestoredWindowSession(
  label: string,
  invokeFn?: InvokeFn,
  storage?: Storage | null,
): Promise<OpenAndNormalizeRestoredWindowSessionResult> {
  const normalizedLabel = normalizeText(label);
  if (!normalizedLabel) {
    return { kind: "noSession" };
  }

  const session = getWindowSession(normalizedLabel, storage);
  if (!session?.projectPath) {
    clearSession(normalizedLabel, storage);
    return { kind: "noSession" };
  }

  const invoke = await resolveInvoke(invokeFn);

  try {
    const probe = await invoke<ProbePathResult>("probe_path", {
      path: session.projectPath,
    });

    if (probe.kind === "gwtProject" && normalizeText(probe.projectPath)) {
      const openedLabelRaw = await invoke<unknown>("open_gwt_window", {
        label: normalizedLabel,
      });
      const openedLabel = normalizeText(openedLabelRaw) ?? normalizedLabel;
      if (openedLabel !== normalizedLabel) {
        clearSession(normalizedLabel, storage);
        upsertWindowSession(openedLabel, normalizeText(probe.projectPath)!, storage);
      }
      return { kind: "opened", openedLabel };
    }

    if (
      probe.kind === "migrationRequired" &&
      normalizeText(probe.migrationSourceRoot)
    ) {
      clearSession(normalizedLabel, storage);
      return {
        kind: "migrationRequired",
        sourceRoot: normalizeText(probe.migrationSourceRoot)!,
      };
    }

    if (isStaleProbeKind(probe.kind)) {
      clearSession(normalizedLabel, storage);
      return { kind: "stale", reason: probe.kind };
    }

    clearSession(normalizedLabel, storage);
    return {
      kind: "error",
      message: probe.message || "Failed to restore window.",
    };
  } catch (err) {
    clearSession(normalizedLabel, storage);
    return { kind: "error", message: toErrorMessage(err) };
  }
}
