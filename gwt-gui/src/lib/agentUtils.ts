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
