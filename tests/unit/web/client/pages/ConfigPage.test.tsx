import React from "react";
// import type { Mock } - use bun:test mock types
import { describe, it, expect, beforeEach,  mock } from "bun:test";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { ApiCodingAgent } from "../../../../../src/types/api.js";
import { ConfigPage } from "../../../../../src/web/client/src/pages/ConfigPage.js";
import {
  useConfig,
  useUpdateConfig,
} from "../../../../../src/web/client/src/hooks/useConfig.js";

mock.module("../../../../../src/web/client/src/hooks/useConfig.js", () => ({
  useConfig: mock(),
  useUpdateConfig: mock(),
}));

const mockedUseConfig = useConfig as unknown as Mock;
const mockedUseUpdateConfig = useUpdateConfig as unknown as Mock;

const sampleAgents: ApiCodingAgent[] = [
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
  const mutateAsync = mock();

  beforeEach(() => {
    mutateAsync.mockReset();
    mockedUseConfig.mockReturnValue({
      data: { codingAgents: sampleAgents, env: [], version: "1" },
      isLoading: false,
      error: null,
    });
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
    expect(screen.getByText("Coding Agent を追加")).toBeInTheDocument();
  });

  it("adds a new custom tool", async () => {
    const newTool = {
      id: "my-tool",
      displayName: "My Tool",
      executionType: "bunx" as const,
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

    mutateAsync.mockResolvedValue({
      codingAgents: [...sampleAgents, newTool],
      env: [],
      version: "2",
    });

    renderPage();

    fireEvent.click(screen.getByText("Coding Agent を追加"));

    const idInput = screen
      .getByText("Agent ID *")
      .parentElement?.querySelector("input");
    const nameInput = screen
      .getByText("表示名 *")
      .parentElement?.querySelector("input");

    if (!idInput || !nameInput) {
      throw new Error("フォーム入力欄が見つかりません");
    }

    fireEvent.change(idInput, { target: { value: "my-tool" } });
    fireEvent.change(nameInput, { target: { value: "My Tool" } });

    const commandInput = screen
      .getByText("パッケージ名 *")
      .parentElement?.querySelector("input");
    if (!commandInput) {
      throw new Error("パッケージ名入力欄が見つかりません");
    }
    fireEvent.change(commandInput, { target: { value: "aider" } });

    const formTitle = screen.getByText("新規 Coding Agent");
    const form = formTitle.closest("form");
    if (!form) {
      throw new Error("フォームが見つかりません");
    }
    fireEvent.click(within(form).getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(mutateAsync).toHaveBeenCalled();
    });

    const payload = mutateAsync.mock.calls[0][0];
    expect(payload.env).toEqual([]);
    expect(payload.codingAgents).toHaveLength(2);
    expect(payload.codingAgents[1]).toMatchObject({
      id: "my-tool",
      command: "aider",
      executionType: "bunx",
    });
  });
});
