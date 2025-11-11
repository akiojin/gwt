/**
 * カスタムツール設定管理
 *
 * ~/.claude-worktree/tools.jsonから設定を読み込み、
 * ビルトインツールと統合して管理します。
 */

import { homedir } from "node:os";
import path from "node:path";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import type {
  ToolsConfig,
  CustomAITool,
  AIToolConfig,
} from "../types/tools.js";
import { BUILTIN_TOOLS } from "./builtin-tools.js";

/**
 * ツール設定ファイルのパス
 */
const TOOLS_CONFIG_PATH = path.join(
  homedir(),
  ".claude-worktree",
  "tools.json",
);

/**
 * ツール設定を読み込む
 *
 * ~/.claude-worktree/tools.jsonから設定を読み込みます。
 * ファイルが存在しない場合は空配列を返します。
 *
 * @returns ToolsConfig
 * @throws JSON構文エラー時
 */
export async function loadToolsConfig(): Promise<ToolsConfig> {
  try {
    const content = await readFile(TOOLS_CONFIG_PATH, "utf-8");
    const parsed = JSON.parse(content) as ToolsConfig;
    const config = normalizeToolsConfig(parsed);

    // 検証
    validateToolsConfig(config);

    return config;
  } catch (error) {
    // ファイルが存在しない場合は空配列を返す
    if (error instanceof Error && "code" in error && error.code === "ENOENT") {
      return {
        version: "1.0.0",
        customTools: [],
      };
    }

    // JSON構文エラーの場合
    if (error instanceof SyntaxError) {
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
 * ToolsConfig全体を検証
 *
 * @param config - 検証対象の設定
 * @throws 検証エラー時
 */
function validateToolsConfig(config: ToolsConfig): void {
  // versionフィールドの検証
  if (!config.version || typeof config.version !== "string") {
    throw new Error("version field is required and must be a string");
  }

  // customToolsフィールドの検証
  if (!Array.isArray(config.customTools)) {
    throw new Error("customTools field must be an array");
  }

  // 各ツールの検証
  const seenIds = new Set<string>();
  for (const tool of config.customTools) {
    validateCustomAITool(tool);

    // ID重複チェック
    if (seenIds.has(tool.id)) {
      throw new Error(
        `Duplicate tool ID found: "${tool.id}"\n` +
          `Each tool must have a unique ID in ${TOOLS_CONFIG_PATH}`,
      );
    }
    seenIds.add(tool.id);

    // ビルトインツールとのID重複チェック
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
 *
 * @param tool - 検証対象のツール
 * @throws 検証エラー時
 */
function validateCustomAITool(tool: unknown): asserts tool is CustomAITool {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const t = tool as any;

  // 必須フィールドの存在チェック
  const requiredFields = ["id", "displayName", "type", "command", "modeArgs"];
  for (const field of requiredFields) {
    if (!t[field]) {
      throw new Error(
        `Required field "${field}" is missing in tool configuration`,
      );
    }
  }

  // id形式の検証（小文字英数字とハイフンのみ）
  if (!/^[a-z0-9-]+$/.test(t.id)) {
    throw new Error(
      `Invalid tool ID format: "${t.id}"\n` +
        `Tool ID must contain only lowercase letters, numbers, and hyphens (pattern: ^[a-z0-9-]+$)`,
    );
  }

  // typeフィールドの値検証
  const validTypes = ["path", "bunx", "command"];
  if (!validTypes.includes(t.type)) {
    throw new Error(
      `Invalid type: "${t.type}"\n` +
        `Type must be one of: ${validTypes.join(", ")}`,
    );
  }

  // type='path'の場合、commandが絶対パスであることを確認
  if (t.type === "path" && !path.isAbsolute(t.command)) {
    throw new Error(
      `For type="path", command must be an absolute path: "${t.command}"`,
    );
  }

  // modeArgsの検証（少なくとも1つのモードが定義されている）
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

  return {
    version,
    customTools,
  };
}

export async function saveToolsConfig(config: ToolsConfig): Promise<void> {
  const normalized = normalizeToolsConfig(config);
  validateToolsConfig(normalized);

  const dir = path.dirname(TOOLS_CONFIG_PATH);
  await mkdir(dir, { recursive: true });
  await writeFile(
    TOOLS_CONFIG_PATH,
    JSON.stringify(normalized, null, 2),
    "utf-8",
  );
}

/**
 * IDでツールを検索
 *
 * @param id - ツールID
 * @returns ツール設定（見つからない場合はundefined）
 */
export async function getToolById(
  id: string,
): Promise<CustomAITool | undefined> {
  // ビルトインツールから検索
  const builtinTool = BUILTIN_TOOLS.find((t) => t.id === id);
  if (builtinTool) {
    return builtinTool;
  }

  // カスタムツールから検索
  const config = await loadToolsConfig();
  return config.customTools.find((t) => t.id === id);
}

/**
 * すべてのツール（ビルトイン+カスタム）を取得
 *
 * @returns AIToolConfigの配列
 */
export async function getAllTools(): Promise<AIToolConfig[]> {
  const config = await loadToolsConfig();

  // ビルトインツールをAIToolConfig形式に変換
  const builtinConfigs: AIToolConfig[] = BUILTIN_TOOLS.map((tool) => ({
    id: tool.id,
    displayName: tool.displayName,
    ...(tool.icon ? { icon: tool.icon } : {}),
    isBuiltin: true,
  }));

  // カスタムツールをAIToolConfig形式に変換
  const customConfigs: AIToolConfig[] = config.customTools.map((tool) => ({
    id: tool.id,
    displayName: tool.displayName,
    ...(tool.icon ? { icon: tool.icon } : {}),
    isBuiltin: false,
    customConfig: tool,
  }));

  // ビルトイン + カスタム の順で統合
  return [...builtinConfigs, ...customConfigs];
}
