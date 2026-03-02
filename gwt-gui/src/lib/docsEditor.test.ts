import { describe, expect, it } from "vitest";
import {
  AGENT_INSTRUCTION_DOC_FILES,
  buildDocsEditorCommand,
  isTerminalProcessEnded,
  shouldAutoCloseDocsEditorTab,
} from "./docsEditor";

describe("docsEditor", () => {
  it("builds vi command with exit on non-Windows platforms", () => {
    expect(buildDocsEditorCommand("macOS")).toBe(
      "vi CLAUDE.md AGENTS.md GEMINI.md; exit",
    );
  });

  it("builds vi command with exit for Windows WSL", () => {
    expect(buildDocsEditorCommand("Windows", "wsl")).toBe(
      "vi CLAUDE.md AGENTS.md GEMINI.md; exit",
    );
  });

  it("builds PowerShell code/notepad fallback command", () => {
    expect(buildDocsEditorCommand("Windows", "powershell")).toBe(
      "if (Get-Command code -ErrorAction SilentlyContinue) { code -g CLAUDE.md -g AGENTS.md -g GEMINI.md } else { notepad CLAUDE.md; notepad AGENTS.md; notepad GEMINI.md }",
    );
  });

  it("builds cmd code/notepad fallback command", () => {
    expect(buildDocsEditorCommand("Windows", "cmd")).toBe(
      "where code >NUL 2>&1 && (code -g CLAUDE.md -g AGENTS.md -g GEMINI.md) || (notepad CLAUDE.md & notepad AGENTS.md & notepad GEMINI.md)",
    );
  });

  it("auto-closes only when vi path is used", () => {
    expect(shouldAutoCloseDocsEditorTab("macOS")).toBe(true);
    expect(shouldAutoCloseDocsEditorTab("Linux")).toBe(true);
    expect(shouldAutoCloseDocsEditorTab("Windows", "wsl")).toBe(true);
    expect(shouldAutoCloseDocsEditorTab("Windows", "powershell")).toBe(false);
    expect(shouldAutoCloseDocsEditorTab("Windows", "cmd")).toBe(false);
  });

  it("detects ended terminal states", () => {
    expect(isTerminalProcessEnded("completed(0)")).toBe(true);
    expect(isTerminalProcessEnded("error: failed to launch")).toBe(true);
    expect(isTerminalProcessEnded("running")).toBe(false);
    expect(isTerminalProcessEnded("starting")).toBe(false);
  });

  it("exports docs target files in fixed order", () => {
    expect(AGENT_INSTRUCTION_DOC_FILES).toEqual([
      "CLAUDE.md",
      "AGENTS.md",
      "GEMINI.md",
    ]);
  });
});
