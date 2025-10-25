import { homedir } from "node:os";
import path from "node:path";
import { readFile, writeFile, mkdir } from "node:fs/promises";

export interface AppConfig {
  defaultBaseBranch: string;
  skipPermissions: boolean;
  enableGitHubIntegration: boolean;
  enableDebugMode: boolean;
  worktreeNamingPattern: string;
}

export interface SessionData {
  lastWorktreePath: string | null;
  lastBranch: string | null;
  timestamp: number;
  repositoryRoot: string;
}

const DEFAULT_CONFIG: AppConfig = {
  defaultBaseBranch: "main",
  skipPermissions: false,
  enableGitHubIntegration: true,
  enableDebugMode: false,
  worktreeNamingPattern: "{repo}-{branch}",
};

/**
 * 設定ファイルを読み込む
 */
export async function loadConfig(): Promise<AppConfig> {
  const configPaths = [
    path.join(process.cwd(), ".claude-worktree.json"),
    path.join(homedir(), ".config", "claude-worktree", "config.json"),
    path.join(homedir(), ".claude-worktree.json"),
  ];

  for (const configPath of configPaths) {
    try {
      const content = await readFile(configPath, "utf-8");
      const userConfig = JSON.parse(content);
      return { ...DEFAULT_CONFIG, ...userConfig };
    } catch (error) {
      // 設定ファイルが見つからない場合は次を試す
      if (process.env.DEBUG_CONFIG) {
        console.error(
          `Failed to load config from ${configPath}:`,
          error instanceof Error ? error.message : String(error),
        );
      }
    }
  }

  // 環境変数からも読み込み
  return {
    ...DEFAULT_CONFIG,
    defaultBaseBranch:
      process.env.CLAUDE_WORKTREE_BASE_BRANCH ||
      DEFAULT_CONFIG.defaultBaseBranch,
    skipPermissions: process.env.CLAUDE_WORKTREE_SKIP_PERMISSIONS === "true",
    enableGitHubIntegration: process.env.CLAUDE_WORKTREE_GITHUB !== "false",
    enableDebugMode:
      process.env.DEBUG_CLEANUP === "true" || process.env.DEBUG === "true",
  };
}

/**
 * 設定値を取得する
 */
let cachedConfig: AppConfig | null = null;

export async function getConfig(): Promise<AppConfig> {
  if (!cachedConfig) {
    cachedConfig = await loadConfig();
  }
  return cachedConfig;
}

export function resetConfigCache(): void {
  cachedConfig = null;
}

/**
 * セッションデータの保存・読み込み
 */
function getSessionFilePath(repositoryRoot: string): string {
  const sessionDir = path.join(
    homedir(),
    ".config",
    "claude-worktree",
    "sessions",
  );
  const repoName = path.basename(repositoryRoot);
  const repoHash = Buffer.from(repositoryRoot)
    .toString("base64")
    .replace(/[/+=]/g, "_");
  return path.join(sessionDir, `${repoName}_${repoHash}.json`);
}

export async function saveSession(sessionData: SessionData): Promise<void> {
  try {
    const sessionPath = getSessionFilePath(sessionData.repositoryRoot);
    const sessionDir = path.dirname(sessionPath);

    // ディレクトリを作成
    await mkdir(sessionDir, { recursive: true });

    await writeFile(sessionPath, JSON.stringify(sessionData, null, 2), "utf-8");
  } catch (error) {
    // セッション保存の失敗は致命的ではないため、エラーをログに出力するのみ
    if (process.env.DEBUG_SESSION) {
      console.error(
        "Failed to save session:",
        error instanceof Error ? error.message : String(error),
      );
    }
  }
}

export async function loadSession(
  repositoryRoot: string,
): Promise<SessionData | null> {
  try {
    const sessionPath = getSessionFilePath(repositoryRoot);
    const content = await readFile(sessionPath, "utf-8");
    const sessionData = JSON.parse(content) as SessionData;

    // セッションが24時間以内のもののみ有効とする
    const now = Date.now();
    const sessionAge = now - sessionData.timestamp;
    const maxAge = 24 * 60 * 60 * 1000; // 24時間

    if (sessionAge > maxAge) {
      return null;
    }

    return sessionData;
  } catch (error) {
    if (process.env.DEBUG_SESSION) {
      console.error(
        "Failed to load session:",
        error instanceof Error ? error.message : String(error),
      );
    }
    return null;
  }
}

export async function getAllSessions(): Promise<SessionData[]> {
  try {
    const sessionDir = path.join(
      homedir(),
      ".config",
      "claude-worktree",
      "sessions",
    );
    const { readdir } = await import("node:fs/promises");

    const files = await readdir(sessionDir);
    const sessions: SessionData[] = [];
    const now = Date.now();
    const maxAge = 24 * 60 * 60 * 1000; // 24時間

    for (const file of files) {
      if (!file.endsWith(".json")) continue;

      try {
        const filePath = path.join(sessionDir, file);
        const content = await readFile(filePath, "utf-8");
        const sessionData = JSON.parse(content) as SessionData;

        // 有効期限内のセッションのみ
        const sessionAge = now - sessionData.timestamp;
        if (sessionAge <= maxAge) {
          sessions.push(sessionData);
        }
      } catch (error) {
        if (process.env.DEBUG_SESSION) {
          console.error(
            `Failed to load session file ${file}:`,
            error instanceof Error ? error.message : String(error),
          );
        }
      }
    }

    // 最新のものから順にソート
    sessions.sort((a, b) => b.timestamp - a.timestamp);

    return sessions;
  } catch (error) {
    if (process.env.DEBUG_SESSION) {
      console.error(
        "Failed to get all sessions:",
        error instanceof Error ? error.message : String(error),
      );
    }
    return [];
  }
}
