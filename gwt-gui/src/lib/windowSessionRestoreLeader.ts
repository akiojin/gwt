export const WINDOW_SESSION_RESTORE_LEAD_KEY =
  "gwt.windowSessions.restoreLeader.v1";
export const WINDOW_SESSION_RESTORE_LEAD_TTL_MS = 15_000;

export type WindowSessionRestoreLeaderState = {
  label: string;
  expiresAt: number;
};

function normalizeLabel(label: string): string {
  return label.trim();
}

export function isWindowSessionRestoreLeaderCandidate(label: string): boolean {
  return normalizeLabel(label) === "main";
}

export function readWindowSessionRestoreLeader(
  storage: Storage,
): WindowSessionRestoreLeaderState | null {
  try {
    const raw = storage.getItem(WINDOW_SESSION_RESTORE_LEAD_KEY);
    if (!raw) return null;

    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;

    const candidate = parsed as {
      label?: unknown;
      expiresAt?: unknown;
    };
    const label =
      typeof candidate.label === "string" ? candidate.label.trim() : "";
    const expiresAtRaw = candidate.expiresAt;
    const expiresAt =
      typeof expiresAtRaw === "number" && Number.isFinite(expiresAtRaw)
        ? Math.floor(expiresAtRaw)
        : NaN;
    if (!label || !Number.isFinite(expiresAt)) return null;

    return { label, expiresAt };
  } catch {
    return null;
  }
}

export function tryAcquireWindowSessionRestoreLead(
  storage: Storage,
  label: string,
  now = Date.now(),
): boolean {
  const normalizedLabel = normalizeLabel(label);
  if (!isWindowSessionRestoreLeaderCandidate(normalizedLabel)) {
    return false;
  }

  try {
    const existing = readWindowSessionRestoreLeader(storage);
    if (
      existing &&
      existing.label.length > 0 &&
      existing.expiresAt > now &&
      existing.label !== normalizedLabel
    ) {
      return false;
    }

    const next: WindowSessionRestoreLeaderState = {
      label: normalizedLabel,
      expiresAt: now + WINDOW_SESSION_RESTORE_LEAD_TTL_MS,
    };
    storage.setItem(WINDOW_SESSION_RESTORE_LEAD_KEY, JSON.stringify(next));
    return true;
  } catch {
    return false;
  }
}

export function releaseWindowSessionRestoreLead(storage: Storage, label: string) {
  const normalizedLabel = normalizeLabel(label);
  if (!normalizedLabel) return;

  try {
    const existing = readWindowSessionRestoreLeader(storage);
    if (!existing || existing.label !== normalizedLabel) return;
    storage.removeItem(WINDOW_SESSION_RESTORE_LEAD_KEY);
  } catch {
    // Ignore storage failures.
  }
}
