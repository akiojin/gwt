export type BranchPrefix = "feature/" | "bugfix/" | "hotfix/" | "release/" | "";

/**
 * Determine branch prefix from GitHub issue labels.
 * Returns null if no deterministic mapping exists (AI fallback needed).
 */
export function determinePrefixFromLabels(labels: { name: string }[]): BranchPrefix | null {
  const names = labels.map(l => l.name.toLowerCase());
  if (names.includes("hotfix")) return "hotfix/";
  if (names.includes("bug")) return "bugfix/";
  return null;
}

export type AgentId = "claude" | "codex" | "gemini" | "opencode";

function normalizeAgentId(raw: string | null | undefined): string {
  return (raw ?? "").trim().toLowerCase();
}

export function inferAgentId(raw: string | null | undefined): AgentId | null {
  const value = normalizeAgentId(raw);
  if (value.includes("claude")) return "claude";
  if (value.includes("codex")) return "codex";
  if (value.includes("gemini")) return "gemini";
  if (value.includes("opencode") || value.includes("open-code")) return "opencode";
  return null;
}
