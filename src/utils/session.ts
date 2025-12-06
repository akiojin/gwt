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
    const files = (await readdir(dir)).filter(filter);
    if (!files.length) return null;

    const withStats = await Promise.all(
      files.map(async (name) => {
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

async function readSessionIdFromFile(filePath: string): Promise<string | null> {
  try {
    const content = await readFile(filePath, "utf-8");
    return pickSessionIdFromText(content);
  } catch {
    return null;
  }
}

export async function findLatestCodexSessionId(): Promise<string | null> {
  const baseDir = path.join(homedir(), ".codex", "sessions");
  const latest = await findLatestFile(
    baseDir,
    (name) => name.endsWith(".json") || name.endsWith(".jsonl"),
  );
  if (!latest) return null;
  return readSessionIdFromFile(latest);
}

export function encodeClaudeProjectPath(cwd: string): string {
  // Normalize to forward slashes, drop drive colon, replace / and _ with -
  const normalized = cwd.replace(/\\/g, "/").replace(/:/g, "");
  return normalized.replace(/_/g, "-").replace(/\//g, "-");
}

export async function findLatestClaudeSessionId(
  cwd: string,
): Promise<string | null> {
  const encoded = encodeClaudeProjectPath(cwd);
  const baseDir = path.join(
    homedir(),
    ".claude",
    "projects",
    encoded,
    "sessions",
  );
  const latest = await findLatestFile(
    baseDir,
    (name) => name.endsWith(".jsonl") || name.endsWith(".json"),
  );
  if (!latest) return null;
  return readSessionIdFromFile(latest);
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

export async function findLatestGeminiSessionId(
  _cwd: string,
): Promise<string | null> {
  // Gemini stores sessions under ~/.gemini/tmp/<project_hash>/chats/*.json
  const baseDir = path.join(homedir(), ".gemini", "tmp");
  const latest = await findLatestNestedSessionFile(baseDir, ["chats"], (name) =>
    name.endsWith(".json"),
  );
  if (!latest) return null;
  return readSessionIdFromFile(latest);
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
