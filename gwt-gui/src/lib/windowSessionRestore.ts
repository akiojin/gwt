import type { OpenProjectResult, ProbePathResult } from "./types";
import {
  getWindowSession,
  renameWindowSession,
  removeWindowSession,
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

/**
 * Result of restoring the current startup window session.
 *
 * `opened` means `open_project` succeeded for the probed path, `focusedExisting`
 * means another window already owned the project, `migrationRequired` means the
 * stored path must be migrated before opening, `stale` means the stored entry is
 * invalid and was removed, `error` means a transient invoke failure occurred and
 * the stored session was preserved for a later retry, and `noSession` means no
 * usable session was available.
 */
export type RestoreCurrentWindowSessionResult =
  | { kind: "noSession" }
  | { kind: "opened"; result: OpenProjectResult }
  | { kind: "focusedExisting"; focusedWindowLabel: string | null }
  | { kind: "migrationRequired"; sourceRoot: string }
  | { kind: "stale"; reason: StaleRestoreReason }
  | { kind: "error"; message: string };

/**
 * Result of opening a secondary restored window during startup recovery.
 *
 * `opened` means a new or normalized window label is ready, `migrationRequired`
 * means the stored project must be migrated first, `stale` means the stored
 * entry was invalid and removed, `error` means a transient invoke failure
 * occurred and the session was preserved, and `noSession` means there was no
 * usable stored session for the requested label.
 */
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

/**
 * Restore the current window from the stored startup session.
 *
 * This may remove stale sessions, keep transient failures retriable, or surface
 * migration/focused-existing outcomes without directly mutating App state.
 */
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
    return { kind: "error", message: toErrorMessage(err) };
  }
}

/**
 * Open and normalize a secondary restored window session during startup.
 *
 * This may relabel the stored session atomically, remove stale entries, or keep
 * transient failures retriable on the next app launch.
 */
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
        renameWindowSession(
          normalizedLabel,
          openedLabel,
          normalizeText(probe.projectPath)!,
          storage,
        );
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
    return { kind: "error", message: toErrorMessage(err) };
  }
}
