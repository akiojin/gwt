import type { SessionData, ToolSessionEntry } from "../../../config/index.js";
import { findLatestSessionIdForTool } from "../../../utils/session.js";

export interface ContinueSessionContext {
  history: ToolSessionEntry[];
  sessionData: SessionData | null;
  branch: string;
  toolId: string;
  repoRoot: string | null;
  lookupLatestSessionId?: (
    toolId: string,
    cwd: string,
  ) => Promise<string | null>;
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
  const {
    history,
    sessionData,
    branch,
    toolId,
    repoRoot,
    lookupLatestSessionId = findLatestSessionIdForTool,
  } = context;

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

  // 3) それでも無ければ、同一ブランチ/ツールの直近セッションをツール固有の保存場所から検索
  if (
    sessionData?.lastBranch === branch &&
    sessionData.lastUsedTool === toolId
  ) {
    return lookupLatestSessionId(toolId, repoRoot ?? process.cwd());
  }

  return null;
}
