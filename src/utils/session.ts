import path from "node:path";
import { homedir } from "node:os";
import { readdir, readFile, stat } from "node:fs/promises";

const UUID_REGEX =
  /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

/**
 * Validates that a string is a properly formatted UUID session ID.
 * @param id - The string to validate
 * @returns true if the string is a valid UUID format
 */
export function isValidUuidSessionId(id: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(id);
}

function pickSessionIdFromObject(obj: unknown): string | null {
  if (!obj || typeof obj !== "object") return null;
  const candidate = obj as Record<string, unknown>;
  const keys = ["sessionId", "session_id", "id", "conversation_id"];
  for (const key of keys) {
    const value = candidate[key];
    if (typeof value === "string" && value.trim().length > 0) {
      const trimmed = value.trim();
      // Only accept values that are valid UUIDs to avoid picking up arbitrary strings
      if (isValidUuidSessionId(trimmed)) {
        return trimmed;
      }
    }
  }
  return null;
}

function pickCwdFromObject(obj: unknown): string | null {
  if (!obj || typeof obj !== "object") return null;
  const candidate = obj as Record<string, unknown>;
  const keys = ["cwd", "workingDirectory", "workdir", "directory", "projectPath"];
  for (const key of keys) {
    const value = candidate[key];
    if (typeof value === "string" && value.trim().length > 0) {
      return value;
    }
  }
  // Check nested payload object (for Codex session format)
  const payload = candidate["payload"];
  if (payload && typeof payload === "object") {
    const nested = pickCwdFromObject(payload);
    if (nested) return nested;
  }
  return null;
}

function pickSessionIdFromText(content: string): string | null {
  // Try whole content as JSON
  try {
    const parsed = JSON.parse(content);
    const fromObject = pickSessionIdFromObject(parsed);
    if (fromObject) return fromObject;
  } catch {
    // ignore
  }

  // Try JSONL lines
  const lines = content.split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    try {
      const parsedLine = JSON.parse(trimmed);
      const fromLine = pickSessionIdFromObject(parsedLine);
      if (fromLine) return fromLine;
    } catch {
      // ignore
    }
    const match = trimmed.match(UUID_REGEX);
    if (match) return match[0];
  }

  // Fallback: find any UUID in the whole text
  const match = content.match(UUID_REGEX);
  return match ? match[0] : null;
}

async function findLatestFile(
  dir: string,
  filter: (name: string) => boolean,
): Promise<string | null> {
  try {
    const entries = await readdir(dir, { withFileTypes: true });
    const files = entries.filter((e) => e.isFile()).map((e) => e.name);
    const filtered = files.filter(filter);
    if (!filtered.length) return null;

    const withStats = await Promise.all(
      filtered.map(async (name) => {
        const fullPath = path.join(dir, name);
        try {
          const info = await stat(fullPath);
          return { fullPath, mtime: info.mtimeMs };
        } catch {
          return null;
        }
      }),
    );

    const valid = withStats.filter(
      (entry): entry is { fullPath: string; mtime: number } => Boolean(entry),
    );
    if (!valid.length) return null;

    valid.sort((a, b) => b.mtime - a.mtime);
    return valid[0]?.fullPath ?? null;
  } catch {
    return null;
  }
}

async function findNewestSessionIdFromDir(
  dir: string,
  recursive: boolean,
  options: { since?: number; until?: number; preferClosestTo?: number; windowMs?: number } = {},
): Promise<{ id: string; mtime: number } | null> {
  try {
    const files: { fullPath: string; mtime: number }[] = [];

    const processDir = async (currentDir: string) => {
      const currentEntries = await readdir(currentDir, { withFileTypes: true });
      for (const entry of currentEntries) {
        const fullPath = path.join(currentDir, entry.name);
        if (entry.isDirectory()) {
          if (recursive) {
            await processDir(fullPath);
          }
          continue;
        }
        if (!entry.isFile()) continue;
        if (!entry.name.endsWith(".json") && !entry.name.endsWith(".jsonl"))
          continue;
        try {
          const info = await stat(fullPath);
          files.push({ fullPath, mtime: info.mtimeMs });
        } catch {
          // ignore unreadable
        }
      }
    };

    await processDir(dir);

    // Apply since/until filters clearly
    const filtered = files.filter((f) => {
      if (options.since !== undefined && f.mtime < options.since) return false;
      if (options.until !== undefined && f.mtime > options.until) return false;
      return true;
    });

    if (!filtered.length) return null;

    // Sort by mtime descending (newest first)
    let pool = filtered.sort((a, b) => b.mtime - a.mtime);

    // Apply preferClosestTo window if specified
    const ref = options.preferClosestTo;
    if (typeof ref === "number") {
      const window = options.windowMs ?? 30 * 60 * 1000;
      const withinWindow = pool.filter(
        (f) => Math.abs(f.mtime - ref) <= window,
      );
      if (withinWindow.length) {
        pool = withinWindow.sort((a, b) => b.mtime - a.mtime);
      }
    }

    for (const file of pool) {
      const id = await readSessionIdFromFile(file.fullPath);
      if (id) return { id, mtime: file.mtime };
    }
  } catch {
    // ignore
  }
  return null;
}

async function readSessionIdFromFile(filePath: string): Promise<string | null> {
  try {
    // Priority 1: Use filename UUID (most reliable for Claude session files)
    // Claude session files are named with their session ID: {uuid}.jsonl
    const basename = path.basename(filePath);
    const filenameWithoutExt = basename.replace(/\.(json|jsonl)$/i, "");
    if (isValidUuidSessionId(filenameWithoutExt)) {
      return filenameWithoutExt;
    }

    // Priority 2: Extract from file content (for other formats)
    const content = await readFile(filePath, "utf-8");
    const fromContent = pickSessionIdFromText(content);
    if (fromContent) return fromContent;

    // Priority 3: Fallback to any UUID in filename
    const filenameMatch = basename.match(UUID_REGEX);
    return filenameMatch ? filenameMatch[0] : null;
  } catch {
    return null;
  }
}

async function readSessionInfoFromFile(
  filePath: string,
): Promise<{ id: string | null; cwd: string | null }> {
  try {
    const content = await readFile(filePath, "utf-8");
    try {
      const parsed = JSON.parse(content);
      const id = pickSessionIdFromObject(parsed);
      const cwd = pickCwdFromObject(parsed);
      if (id || cwd) return { id, cwd };
    } catch {
      // ignore
    }

    const lines = content.split(/\r?\n/);
    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      try {
        const parsedLine = JSON.parse(trimmed);
        const id = pickSessionIdFromObject(parsedLine);
        const cwd = pickCwdFromObject(parsedLine);
        if (id || cwd) return { id, cwd };
      } catch {
        // ignore
      }
    }

    // Fallback: filename UUID
    const filenameMatch = path.basename(filePath).match(UUID_REGEX);
    if (filenameMatch) return { id: filenameMatch[0], cwd: null };
  } catch {
    // ignore unreadable
  }
  return { id: null, cwd: null };
}

export interface CodexSessionInfo {
  id: string;
  mtime: number;
}

export interface GeminiSessionInfo {
  id: string;
  mtime: number;
}

async function collectFilesRecursive(
  dir: string,
  filter: (name: string) => boolean,
): Promise<{ fullPath: string; mtime: number }[]> {
  const results: { fullPath: string; mtime: number }[] = [];
  try {
    const entries = await readdir(dir, { withFileTypes: true });
    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        const nested = await collectFilesRecursive(fullPath, filter);
        results.push(...nested);
      } else if (entry.isFile() && filter(entry.name)) {
        try {
          const info = await stat(fullPath);
          results.push({ fullPath, mtime: info.mtimeMs });
        } catch {
          // ignore unreadable file
        }
      }
    }
  } catch {
    // ignore unreadable directory
  }
  return results;
}

export async function findLatestCodexSession(
  options: {
    since?: number;
    until?: number;
    preferClosestTo?: number;
    windowMs?: number;
    cwd?: string | null;
  } = {},
): Promise<CodexSessionInfo | null> {
  // Codex CLI respects CODEX_HOME. Default is ~/.codex.
  const codexHome = process.env.CODEX_HOME ?? path.join(homedir(), ".codex");
  const baseDir = path.join(codexHome, "sessions");
  const candidates = await collectFilesRecursive(
    baseDir,
    (name) => name.endsWith(".json") || name.endsWith(".jsonl"),
  );
  if (!candidates.length) return null;

  const sinceFiltered = options.since
    ? candidates.filter((c) => c.mtime >= options.since!)
    : candidates;
  const bounded =
    options.until !== undefined
      ? sinceFiltered.filter((c) => c.mtime <= options.until!)
      : sinceFiltered;
  const hasWindow = options.since !== undefined || options.until !== undefined;
  const pool = bounded.length ? bounded : hasWindow ? [] : sinceFiltered;

  if (!pool.length) return null;

  const ref = options.preferClosestTo;
  const window = options.windowMs ?? 30 * 60 * 1000; // 30 minutes default
  const ordered = [...pool].sort((a, b) => {
    if (typeof ref === "number") {
      const da = Math.abs(a.mtime - ref);
      const db = Math.abs(b.mtime - ref);
      if (da === db) return b.mtime - a.mtime;
      if (da <= window || db <= window) return da - db;
    }
    return b.mtime - a.mtime;
  });

  for (const file of ordered) {
    // Priority 1: Extract session ID from filename (most reliable for Codex)
    // Codex filenames follow pattern: rollout-YYYY-MM-DDTHH-MM-SS-{uuid}.jsonl
    const filenameMatch = path.basename(file.fullPath).match(UUID_REGEX);
    if (filenameMatch) {
      const sessionId = filenameMatch[0];
      // If cwd filtering is needed, read file content to check cwd
      if (options.cwd) {
        const info = await readSessionInfoFromFile(file.fullPath);
        if (
          info.cwd &&
          // Match if: exact match, session cwd starts with options.cwd,
          // or options.cwd starts with session cwd (for worktree subdirectories)
          (info.cwd === options.cwd ||
            info.cwd.startsWith(options.cwd) ||
            options.cwd.startsWith(info.cwd))
        ) {
          return { id: sessionId, mtime: file.mtime };
        }
        continue; // cwd doesn't match, try next file
      }
      return { id: sessionId, mtime: file.mtime };
    }

    // Priority 2: Fallback to reading file content if filename lacks UUID
    const info = await readSessionInfoFromFile(file.fullPath);
    if (!info.id) continue;
    if (options.cwd) {
      if (
        info.cwd &&
        // Match if: exact match, session cwd starts with options.cwd,
        // or options.cwd starts with session cwd (for worktree subdirectories)
        (info.cwd === options.cwd ||
          info.cwd.startsWith(options.cwd) ||
          options.cwd.startsWith(info.cwd))
      ) {
        return { id: info.id, mtime: file.mtime };
      }
      continue;
    }
    return { id: info.id, mtime: file.mtime };
  }

  return null;
}

export async function findLatestCodexSessionId(
  options: {
    since?: number;
    until?: number;
    preferClosestTo?: number;
    windowMs?: number;
    cwd?: string | null;
  } = {},
): Promise<string | null> {
  const found = await findLatestCodexSession(options);
  return found?.id ?? null;
}

export async function waitForCodexSessionId(options: {
  startedAt: number;
  timeoutMs?: number;
  pollIntervalMs?: number;
  cwd?: string | null;
}): Promise<string | null> {
  const timeoutMs = options.timeoutMs ?? 120_000;
  const pollIntervalMs = options.pollIntervalMs ?? 2_000;
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    const found = await findLatestCodexSession({
      since: options.startedAt,
      preferClosestTo: options.startedAt,
      windowMs: 10 * 60 * 1000,
      cwd: options.cwd ?? null,
    });
    if (found?.id) return found.id;
    await new Promise((resolve) => setTimeout(resolve, pollIntervalMs));
  }
  return null;
}

export function encodeClaudeProjectPath(cwd: string): string {
  // Normalize to forward slashes, drop drive colon, replace / and _ with -
  const normalized = cwd.replace(/\\/g, "/").replace(/:/g, "");
  return normalized.replace(/_/g, "-").replace(/\//g, "-");
}

function generateClaudeProjectPathCandidates(cwd: string): string[] {
  const base = encodeClaudeProjectPath(cwd);
  const dotToDash = cwd
    .replace(/\\/g, "/")
    .replace(/:/g, "")
    .replace(/\./g, "-")
    .replace(/_/g, "-")
    .replace(/\//g, "-");
  const collapsed = dotToDash.replace(/-+/g, "-");
  const candidates = [base, dotToDash, collapsed];
  return Array.from(new Set(candidates));
}

export async function findLatestClaudeSessionId(
  cwd: string,
  options: { since?: number; until?: number; preferClosestTo?: number; windowMs?: number } = {},
): Promise<string | null> {
  const found = await findLatestClaudeSession(cwd, options);
  return found?.id ?? null;
}

export interface ClaudeSessionInfo {
  id: string;
  mtime: number;
}

export async function findLatestClaudeSession(
  cwd: string,
  options: { since?: number; until?: number; preferClosestTo?: number; windowMs?: number } = {},
): Promise<ClaudeSessionInfo | null> {
  const rootCandidates: string[] = [];
  if (process.env.CLAUDE_CONFIG_DIR) {
    rootCandidates.push(process.env.CLAUDE_CONFIG_DIR);
  }
  rootCandidates.push(
    path.join(homedir(), ".claude"),
    path.join(homedir(), ".config", "claude"),
  );

  const encodedPaths = generateClaudeProjectPathCandidates(cwd);

  for (const claudeRoot of rootCandidates) {
    for (const encoded of encodedPaths) {
      const projectDir = path.join(claudeRoot, "projects", encoded);
      const sessionsDir = path.join(projectDir, "sessions");

      // 1) Look under sessions/ (official location) - prefer newest file with valid ID
      const session = await findNewestSessionIdFromDir(
        sessionsDir,
        false,
        options,
      );
      if (session) return session;

      // 2) Look directly under project dir and subdirs (some versions emit files at root)
      const rootSession = await findNewestSessionIdFromDir(
        projectDir,
        true,
        options,
      );
      if (rootSession) return rootSession;
    }
  }

  // Fallback: parse ~/.claude/history.jsonl (Claude Code global history)
  try {
    const historyPath = path.join(homedir(), ".claude", "history.jsonl");
    const content = await readFile(historyPath, "utf-8");
    const lines = content.split(/\r?\n/).filter(Boolean);
    for (let i = lines.length - 1; i >= 0; i -= 1) {
      try {
        const line = lines[i] ?? "";
        const parsed = JSON.parse(line) as Record<string, unknown>;
        const project = typeof parsed.project === "string" ? parsed.project : null;
        const sessionId = typeof parsed.sessionId === "string" ? parsed.sessionId : null;
        if (project && sessionId && (project === cwd || cwd.startsWith(project))) {
          return { id: sessionId, mtime: Date.now() };
        }
      } catch {
        // ignore malformed lines
      }
    }
  } catch {
    // ignore if history not present
  }

  return null;
}

export async function waitForClaudeSessionId(
  cwd: string,
  options: {
    timeoutMs?: number;
    pollIntervalMs?: number;
    since?: number;
    until?: number;
    preferClosestTo?: number;
    windowMs?: number;
  } = {},
): Promise<string | null> {
  const timeoutMs = options.timeoutMs ?? 120_000;
  const pollIntervalMs = options.pollIntervalMs ?? 2_000;
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    const opt: { since?: number; until?: number; preferClosestTo?: number; windowMs?: number } = {};
    if (options.since !== undefined) opt.since = options.since;
    if (options.until !== undefined) opt.until = options.until;
    if (options.preferClosestTo !== undefined) opt.preferClosestTo = options.preferClosestTo;
    if (options.windowMs !== undefined) opt.windowMs = options.windowMs;

    const found = await findLatestClaudeSession(cwd, opt);
    if (found?.id) return found.id;
    await new Promise((resolve) => setTimeout(resolve, pollIntervalMs));
  }
  return null;
}

async function findLatestNestedSessionFile(
  baseDir: string,
  subPath: string[],
  predicate: (name: string) => boolean,
): Promise<string | null> {
  try {
    const entries = await readdir(baseDir);
    if (!entries.length) return null;

    const candidates: { fullPath: string; mtime: number }[] = [];

    for (const entry of entries) {
      const dirPath = path.join(baseDir, entry, ...subPath);
      const latest = await findLatestFile(dirPath, predicate);
      if (latest) {
        try {
          const info = await stat(latest);
          candidates.push({ fullPath: latest, mtime: info.mtimeMs });
        } catch {
          // ignore
        }
      }
    }

    if (!candidates.length) return null;
    candidates.sort((a, b) => b.mtime - a.mtime);
    return candidates[0]?.fullPath ?? null;
  } catch {
    return null;
  }
}

export async function findLatestGeminiSession(
  _cwd: string,
  options: { since?: number; until?: number; preferClosestTo?: number; windowMs?: number; cwd?: string | null } = {},
): Promise<GeminiSessionInfo | null> {
  // Gemini stores sessions/logs under ~/.gemini/tmp/<project_hash>/(chats|logs).json
  const baseDir = path.join(homedir(), ".gemini", "tmp");
  const files = await collectFilesRecursive(
    baseDir,
    (name) => name.endsWith(".json") || name.endsWith(".jsonl"),
  );
  if (!files.length) return null;

  let pool = files;
  if (options.since !== undefined) {
    pool = pool.filter((f) => f.mtime >= options.since!);
  }
  if (options.until !== undefined) {
    pool = pool.filter((f) => f.mtime <= options.until!);
  }
  const hasWindow = options.since !== undefined || options.until !== undefined;
  if (!pool.length) {
    if (!hasWindow) {
      pool = files;
    } else {
      return null;
    }
  }

  const ref = options.preferClosestTo;
  const window = options.windowMs ?? 30 * 60 * 1000;
  pool = pool
    .slice()
    .sort((a, b) => {
      if (typeof ref === "number") {
        const da = Math.abs(a.mtime - ref);
        const db = Math.abs(b.mtime - ref);
        if (da === db) return b.mtime - a.mtime;
        if (da <= window || db <= window) return da - db;
      }
      return b.mtime - a.mtime;
    });

  for (const file of pool) {
    const info = await readSessionInfoFromFile(file.fullPath);
    if (!info.id) continue;
    if (options.cwd) {
      if (
        info.cwd &&
        (info.cwd === options.cwd || info.cwd.startsWith(options.cwd))
      ) {
        return { id: info.id, mtime: file.mtime };
      }
      continue;
    }
    return { id: info.id, mtime: file.mtime };
  }

  return null;
}

export async function findLatestGeminiSessionId(
  cwd: string,
  options: { since?: number; until?: number; preferClosestTo?: number; windowMs?: number; cwd?: string | null } = {},
): Promise<string | null> {
  const normalized: { since?: number; until?: number; preferClosestTo?: number; windowMs?: number } = {};
  if (options.since !== undefined) normalized.since = options.since as number;
  if (options.until !== undefined) normalized.until = options.until as number;
  if (options.preferClosestTo !== undefined)
    normalized.preferClosestTo = options.preferClosestTo as number;
  if (options.windowMs !== undefined) normalized.windowMs = options.windowMs as number;

  const found = await findLatestGeminiSession(cwd, { ...normalized, cwd: options.cwd ?? cwd });
  return found?.id ?? null;
}

export async function findLatestQwenSessionId(
  _cwd: string,
): Promise<string | null> {
  // Qwen stores checkpoints/saves under ~/.qwen/tmp/<project_hash>/
  const baseDir = path.join(homedir(), ".qwen", "tmp");
  const latest =
    (await findLatestNestedSessionFile(
      baseDir,
      [],
      (name) => name.endsWith(".json") || name.endsWith(".jsonl"),
    )) ??
    (await findLatestNestedSessionFile(
      baseDir,
      ["checkpoints"],
      (name) => name.endsWith(".json") || name.endsWith(".ckpt"),
    ));

  if (!latest) return null;
  const fromContent = await readSessionIdFromFile(latest);
  if (fromContent) return fromContent;
  // Fallback: use filename (without extension) as tag
  return path.basename(latest).replace(/\.[^.]+$/, "");
}

/**
 * Returns the list of possible Claude root directories.
 */
function getClaudeRootCandidates(): string[] {
  const roots: string[] = [];
  if (process.env.CLAUDE_CONFIG_DIR) {
    roots.push(process.env.CLAUDE_CONFIG_DIR);
  }
  roots.push(
    path.join(homedir(), ".claude"),
    path.join(homedir(), ".config", "claude"),
  );
  return roots;
}

/**
 * Checks if a Claude session file exists for the given session ID and worktree path.
 * @param sessionId - The session ID to check
 * @param worktreePath - The worktree path (used to determine project encoding)
 * @returns true if a session file exists for this ID
 */
export async function claudeSessionFileExists(
  sessionId: string,
  worktreePath: string,
): Promise<boolean> {
  if (!isValidUuidSessionId(sessionId)) {
    return false;
  }

  const encodedPaths = generateClaudeProjectPathCandidates(worktreePath);
  const roots = getClaudeRootCandidates();

  for (const root of roots) {
    for (const enc of encodedPaths) {
      const candidate = path.join(root, "projects", enc, `${sessionId}.jsonl`);
      try {
        await stat(candidate);
        return true;
      } catch {
        // continue to next candidate
      }
    }
  }
  return false;
}
