import { loadToolsConfig, saveToolsConfig } from "../../../config/tools.js";
import { recordEnvHistory } from "../../../config/env-history.js";
import type { EnvironmentHistoryEntry } from "../../../types/api.js";

const IMPORTABLE_KEYS = [
  "OPENAI_API_KEY",
  "ANTHROPIC_API_KEY",
  "GITHUB_TOKEN",
  "GH_TOKEN",
  "PERSONAL_ACCESS_TOKEN",
  "HTTP_PROXY",
  "HTTPS_PROXY",
];

const importedKeySet = new Set<string>();

export async function importOsEnvIntoSharedConfig(): Promise<string[]> {
  const config = await loadToolsConfig();
  const sharedEnv = { ...(config.env ?? {}) };
  const importedKeys: string[] = [];

  for (const key of IMPORTABLE_KEYS) {
    const value = process.env[key];
    if (!value) continue;
    if (sharedEnv[key]) continue;
    sharedEnv[key] = value;
    importedKeys.push(key);
    importedKeySet.add(key);
  }

  if (!importedKeys.length) {
    return [];
  }

  await saveToolsConfig({
    ...config,
    env: sharedEnv,
  });

  const timestamp = new Date().toISOString();
  const historyEntries: EnvironmentHistoryEntry[] = importedKeys.map((key) => ({
    key,
    action: "import",
    source: "os",
    timestamp,
  }));
  await recordEnvHistory(historyEntries);

  return importedKeys;
}

export function getImportedEnvKeys(): string[] {
  return Array.from(importedKeySet);
}
