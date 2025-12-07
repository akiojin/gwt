import path from "node:path";
import { homedir } from "node:os";
import { readdir, readFile, stat } from "node:fs/promises";

const UUID_REGEX =
  /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

function pickSessionIdFromObject(obj: unknown): string | null {
  if (!obj || typeof obj !== "object") return null;
  const candidate = obj as Record<string, unknown>;
  const keys = ["sessionId", "session_id", "id", "conversation_id"];
  for (const key of keys) {
    const value = candidate[key];
    if (typeof value === "string" && value.trim().length > 0) {
      return value;
    }
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

async function findLatestFileRecursive(
  dir: string,
  filter: (name: string) => boolean,
) : Promise<string | null> {
  try {
    const entries = await readdir(dir, { withFileTypes: true });
    const candidates: { fullPath: string; mtime: number }[] = [];

    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        const nested = await findLatestFileRecursive(fullPath, filter);
        if (nested) {
          const info = await stat(nested);
          candidates.push({ fullPath: nested, mtime: info.mtimeMs });
        }
      } else if (entry.isFile() && filter(entry.name)) {
        try {
          const info = await stat(fullPath);
          candidates.push({ fullPath, mtime: info.mtimeMs });
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

async function findNewestSessionIdFromDir(
  dir: string,
  recursive: boolean,
  options: { since?: number; until?: number; preferClosestTo?: number; windowMs?: number } = {},
): Promise<{ id: string; mtime: number } | null> {
  try {
    const entries = await readdir(dir, { withFileTypes: true });
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
    if (options.since !== undefined) {
      files.splice(0, files.length, ...files.filter((f) => f.mtime >= options.since!));
    }
    if (options.until !== undefined) {
      files.splice(0, files.length, ...files.filter((f) => f.mtime <= options.until!));
    }
    const sorted = files.sort((a, b) => b.mtime - a.mtime);

    const ref = options.preferClosestTo;
    if (typeof ref === "number") {
      const window = options.windowMs ?? 30 * 60 * 1000;
      sorted.sort((a, b) => {
        const da = Math.abs(a.mtime - ref);
        const db = Math.abs(b.mtime - ref);
        if (da === db) return b.mtime - a.mtime;
        if (da <= window || db <= window) return da - db;
        return b.mtime - a.mtime;
      });
    }

    for (const file of sorted) {
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
    const content = await readFile(filePath, "utf-8");
    const fromContent = pickSessionIdFromText(content);
    if (fromContent) return fromContent;
    // Fallback: try to extract UUID from filename
    const filenameMatch = path.basename(filePath).match(UUID_REGEX);
    return filenameMatch ? filenameMatch[0] : null;
  } catch {
    return null;
  }
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
  const pool = bounded.length ? bounded : sinceFiltered.length ? sinceFiltered : candidates;

  let chosen: { fullPath: string; mtime: number } | null = null;

  if (typeof options.preferClosestTo === "number") {
    const ref = options.preferClosestTo;
    const window = options.windowMs ?? 30 * 60 * 1000; // 30 minutes default
    const withinWindow = pool.filter(
      (c) => Math.abs(c.mtime - ref) <= window,
    );
    const scored = (withinWindow.length ? withinWindow : pool).sort((a, b) => {
      const diffA = Math.abs(a.mtime - ref);
      const diffB = Math.abs(b.mtime - ref);
      if (diffA === diffB) {
        return b.mtime - a.mtime;
      }
      return diffA - diffB;
    });
    chosen = scored[0] ?? null;
  } else {
    const sorted = [...pool].sort((a, b) => b.mtime - a.mtime);
    chosen = sorted[0] ?? null;
  }

  if (!chosen) return null;

  const id = await readSessionIdFromFile(chosen.fullPath);
  if (!id) return null;
  return { id, mtime: chosen.mtime };
}

export async function findLatestCodexSessionId(
  options: { since?: number; preferClosestTo?: number; windowMs?: number } = {},
): Promise<string | null> {
  const found = await findLatestCodexSession(options);
  return found?.id ?? null;
}

export async function waitForCodexSessionId(options: {
  startedAt: number;
  timeoutMs?: number;
  pollIntervalMs?: number;
}): Promise<string | null> {
  const timeoutMs = options.timeoutMs ?? 120_000;
  const pollIntervalMs = options.pollIntervalMs ?? 2_000;
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    const found = await findLatestCodexSession({
      since: options.startedAt - 30_000,
      preferClosestTo: options.startedAt,
      windowMs: 30 * 60 * 1000,
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
): Promise<string | null> {
  const found = await findLatestClaudeSession(cwd);
  return found?.id ?? null;
}

export interface ClaudeSessionInfo {
  id: string;
  mtime: number;
}

export async function findLatestClaudeSession(
  cwd: string,
  options: { since?: number; preferClosestTo?: number; windowMs?: number } = {},
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
  options: { timeoutMs?: number; pollIntervalMs?: number } = {},
): Promise<string | null> {
  const timeoutMs = options.timeoutMs ?? 120_000;
  const pollIntervalMs = options.pollIntervalMs ?? 2_000;
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    const found = await findLatestClaudeSession(cwd);
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
  options: { since?: number; until?: number; preferClosestTo?: number; windowMs?: number } = {},
): Promise<GeminiSessionInfo | null> {
  // Gemini stores sessions under ~/.gemini/tmp/<project_hash>/chats/*.json
  const baseDir = path.join(homedir(), ".gemini", "tmp");
  const latest = await findLatestNestedSessionFile(baseDir, ["chats"], (name) =>
    name.endsWith(".json"),
  );
  if (!latest) return null;
  try {
    const info = await stat(latest);
    const id = await readSessionIdFromFile(latest);
    if (!id) return null;

    if (options.since !== undefined && info.mtimeMs < options.since) {
      return null;
    }
    if (options.until !== undefined && info.mtimeMs > options.until) {
      return null;
    }

    return { id, mtime: info.mtimeMs };
  } catch {
    return null;
  }
}

export async function findLatestGeminiSessionId(
  cwd: string,
  options: { since?: number; until?: number; preferClosestTo?: number; windowMs?: number } = {},
): Promise<string | null> {
  const found = await findLatestGeminiSession(cwd, options);
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
