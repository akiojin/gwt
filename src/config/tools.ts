/**
 * コーディングエージェント設定管理
 *
 * ~/.gwt/tools.jsonから設定を読み込み、
 * ビルトインエージェントと統合して管理します。
 */

import { homedir } from "node:os";
import path from "node:path";
import { readFile, writeFile, mkdir, rename } from "node:fs/promises";
import type {
  CodingAgentsConfig,
  CodingAgent,
  CodingAgentConfig,
} from "../types/tools.js";
import { BUILTIN_CODING_AGENTS } from "./builtin-coding-agents.js";
import { createLogger } from "../logging/logger.js";
import { resolveProfileEnv } from "./profiles.js";

const logger = createLogger({ category: "config" });

/**
 * コーディングエージェント設定ファイルのパス
 * 環境変数 GWT_HOME が設定されている場合はそれを使用、それ以外はホームディレクトリ
 */
export const WORKTREE_HOME =
  process.env.GWT_HOME && process.env.GWT_HOME.trim().length > 0
    ? process.env.GWT_HOME
    : homedir();

export const CONFIG_DIR = path.join(WORKTREE_HOME, ".gwt");
export const TOOLS_CONFIG_PATH = path.join(CONFIG_DIR, "tools.json");
const TEMP_CONFIG_PATH = `${TOOLS_CONFIG_PATH}.tmp`;

const DEFAULT_CONFIG: CodingAgentsConfig = {
  version: "1.0.0",
  env: {},
  customCodingAgents: [],
};

/**
 * コーディングエージェント設定を読み込む
 *
 * ~/.gwt/tools.jsonから設定を読み込みます。
 * ファイルが存在しない場合は空配列を返します。
 *
 * @returns CodingAgentsConfig
 * @throws JSON構文エラー時
 */
export async function loadCodingAgentsConfig(): Promise<CodingAgentsConfig> {
  try {
    const content = await readFile(TOOLS_CONFIG_PATH, "utf-8");
    const config = JSON.parse(content) as CodingAgentsConfig;

    // マイグレーション: customTools → customCodingAgents (後方互換性)
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const legacyConfig = config as any;
    if (!config.customCodingAgents && legacyConfig.customTools) {
      config.customCodingAgents = legacyConfig.customTools;
      logger.warn(
        { path: TOOLS_CONFIG_PATH },
        "Migrating deprecated 'customTools' to 'customCodingAgents'",
      );
    }

    // フォールバック: undefined/null → 空配列
    if (!config.customCodingAgents) {
      config.customCodingAgents = [];
    }

    // 検証
    validateCodingAgentsConfig(config);

    logger.debug(
      {
        path: TOOLS_CONFIG_PATH,
        agentCount: config.customCodingAgents.length,
      },
      "Coding agents config loaded",
    );

    return {
      ...config,
      env: config.env ?? {},
    };
  } catch (error) {
    // ファイルが存在しない場合は空配列を返す
    if (error instanceof Error && "code" in error && error.code === "ENOENT") {
      logger.debug(
        { path: TOOLS_CONFIG_PATH },
        "Coding agents config not found, using defaults",
      );
      return { ...DEFAULT_CONFIG };
    }

    // JSON構文エラーの場合
    if (error instanceof SyntaxError) {
      logger.error(
        { path: TOOLS_CONFIG_PATH, error: error.message },
        "Coding agents config parse error",
      );
      throw new Error(
        `Failed to parse tools.json: ${error.message}\n` +
          `Please check the JSON syntax in ${TOOLS_CONFIG_PATH}`,
      );
    }

    // その他のエラー
    throw error;
  }
}

/**
 * CodingAgentsConfig全体を検証
 *
 * @param config - 検証対象の設定
 * @throws 検証エラー時
 */
function validateCodingAgentsConfig(config: CodingAgentsConfig): void {
  // versionフィールドの検証
  if (!config.version || typeof config.version !== "string") {
    throw new Error("version field is required and must be a string");
  }

  // customCodingAgentsフィールドの検証
  if (!Array.isArray(config.customCodingAgents)) {
    throw new Error("customCodingAgents field must be an array");
  }

  if (config.env && typeof config.env !== "object") {
    throw new Error("env field must be an object map of key/value pairs");
  }

  if (config.env) {
    for (const [key, value] of Object.entries(config.env)) {
      if (!key || typeof key !== "string") {
        throw new Error("env keys must be non-empty strings");
      }
      if (typeof value !== "string") {
        throw new Error(`env value for key "${key}" must be a string`);
      }
    }
  }

  // 各エージェントの検証
  const seenIds = new Set<string>();
  for (const agent of config.customCodingAgents) {
    validateCodingAgent(agent);

    // ID重複チェック
    if (seenIds.has(agent.id)) {
      throw new Error(
        `Duplicate agent ID found: "${agent.id}"\n` +
          `Each agent must have a unique ID in ${TOOLS_CONFIG_PATH}`,
      );
    }
    seenIds.add(agent.id);

    // ビルトインエージェントとのID重複チェック
    const builtinIds = BUILTIN_CODING_AGENTS.map((t) => t.id);
    if (builtinIds.includes(agent.id)) {
      throw new Error(
        `Agent ID "${agent.id}" conflicts with builtin agent\n` +
          `Builtin agent IDs: ${builtinIds.join(", ")}`,
      );
    }
  }
}

export async function saveCodingAgentsConfig(
  config: CodingAgentsConfig,
): Promise<void> {
  const normalized: CodingAgentsConfig = {
    version: config.version,
    updatedAt: config.updatedAt ?? new Date().toISOString(),
    env: config.env ?? {},
    customCodingAgents: config.customCodingAgents,
  };

  validateCodingAgentsConfig(normalized);

  await mkdir(CONFIG_DIR, { recursive: true });
  const payload = JSON.stringify(normalized, null, 2);
  await writeFile(TEMP_CONFIG_PATH, payload, { mode: 0o600 });
  await rename(TEMP_CONFIG_PATH, TOOLS_CONFIG_PATH);
}

/**
 * 共有環境変数を取得
 *
 * コーディングエージェント起動時に適用される環境変数を返します。
 * マージ優先順位（後勝ち）:
 * 1. tools.json の env フィールド
 * 2. profiles.yaml のアクティブプロファイル
 *
 * @returns 環境変数のRecord
 */
export async function getSharedEnvironment(): Promise<Record<string, string>> {
  const [config, profileEnv] = await Promise.all([
    loadCodingAgentsConfig(),
    resolveProfileEnv(),
  ]);

  return {
    ...(config.env ?? {}),
    ...profileEnv, // プロファイルが後勝ち
  };
}

/**
 * CodingAgent単体を検証
 *
 * @param agent - 検証対象のエージェント
 * @throws 検証エラー時
 */
function validateCodingAgent(agent: unknown): asserts agent is CodingAgent {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const a = agent as any;

  // 必須フィールドの存在チェック
  const requiredFields = ["id", "displayName", "type", "command", "modeArgs"];
  for (const field of requiredFields) {
    if (!a[field]) {
      throw new Error(
        `Required field "${field}" is missing in agent configuration`,
      );
    }
  }

  // id形式の検証（小文字英数字とハイフンのみ）
  if (!/^[a-z0-9-]+$/.test(a.id)) {
    throw new Error(
      `Invalid agent ID format: "${a.id}"\n` +
        `Agent ID must contain only lowercase letters, numbers, and hyphens (pattern: ^[a-z0-9-]+$)`,
    );
  }

  // typeフィールドの値検証
  const validTypes = ["path", "bunx", "command"];
  if (!validTypes.includes(a.type)) {
    throw new Error(
      `Invalid type: "${a.type}"\n` +
        `Type must be one of: ${validTypes.join(", ")}`,
    );
  }

  // type='path'の場合、commandが絶対パスであることを確認
  if (a.type === "path" && !path.isAbsolute(a.command)) {
    throw new Error(
      `For type="path", command must be an absolute path: "${a.command}"`,
    );
  }

  // modeArgsの検証（少なくとも1つのモードが定義されている）
  if (!a.modeArgs.normal && !a.modeArgs.continue && !a.modeArgs.resume) {
    throw new Error(
      `modeArgs must define at least one mode (normal, continue, or resume) for agent "${a.id}"`,
    );
  }
}

/**
 * IDでコーディングエージェントを検索
 *
 * @param id - エージェントID
 * @returns エージェント設定（見つからない場合はundefined）
 */
export async function getCodingAgentById(
  id: string,
): Promise<CodingAgent | undefined> {
  // ビルトインエージェントから検索
  const builtinAgent = BUILTIN_CODING_AGENTS.find((a) => a.id === id);
  if (builtinAgent) {
    logger.debug({ id, found: true, isBuiltin: true }, "Coding agent lookup");
    return builtinAgent;
  }

  // カスタムエージェントから検索
  const config = await loadCodingAgentsConfig();
  const customAgent = config.customCodingAgents.find((a) => a.id === id);
  logger.debug(
    { id, found: !!customAgent, isBuiltin: false },
    "Coding agent lookup",
  );
  return customAgent;
}

/**
 * すべてのコーディングエージェント（ビルトイン+カスタム）を取得
 *
 * @returns CodingAgentConfigの配列
 */
export async function getAllCodingAgents(): Promise<CodingAgentConfig[]> {
  const config = await loadCodingAgentsConfig();

  // ビルトインエージェントをCodingAgentConfig形式に変換
  const builtinConfigs: CodingAgentConfig[] = BUILTIN_CODING_AGENTS.map(
    (agent) => ({
      id: agent.id,
      displayName: agent.displayName,
      ...(agent.icon ? { icon: agent.icon } : {}),
      isBuiltin: true,
    }),
  );

  // カスタムエージェントをCodingAgentConfig形式に変換
  const customConfigs: CodingAgentConfig[] = config.customCodingAgents.map(
    (agent) => ({
      id: agent.id,
      displayName: agent.displayName,
      ...(agent.icon ? { icon: agent.icon } : {}),
      isBuiltin: false,
      customConfig: agent,
    }),
  );

  // ビルトイン + カスタム の順で統合
  return [...builtinConfigs, ...customConfigs];
}
