import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import type { EnvironmentHistoryEntry } from "../types/api.js";
import { CONFIG_DIR } from "./tools.js";

const HISTORY_PATH = path.join(CONFIG_DIR, "env-history.json");
const HISTORY_LIMIT = 200;

interface HistoryFile {
  entries: EnvironmentHistoryEntry[];
}

export async function loadEnvHistory(): Promise<EnvironmentHistoryEntry[]> {
  try {
    const raw = await readFile(HISTORY_PATH, "utf8");
    const parsed = JSON.parse(raw) as HistoryFile;
    return parsed.entries ?? [];
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") {
      return [];
    }
    throw error;
  }
}

export async function recordEnvHistory(
  entries: EnvironmentHistoryEntry[],
): Promise<void> {
  if (!entries.length) {
    return;
  }

  const existing = await loadEnvHistory();
  const updated = [...existing, ...entries];
  const trimmed =
    updated.length > HISTORY_LIMIT
      ? updated.slice(updated.length - HISTORY_LIMIT)
      : updated;

  await mkdir(CONFIG_DIR, { recursive: true });
  const payload: HistoryFile = { entries: trimmed };
  await writeFile(HISTORY_PATH, JSON.stringify(payload, null, 2), {
    mode: 0o600,
  });
}
