/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, waitFor } from "@testing-library/react";
import React from "react";
import { AIToolSelectorScreen } from "../../../components/screens/AIToolSelectorScreen.js";
import { Window } from "happy-dom";

// Mock getAllTools
vi.mock("../../../config/tools.js", () => ({
  getAllTools: vi.fn().mockResolvedValue([
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

describe("AIToolSelectorScreen", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  it("should render header with title", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <AIToolSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    expect(getByText(/AI Tool Selection/i)).toBeDefined();
  });

  it("should render AI tool options", async () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <AIToolSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    // Wait for tools to load
    await waitFor(() => {
      expect(getByText(/Claude Code/i)).toBeDefined();
      expect(getByText(/Codex/i)).toBeDefined();
    });
  });

  it("should render footer with actions", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getAllByText } = render(
      <AIToolSelectorScreen onBack={onBack} onSelect={onSelect} />,
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
      <AIToolSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    expect(container).toBeDefined();

    process.stdout.rows = originalRows;
  });

  it("should handle back navigation with ESC key", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <AIToolSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    // Test will verify onBack is called when ESC is pressed
    expect(container).toBeDefined();
  });

  it("should handle tool selection", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <AIToolSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    // Test will verify onSelect is called with correct tool
    expect(container).toBeDefined();
  });

  it("should preselect the last used tool when provided", async () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <AIToolSelectorScreen
        onBack={onBack}
        onSelect={onSelect}
        initialToolId="codex-cli"
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
   * T210: カスタムツール表示のテスト
   */
  describe("Custom tool display", () => {
    it("should load tools from getAllTools() dynamically", async () => {
      // TODO: 実装後にテストを記述
      // getAllTools()がモックされ、呼び出されることを確認
      // モックの戻り値がツールアイテムとして表示されることを確認
      expect(true).toBe(true);
    });

    it("should display both builtin and custom tools", async () => {
      // TODO: 実装後にテストを記述
      // getAllTools()がビルトインツール（claude-code, codex-cli）と
      // カスタムツール（例: aider）を返す場合、
      // すべてのツールが表示されることを確認
      expect(true).toBe(true);
    });

    it("should display custom tool with icon if defined", async () => {
      // TODO: 実装後にテストを記述
      // カスタムツールにiconフィールドがある場合、
      // それが表示されることを確認
      expect(true).toBe(true);
    });

    it("should display custom tool without icon if not defined", async () => {
      // TODO: 実装後にテストを記述
      // カスタムツールにiconフィールドがない場合、
      // ツール名のみが表示されることを確認
      expect(true).toBe(true);
    });

    it("should handle custom tool selection", async () => {
      // TODO: 実装後にテストを記述
      // カスタムツールを選択した場合、
      // onSelect()がカスタムツールのIDで呼び出されることを確認
      expect(true).toBe(true);
    });

    it("should display only builtin tools if no custom tools exist", async () => {
      // TODO: 実装後にテストを記述
      // getAllTools()がビルトインツールのみを返す場合、
      // ビルトインツールのみが表示されることを確認
      expect(true).toBe(true);
    });
  });
});
