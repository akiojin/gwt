import type { SessionData, ToolSessionEntry } from "../../../config/index.js";
import type { SessionSearchOptions } from "../../../utils/session/index.js";
import {
  findLatestClaudeSession,
  findLatestCodexSession,
  findLatestGeminiSession,
  findLatestOpenCodeSession,
} from "../../../utils/session/index.js";
import { listAllWorktrees } from "../../../worktree.js";

export interface ContinueSessionContext {
  history: ToolSessionEntry[];
  sessionData: SessionData | null;
  branch: string;
  toolId: string;
  repoRoot: string | null;
}

/**
 * 指定されたブランチ/ツールに紐づく最新セッションIDを解決する。
 * 1. 履歴(history)の最新マッチを優先
 * 2. lastSessionId がブランチ/ツール一致であれば利用
 * 3. それでも無い場合、同一ブランチ/ツールであればツール固有の保存場所から検出
 */
export async function resolveContinueSessionId(
  context: ContinueSessionContext,
): Promise<string | null> {
  const { history, sessionData, branch, toolId, repoRoot: _repoRoot } = context;

  // 1) 履歴から最新マッチを探す（末尾から遡る）
  for (let i = history.length - 1; i >= 0; i -= 1) {
    const entry = history[i];
    if (
      entry &&
      entry.branch === branch &&
      entry.toolId === toolId &&
      entry.sessionId
    ) {
      return entry.sessionId;
    }
  }

  // 2) lastSessionId が一致する場合はそれを返す
  if (
    sessionData?.lastSessionId &&
    sessionData.lastBranch === branch &&
    sessionData.lastUsedTool === toolId
  ) {
    return sessionData.lastSessionId;
  }

  return null;
}

export function findLatestBranchSession(
  history: ToolSessionEntry[],
  branch: string,
  toolId?: string | null,
): ToolSessionEntry | null {
  const byBranch = history.filter((entry) => entry && entry.branch === branch);
  if (!byBranch.length) return null;

  const pickLatest = (entries: ToolSessionEntry[]) =>
    entries.reduce<ToolSessionEntry | null>((latest, entry) => {
      if (!latest) return entry;
      const latestTs = latest.timestamp ?? 0;
      const currentTs = entry.timestamp ?? 0;
      return currentTs >= latestTs ? entry : latest;
    }, null);

  if (toolId) {
    const byTool = byBranch.filter((entry) => entry.toolId === toolId);
    if (byTool.length) {
      return pickLatest(byTool);
    }
  }

  return pickLatest(byBranch);
}

export function findLatestBranchSessionsByTool(
  history: ToolSessionEntry[],
  branch: string,
  worktreePath?: string | null,
): ToolSessionEntry[] {
  const byBranch = history.filter((entry) => entry && entry.branch === branch);
  if (!byBranch.length) return [];

  const source = worktreePath
    ? byBranch.filter((entry) => entry.worktreePath === worktreePath)
    : byBranch;
  if (!source.length) return [];

  const latestByTool = new Map<string, ToolSessionEntry>();
  for (const entry of source) {
    if (!entry.toolId) continue;
    const current = latestByTool.get(entry.toolId);
    const currentTs = current?.timestamp ?? 0;
    const entryTs = entry.timestamp ?? 0;
    if (!current || entryTs >= currentTs) {
      latestByTool.set(entry.toolId, entry);
    }
  }

  return Array.from(latestByTool.values()).sort(
    (a, b) => (b.timestamp ?? 0) - (a.timestamp ?? 0),
  );
}

export interface QuickStartRefreshContext {
  branch: string;
  worktreePath?: string | null;
}

export interface QuickStartSessionLookups {
  findLatestCodexSession?: typeof findLatestCodexSession;
  findLatestClaudeSession?: typeof findLatestClaudeSession;
  findLatestGeminiSession?: typeof findLatestGeminiSession;
  findLatestOpenCodeSession?: typeof findLatestOpenCodeSession;
  listAllWorktrees?: typeof listAllWorktrees;
}

export async function refreshQuickStartEntries(
  entries: ToolSessionEntry[],
  context: QuickStartRefreshContext,
  lookups: QuickStartSessionLookups = {},
): Promise<ToolSessionEntry[]> {
  if (!entries.length) return entries;
  const worktreePath = context.worktreePath ?? null;
  if (!worktreePath) return entries;

  const lookupWorktrees = lookups.listAllWorktrees ?? listAllWorktrees;
  let resolvedWorktrees: { path: string; branch: string }[] | null = null;
  try {
    const allWorktrees = await lookupWorktrees();
    resolvedWorktrees = allWorktrees
      .filter((entry) => entry?.path && entry?.branch)
      .map((entry) => ({ path: entry.path, branch: entry.branch }));
  } catch {
    resolvedWorktrees = null;
  }

  const searchOptions: SessionSearchOptions = {
    branch: context.branch,
    ...(resolvedWorktrees && resolvedWorktrees.length > 0
      ? { worktrees: resolvedWorktrees }
      : {}),
  };

  const lookupCodex = lookups.findLatestCodexSession ?? findLatestCodexSession;
  const lookupClaude =
    lookups.findLatestClaudeSession ?? findLatestClaudeSession;
  const lookupGemini =
    lookups.findLatestGeminiSession ?? findLatestGeminiSession;
  const lookupOpenCode =
    lookups.findLatestOpenCodeSession ?? findLatestOpenCodeSession;

  const updated = await Promise.all(
    entries.map(async (entry) => {
      let latest: { id: string; mtime: number } | null = null;
      switch (entry.toolId) {
        case "codex-cli":
          latest = await lookupCodex(searchOptions);
          break;
        case "claude-code":
          latest = await lookupClaude(worktreePath, searchOptions);
          break;
        case "gemini-cli":
          latest = await lookupGemini(searchOptions);
          break;
        case "opencode":
          latest = await lookupOpenCode(searchOptions);
          break;
        default:
          return entry;
      }

      if (!latest?.id) return entry;
      const updatedTimestamp = Math.max(entry.timestamp ?? 0, latest.mtime);
      return {
        ...entry,
        sessionId: latest.id,
        timestamp: updatedTimestamp,
      };
    }),
  );

  return updated.sort((a, b) => (b.timestamp ?? 0) - (a.timestamp ?? 0));
}
