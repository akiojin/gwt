/**
 * カスタムツール設定管理
 *
 * ~/.claude-worktree/tools.json から設定を読み込み、
 * ビルトインツールと統合して管理します。
 */

import { homedir } from "node:os";
import path from "node:path";
import { readFile, writeFile, mkdir, rename } from "node:fs/promises";
import type {
  ToolsConfig,
  CustomAITool,
  AIToolConfig,
} from "../types/tools.js";
import { BUILTIN_TOOLS } from "./builtin-tools.js";
import {
  sanitizeEnvRecord,
  validateEnvRecord,
  mergeWithBootstrapEnv,
  type EnvironmentRecord,
} from "./shared-env.js";

export const WORKTREE_HOME =
  process.env.CLAUDE_WORKTREE_HOME &&
  process.env.CLAUDE_WORKTREE_HOME.trim().length > 0
    ? process.env.CLAUDE_WORKTREE_HOME
    : homedir();
export const CONFIG_DIR = path.join(WORKTREE_HOME, ".claude-worktree");
export const TOOLS_CONFIG_PATH = path.join(CONFIG_DIR, "tools.json");
const TEMP_CONFIG_PATH = `${TOOLS_CONFIG_PATH}.tmp`;

const DEFAULT_CONFIG: ToolsConfig = {
  version: "1.0.0",
  env: {},
  customTools: [],
};

let envBootstrapApplied = false;

/**
 * ツール設定を読み込む
 */
export async function loadToolsConfig(): Promise<ToolsConfig> {
  try {
    const content = await readFile(TOOLS_CONFIG_PATH, "utf-8");
    let config = normalizeToolsConfig(JSON.parse(content) as ToolsConfig);

    validateToolsConfig(config);
    config = await bootstrapSharedEnv(config);

    return {
      ...config,
      env: config.env ?? {},
    };
  } catch (error) {
    const nodeError = error as NodeJS.ErrnoException;
    if (nodeError?.code === "ENOENT") {
      return { ...DEFAULT_CONFIG };
    }

    if (error instanceof SyntaxError) {
      throw new Error(
        `Failed to parse tools.json: ${error.message}\n` +
          `Please check the JSON syntax in ${TOOLS_CONFIG_PATH}`,
      );
    }

    throw error;
  }
}

/**
 * ToolsConfig全体を検証
 */
function validateToolsConfig(config: ToolsConfig): void {
  if (!config.version || typeof config.version !== "string") {
    throw new Error("version field is required and must be a string");
  }

  if (!Array.isArray(config.customTools)) {
    throw new Error("customTools field must be an array");
  }

  if (config.env && typeof config.env !== "object") {
    throw new Error("env field must be an object map of key/value pairs");
  }

  if (config.env) {
    validateEnvRecord(config.env as EnvironmentRecord);
  }

  const seenIds = new Set<string>();
  for (const tool of config.customTools) {
    validateCustomAITool(tool);
    if (seenIds.has(tool.id)) {
      throw new Error(
        `Duplicate tool ID found: "${tool.id}"\n` +
          `Each tool must have a unique ID in ${TOOLS_CONFIG_PATH}`,
      );
    }
    seenIds.add(tool.id);

    const builtinIds = BUILTIN_TOOLS.map((t) => t.id);
    if (builtinIds.includes(tool.id)) {
      throw new Error(
        `Tool ID "${tool.id}" conflicts with builtin tool\n` +
          `Builtin tool IDs: ${builtinIds.join(", ")}`,
      );
    }
  }
}

/**
 * CustomAITool単体を検証
 */
function validateCustomAITool(tool: unknown): asserts tool is CustomAITool {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const t = tool as any;
  const requiredFields = ["id", "displayName", "type", "command", "modeArgs"];
  for (const field of requiredFields) {
    if (!t[field]) {
      throw new Error(
        `Required field "${field}" is missing in tool configuration`,
      );
    }
  }

  if (!/^[a-z0-9-]+$/.test(t.id)) {
    throw new Error(
      `Invalid tool ID format: "${t.id}"\n` +
        `Tool ID must contain only lowercase letters, numbers, and hyphens (pattern: ^[a-z0-9-]+$)`,
    );
  }

  const validTypes = ["path", "bunx", "command"];
  if (!validTypes.includes(t.type)) {
    throw new Error(
      `Invalid type: "${t.type}"\n` +
        `Type must be one of: ${validTypes.join(", ")}`,
    );
  }

  if (t.type === "path" && !path.isAbsolute(t.command)) {
    throw new Error(
      `For type="path", command must be an absolute path: "${t.command}"`,
    );
  }

  if (!t.modeArgs.normal && !t.modeArgs.continue && !t.modeArgs.resume) {
    throw new Error(
      `modeArgs must define at least one mode (normal, continue, or resume) for tool "${t.id}"`,
    );
  }
}

function normalizeToolsConfig(config: ToolsConfig): ToolsConfig {
  const version = config.version ?? "1.0.0";
  const customTools = (config.customTools ?? []).map((tool) => {
    const createdAt = tool.createdAt ?? new Date().toISOString();
    const updatedAt = tool.updatedAt ?? createdAt;
    return {
      ...tool,
      createdAt,
      updatedAt,
    };
  });
  const env = sanitizeEnvRecord(config.env);
  const baseConfig: ToolsConfig = {
    version,
    customTools,
    env,
  };

  if (config.updatedAt) {
    baseConfig.updatedAt = config.updatedAt;
  }

  return baseConfig;
}

export async function saveToolsConfig(config: ToolsConfig): Promise<void> {
  const updatedAt = config.updatedAt ?? new Date().toISOString();
  const normalized = normalizeToolsConfig({
    ...config,
    updatedAt,
  });
  validateToolsConfig(normalized);

  await mkdir(CONFIG_DIR, { recursive: true });
  const payload = JSON.stringify(normalized, null, 2);
  await writeFile(TEMP_CONFIG_PATH, payload, { mode: 0o600 });
  await rename(TEMP_CONFIG_PATH, TOOLS_CONFIG_PATH);
}

export async function getSharedEnvironment(): Promise<Record<string, string>> {
  const config = await loadToolsConfig();
  return { ...(config.env ?? {}) };
}

async function bootstrapSharedEnv(config: ToolsConfig): Promise<ToolsConfig> {
  if (envBootstrapApplied) {
    return config;
  }

  const currentEnv = config.env ?? {};
  const { merged, addedKeys } = mergeWithBootstrapEnv(currentEnv, process.env);

  if (addedKeys.length === 0) {
    envBootstrapApplied = true;
    return config;
  }

  const nextConfig: ToolsConfig = {
    ...config,
    env: merged,
  };

  try {
    await saveToolsConfig(nextConfig);
    envBootstrapApplied = true;
  } catch (error) {
    envBootstrapApplied = false;
    throw error;
  }

  return nextConfig;
}

export async function getToolById(
  id: string,
): Promise<CustomAITool | undefined> {
  const builtinTool = BUILTIN_TOOLS.find((t) => t.id === id);
  if (builtinTool) {
    return builtinTool;
  }

  const config = await loadToolsConfig();
  return config.customTools.find((t) => t.id === id);
}

export async function getAllTools(): Promise<AIToolConfig[]> {
  const config = await loadToolsConfig();

  const builtinConfigs: AIToolConfig[] = BUILTIN_TOOLS.map((tool) => ({
    id: tool.id,
    displayName: tool.displayName,
    ...(tool.icon ? { icon: tool.icon } : {}),
    isBuiltin: true,
  }));

  const customConfigs: AIToolConfig[] = config.customTools.map((tool) => ({
    id: tool.id,
    displayName: tool.displayName,
    ...(tool.icon ? { icon: tool.icon } : {}),
    isBuiltin: false,
    customConfig: tool,
  }));

  return [...builtinConfigs, ...customConfigs];
}
