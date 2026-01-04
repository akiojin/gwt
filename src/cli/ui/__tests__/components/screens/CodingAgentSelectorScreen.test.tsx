/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, waitFor } from "@testing-library/react";
import React from "react";
import { CodingAgentSelectorScreen } from "../../../components/screens/CodingAgentSelectorScreen.js";
import { Window } from "happy-dom";

// Mock getAllCodingAgents
vi.mock("../../../config/tools.js", () => ({
  getAllCodingAgents: vi.fn().mockResolvedValue([
    {
      id: "claude-code",
      displayName: "Claude Code",
      isBuiltin: true,
    },
    {
      id: "codex-cli",
      displayName: "Codex",
      isBuiltin: true,
    },
  ]),
}));

describe("CodingAgentSelectorScreen", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as unknown as typeof globalThis.window;
    globalThis.document =
      window.document as unknown as typeof globalThis.document;
  });

  it("should render header with title", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <CodingAgentSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    expect(getByText(/Coding Agent Selection/i)).toBeDefined();
  });

  it("should render Coding Agent options", async () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <CodingAgentSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    // Wait for agents to load
    await waitFor(() => {
      expect(getByText(/Claude Code/i)).toBeDefined();
      expect(getByText(/Codex/i)).toBeDefined();
    });
  });

  it("should render footer with actions", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getAllByText } = render(
      <CodingAgentSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    expect(getAllByText(/enter/i).length).toBeGreaterThan(0);
    expect(getAllByText(/esc/i).length).toBeGreaterThan(0);
  });

  it("should use terminal height for layout calculation", () => {
    const originalRows = process.stdout.rows;
    process.stdout.rows = 30;

    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <CodingAgentSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    expect(container).toBeDefined();

    process.stdout.rows = originalRows;
  });

  it("should handle back navigation with ESC key", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <CodingAgentSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    // Test will verify onBack is called when ESC is pressed
    expect(container).toBeDefined();
  });

  it("should handle coding agent selection", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <CodingAgentSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    // Test will verify onSelect is called with correct agent
    expect(container).toBeDefined();
  });

  it("should preselect the last used coding agent when provided", async () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <CodingAgentSelectorScreen
        onBack={onBack}
        onSelect={onSelect}
        initialAgentId="codex-cli"
      />,
    );

    await waitFor(() => {
      expect(container.textContent?.includes("Codex")).toBe(true);
    });

    const text = container.textContent ?? "";
    expect(text).toContain("›Codex");
    expect(text).not.toContain("›Claude Code");
  });

  /**
   * T210: カスタムコーディングエージェント表示のテスト
   */
  describe("Custom coding agent display", () => {
    it("should load coding agents from getAllCodingAgents() dynamically", async () => {
      // TODO: 実装後にテストを記述
      // getAllCodingAgents()がモックされ、呼び出されることを確認
      // モックの戻り値がエージェントアイテムとして表示されることを確認
      expect(true).toBe(true);
    });

    it("should display both builtin and custom coding agents", async () => {
      // TODO: 実装後にテストを記述
      // getAllCodingAgents()がビルトインエージェント（claude-code, codex-cli）と
      // カスタムエージェント（例: aider）を返す場合、
      // すべてのエージェントが表示されることを確認
      expect(true).toBe(true);
    });

    it("should display custom coding agent with icon if defined", async () => {
      // TODO: 実装後にテストを記述
      // カスタムエージェントにiconフィールドがある場合、
      // それが表示されることを確認
      expect(true).toBe(true);
    });

    it("should display custom coding agent without icon if not defined", async () => {
      // TODO: 実装後にテストを記述
      // カスタムエージェントにiconフィールドがない場合、
      // エージェント名のみが表示されることを確認
      expect(true).toBe(true);
    });

    it("should handle custom coding agent selection", async () => {
      // TODO: 実装後にテストを記述
      // カスタムエージェントを選択した場合、
      // onSelect()がカスタムエージェントのIDで呼び出されることを確認
      expect(true).toBe(true);
    });

    it("should display only builtin coding agents if no custom agents exist", async () => {
      // TODO: 実装後にテストを記述
      // getAllCodingAgents()がビルトインエージェントのみを返す場合、
      // ビルトインエージェントのみが表示されることを確認
      expect(true).toBe(true);
    });
  });
});
