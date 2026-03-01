export const AGENT_INSTRUCTION_DOC_FILES = [
  "CLAUDE.md",
  "AGENTS.md",
  "GEMINI.md",
] as const;

export type DocsEditorShellId = "wsl" | "powershell" | "cmd";

export function isWindowsPlatform(platform: string): boolean {
  return platform.toLowerCase().includes("win");
}

export function shouldAutoCloseDocsEditorTab(
  platform: string,
  shellId?: DocsEditorShellId,
): boolean {
  if (!isWindowsPlatform(platform)) return true;
  return shellId === "wsl";
}

export function isTerminalProcessEnded(status: string): boolean {
  const normalized = status.trim().toLowerCase();
  return normalized.startsWith("completed") || normalized.startsWith("error");
}

export function buildDocsEditorCommand(
  platform: string,
  shellId?: DocsEditorShellId,
): string {
  const files = AGENT_INSTRUCTION_DOC_FILES.join(" ");
  const codeArgs = AGENT_INSTRUCTION_DOC_FILES.map((file) => `-g ${file}`).join(
    " ",
  );
  const powershellNotepadFallback = AGENT_INSTRUCTION_DOC_FILES.map(
    (file) => `notepad ${file}`,
  ).join("; ");
  const cmdNotepadFallback = AGENT_INSTRUCTION_DOC_FILES.map(
    (file) => `notepad ${file}`,
  ).join(" & ");
  if (isWindowsPlatform(platform)) {
    if (shellId === "wsl") {
      return `vi ${files}; exit`;
    }
    if (shellId === "powershell") {
      return `if (Get-Command code -ErrorAction SilentlyContinue) { code ${codeArgs} } else { ${powershellNotepadFallback} }`;
    }
    return `where code >NUL 2>&1 && (code ${codeArgs}) || (${cmdNotepadFallback})`;
  }

  return `vi ${files}; exit`;
}
