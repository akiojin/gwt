export const AGENT_INSTRUCTION_DOC_FILES = [
  "CLAUDE.md",
  "AGENTS.md",
  "GEMINI.md",
] as const;

export type DocsEditorShellId = "wsl" | "powershell" | "cmd";

/**
 * Returns whether the runtime platform should be treated as Windows.
 * @param platform Platform label from the frontend runtime environment.
 * @returns True when the platform string includes `win`.
 */
export function isWindowsPlatform(platform: string): boolean {
  return platform.toLowerCase().includes("win");
}

/**
 * Docs editor tabs are auto-closed only for vi-based flows.
 * @param platform Platform label from the frontend runtime environment.
 * @param shellId Optional configured shell id for Windows (`wsl`/`powershell`/`cmd`).
 * @returns True when docs tabs should auto-close after terminal completion.
 */
export function shouldAutoCloseDocsEditorTab(
  platform: string,
  shellId?: DocsEditorShellId,
): boolean {
  if (!isWindowsPlatform(platform)) return true;
  return shellId === "wsl";
}

/**
 * Terminal status values returned by list_terminals use string labels.
 * completed(*) and error:* indicate the interactive process has finished.
 * @param status Terminal status string returned by `list_terminals`.
 * @returns True when the process status is `completed*` or `error*`.
 */
export function isTerminalProcessEnded(status: string): boolean {
  const normalized = status.trim().toLowerCase();
  return normalized.startsWith("completed") || normalized.startsWith("error");
}

/**
 * Builds the shell command for opening CLAUDE/AGENTS/GEMINI instruction files.
 * @param platform Platform label from the frontend runtime environment.
 * @param shellId Optional configured shell id for Windows (`wsl`/`powershell`/`cmd`).
 * @returns Shell command string to open/edit instruction files.
 */
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
