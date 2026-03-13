export const WINDOW_SESSIONS_STORAGE_KEY = "gwt.windowSessions.v1";

export interface WindowSessionEntry {
  label: string;
  projectPath: string;
}

function getWindowSessionsStorage(storage?: Storage | null): Storage | null {
  if (storage) return storage;
  if (typeof window === "undefined") return null;

  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function normalizeLabel(value: unknown): string {
  const raw = typeof value === "string" ? value.trim() : "";
  return raw;
}

function normalizeProjectPath(value: unknown): string {
  const raw = typeof value === "string" ? value.trim() : "";
  return raw;
}

function sanitizeEntry(raw: unknown): WindowSessionEntry | null {
  if (!raw || typeof raw !== "object") return null;

  const entry = raw as {
    label?: unknown;
    projectPath?: unknown;
  };

  const label = normalizeLabel(entry.label);
  const projectPath = normalizeProjectPath(entry.projectPath);

  if (!label || !projectPath) {
    return null;
  }

  return { label, projectPath };
}

function normalizeEntries(value: unknown): WindowSessionEntry[] {
  if (!Array.isArray(value)) return [];

  const byLabel = new Map<string, WindowSessionEntry>();

  for (const raw of value) {
    const entry = sanitizeEntry(raw);
    if (!entry) continue;
    byLabel.set(entry.label, entry);
  }

  return Array.from(byLabel.values());
}

export function loadWindowSessions(storage?: Storage | null): WindowSessionEntry[] {
  const store = getWindowSessionsStorage(storage);
  if (!store) return [];

  try {
    const raw = store.getItem(WINDOW_SESSIONS_STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    return normalizeEntries(parsed);
  } catch {
    return [];
  }
}

export function persistWindowSessions(
  sessions: WindowSessionEntry[],
  storage?: Storage | null,
) {
  const store = getWindowSessionsStorage(storage);
  if (!store) return;

  try {
    const normalized = normalizeEntries(
      sessions.filter(
        (entry): entry is WindowSessionEntry =>
          normalizeLabel(entry.label).length > 0 &&
          normalizeProjectPath(entry.projectPath).length > 0,
      ),
    );
    store.setItem(
      WINDOW_SESSIONS_STORAGE_KEY,
      JSON.stringify(normalized),
    );
  } catch {
    // Ignore storage failures.
  }
}

export function getWindowSession(
  label: string,
  storage?: Storage | null,
): WindowSessionEntry | null {
  const normalizedLabel = normalizeLabel(label);
  if (!normalizedLabel) return null;

  const sessions = loadWindowSessions(storage);
  return sessions.find((session) => session.label === normalizedLabel) ?? null;
}

export function upsertWindowSession(
  label: string,
  projectPath: string,
  storage?: Storage | null,
) {
  const normalizedLabel = normalizeLabel(label);
  const normalizedPath = normalizeProjectPath(projectPath);
  if (!normalizedLabel || !normalizedPath) return;

  const sessions = loadWindowSessions(storage).filter(
    (session) => session.label !== normalizedLabel,
  );
  sessions.push({ label: normalizedLabel, projectPath: normalizedPath });
  persistWindowSessions(sessions, storage);
}

export function renameWindowSession(
  oldLabel: string,
  newLabel: string,
  projectPath: string,
  storage?: Storage | null,
) {
  const normalizedOldLabel = normalizeLabel(oldLabel);
  const normalizedNewLabel = normalizeLabel(newLabel);
  const normalizedPath = normalizeProjectPath(projectPath);
  if (!normalizedOldLabel || !normalizedNewLabel || !normalizedPath) return;

  const sessions = loadWindowSessions(storage).filter(
    (session) =>
      session.label !== normalizedOldLabel &&
      session.label !== normalizedNewLabel,
  );
  sessions.push({
    label: normalizedNewLabel,
    projectPath: normalizedPath,
  });
  persistWindowSessions(sessions, storage);
}

export function deduplicateByProjectPath(
  sessions: WindowSessionEntry[],
): WindowSessionEntry[] {
  const seen = new Set<string>();
  return sessions.filter((entry) => {
    if (seen.has(entry.projectPath)) return false;
    seen.add(entry.projectPath);
    return true;
  });
}

export function pruneWindowSessions(storage?: Storage | null): void {
  const sessions = loadWindowSessions(storage);
  const cleaned = deduplicateByProjectPath(sessions);
  if (cleaned.length < sessions.length) {
    persistWindowSessions(cleaned, storage);
  }
}

export function removeWindowSession(
  label: string,
  storage?: Storage | null,
) {
  const normalizedLabel = normalizeLabel(label);
  if (!normalizedLabel) return;

  const sessions = loadWindowSessions(storage).filter(
    (session) => session.label !== normalizedLabel,
  );
  persistWindowSessions(sessions, storage);
}
