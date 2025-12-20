/**
 * Shared environment variable helpers.
 *
 * 管理画面やサーバー側の設定ロジックから共通で利用する
 * 環境変数スキーマ/バリデーション/マージ処理を集約する。
 */

const ENV_KEY_PATTERN = /^[A-Z0-9_]+$/;
const MAX_KEY_LENGTH = 100;
const MAX_VALUE_LENGTH = 500;

const DEFAULT_BOOTSTRAP_ENV_KEYS = [
  "ANTHROPIC_API_KEY",
  "ANTHROPIC_API_KEY_PATH",
  "ANTHROPIC_API_BASE",
  "OPENAI_API_KEY",
  "OPENAI_BASE_URL",
  "OPENAI_API_BASE",
  "OPENAI_ORG_ID",
  "OPENAI_PROJECT",
  "OPENAI_API_TYPE",
  "OPENAI_API_VERSION",
  "GITHUB_TOKEN",
  "GH_TOKEN",
  "GITLAB_TOKEN",
  "AZURE_OPENAI_API_KEY",
  "AZURE_OPENAI_ENDPOINT",
] as const;

export type EnvironmentRecord = Record<string, string>;

/**
 * 環境変数キーが有効かどうかを判定。
 */
export function isValidEnvKey(key: string): boolean {
  if (!key || typeof key !== "string") {
    return false;
  }
  if (key.length === 0 || key.length > MAX_KEY_LENGTH) {
    return false;
  }
  return ENV_KEY_PATTERN.test(key);
}

/**
 * 許容されるキー長を取得。
 */
export function getEnvConstraints(): {
  maxKeyLength: number;
  maxValueLength: number;
} {
  return {
    maxKeyLength: MAX_KEY_LENGTH,
    maxValueLength: MAX_VALUE_LENGTH,
  };
}

/**
 * 任意のレコードをサニタイズして string -> string のみに整形する。
 */
export function sanitizeEnvRecord(
  record?: Record<string, unknown>,
): EnvironmentRecord {
  if (!record || typeof record !== "object") {
    return {};
  }
  const result: EnvironmentRecord = {};
  for (const [key, value] of Object.entries(record)) {
    if (value === undefined || value === null) {
      continue;
    }
    result[key] = String(value);
  }
  return result;
}

/**
 * envレコードを検証し、フォーマットが不正な場合は例外を投げる。
 */
export function validateEnvRecord(record: EnvironmentRecord): void {
  for (const [key, value] of Object.entries(record)) {
    if (!isValidEnvKey(key)) {
      throw new Error(
        `Invalid environment variable key: "${key}". Use 1-${MAX_KEY_LENGTH} characters (A-Z, 0-9, underscore).`,
      );
    }
    if (value.length === 0) {
      throw new Error(`Environment variable "${key}" must not be empty.`);
    }
    if (value.length > MAX_VALUE_LENGTH) {
      throw new Error(
        `Environment variable "${key}" exceeds the maximum length (${MAX_VALUE_LENGTH}).`,
      );
    }
  }
}

/**
 * 起動時にOSから読み取って取り込みたい環境変数キー一覧を返す。
 * CLAUDE_WORKTREE_BOOTSTRAP_ENV_KEYS=KEY1,KEY2 で上書き可能。
 */
export function getBootstrapEnvKeys(): string[] {
  const override = process.env.CLAUDE_WORKTREE_BOOTSTRAP_ENV_KEYS;
  if (!override) {
    return [...DEFAULT_BOOTSTRAP_ENV_KEYS];
  }
  return override
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean)
    .map((entry) => entry.toUpperCase());
}

/**
 * 現在のenvレコードへ、指定されたキーのみOS環境変数から補完する。
 */
export function mergeWithBootstrapEnv(
  current: EnvironmentRecord,
  sourceEnv: NodeJS.ProcessEnv,
  keys: string[] = getBootstrapEnvKeys(),
): { merged: EnvironmentRecord; addedKeys: string[] } {
  const merged: EnvironmentRecord = { ...current };
  const addedKeys: string[] = [];

  for (const key of keys) {
    if (merged[key] !== undefined) {
      continue; // 既に設定済み
    }
    const value = sourceEnv[key];
    if (typeof value === "string" && value.length > 0) {
      merged[key] = value;
      addedKeys.push(key);
    }
  }

  return { merged, addedKeys };
}

export { ENV_KEY_PATTERN };
