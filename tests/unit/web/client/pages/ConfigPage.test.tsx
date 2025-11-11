import React from "react";
import type { Mock } from "vitest";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { CustomAITool } from "../../../../../src/types/api.js";
import { ConfigPage } from "../../../../../src/web/client/src/pages/ConfigPage.js";
import {
  useConfig,
  useUpdateConfig,
} from "../../../../../src/web/client/src/hooks/useConfig.js";

vi.mock("../../../../../src/web/client/src/hooks/useConfig.js", () => ({
  useConfig: vi.fn(),
  useUpdateConfig: vi.fn(),
}));

const mockedUseConfig = useConfig as unknown as Mock;
const mockedUseUpdateConfig = useUpdateConfig as unknown as Mock;

const sampleTools: CustomAITool[] = [
  {
    id: "claude-code",
    displayName: "Claude Code",
    executionType: "bunx",
    command: "@anthropic-ai/claude-code@latest",
    defaultArgs: null,
    modeArgs: { normal: [], continue: [], resume: [] },
    permissionSkipArgs: null,
    env: null,
    icon: null,
    description: null,
    createdAt: "2025-11-10T00:00:00Z",
    updatedAt: "2025-11-10T00:00:00Z",
  },
];

describe("ConfigPage", () => {
  const mutateAsync = vi.fn();

  beforeEach(() => {
    mutateAsync.mockReset();
    mockedUseConfig.mockReturnValue({ data: { tools: sampleTools }, isLoading: false, error: null });
    mockedUseUpdateConfig.mockReturnValue({ mutateAsync, isPending: false });
  });

  const renderPage = () =>
    render(
      <MemoryRouter>
        <ConfigPage />
      </MemoryRouter>,
    );

  it("renders existing tools", () => {
    renderPage();
    expect(screen.getByText("Claude Code")).toBeInTheDocument();
    expect(screen.getByText("カスタムツールを追加")).toBeInTheDocument();
  });

  it("adds a new custom tool", async () => {
    const newTool = {
      id: "my-tool",
      displayName: "My Tool",
      executionType: "command" as const,
      command: "aider",
      modeArgs: { normal: [], continue: [], resume: [] },
      defaultArgs: null,
      permissionSkipArgs: null,
      env: null,
      icon: null,
      description: null,
      createdAt: "2025-11-11T00:00:00Z",
      updatedAt: "2025-11-11T00:00:00Z",
    };

    mutateAsync.mockResolvedValue({ tools: [...sampleTools, newTool] });

    renderPage();

    fireEvent.click(screen.getByText("カスタムツールを追加"));

    fireEvent.change(screen.getByLabelText("ツールID *"), { target: { value: "my-tool" } });
    fireEvent.change(screen.getByLabelText("表示名 *"), { target: { value: "My Tool" } });
    fireEvent.change(screen.getByLabelText("実行タイプ *"), { target: { value: "command" } });
    fireEvent.change(screen.getByLabelText(/コマンド/i), { target: { value: "aider" } });

    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(mutateAsync).toHaveBeenCalled();
    });
  });
});
