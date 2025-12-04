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
  lastUsedTool?: string;
  timestamp: number;
  repositoryRoot: string;
  mode?: "normal" | "continue" | "resume";
  model?: string | null;
  toolLabel?: string | null;
  history?: ToolSessionEntry[];
}

export interface ToolSessionEntry {
  branch: string;
  worktreePath: string | null;
  toolId: string;
  toolLabel: string;
  mode?: "normal" | "continue" | "resume" | null;
  model?: string | null;
  timestamp: number;
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
    path.join(process.cwd(), ".gwt.json"),
    path.join(process.cwd(), ".claude-worktree.json"), // 後方互換性
    path.join(homedir(), ".config", "gwt", "config.json"),
    path.join(homedir(), ".config", "claude-worktree", "config.json"), // 後方互換性
    path.join(homedir(), ".gwt.json"),
    path.join(homedir(), ".claude-worktree.json"), // 後方互換性
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
  const sessionDir = path.join(homedir(), ".config", "gwt", "sessions");
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

    // 既存履歴を読み込み（後方互換のため失敗は無視）
    let existingHistory: ToolSessionEntry[] = [];
    try {
      const currentContent = await readFile(sessionPath, "utf-8");
      const parsed = JSON.parse(currentContent) as SessionData;
      if (Array.isArray(parsed.history)) {
        existingHistory = parsed.history;
      }
    } catch {
      // ignore
    }

    // 新しい履歴エントリを追加（branch/worktree/toolが揃っている場合のみ）
    if (sessionData.lastBranch && sessionData.lastWorktreePath) {
      const entry: ToolSessionEntry = {
        branch: sessionData.lastBranch,
        worktreePath: sessionData.lastWorktreePath,
        toolId: sessionData.lastUsedTool ?? "unknown",
        toolLabel:
          sessionData.toolLabel ?? sessionData.lastUsedTool ?? "Custom",
        mode: sessionData.mode ?? null,
        model: sessionData.model ?? null,
        timestamp: sessionData.timestamp,
      };
      existingHistory = [...existingHistory, entry].slice(-100); // keep latest 100
    }

    const payload: SessionData = {
      ...sessionData,
      history: existingHistory,
    };

    await writeFile(sessionPath, JSON.stringify(payload, null, 2), "utf-8");
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
    const sessionDir = path.join(homedir(), ".config", "gwt", "sessions");
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

/**
 * 各ブランチの最新ツール利用履歴を取得
 */
export async function getLastToolUsageMap(
  repositoryRoot: string,
): Promise<Map<string, ToolSessionEntry>> {
  const map = new Map<string, ToolSessionEntry>();
  try {
    const sessionPath = getSessionFilePath(repositoryRoot);
    const content = await readFile(sessionPath, "utf-8");
    const parsed = JSON.parse(content) as SessionData;

    const history: ToolSessionEntry[] = Array.isArray(parsed.history)
      ? parsed.history
      : [];

    // 後方互換: historyが無い場合はlastUsedToolを1件扱い
    if (!history.length && parsed.lastBranch && parsed.lastWorktreePath) {
      history.push({
        branch: parsed.lastBranch,
        worktreePath: parsed.lastWorktreePath,
        toolId: parsed.lastUsedTool ?? "unknown",
        toolLabel: parsed.toolLabel ?? parsed.lastUsedTool ?? "Custom",
        mode: parsed.mode ?? null,
        model: parsed.model ?? null,
        timestamp: parsed.timestamp ?? Date.now(),
      });
    }

    for (const entry of history) {
      const existing = map.get(entry.branch);
      if (!existing || existing.timestamp < entry.timestamp) {
        map.set(entry.branch, entry);
      }
    }
  } catch {
    // セッションファイルが無い/壊れている場合は空のMapを返す
  }
  return map;
}
