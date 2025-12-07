/**
 * @vitest-environment happy-dom
 */
import React from "react";
import { describe, it, expect, beforeEach } from "vitest";
import { render } from "@testing-library/react";
import { Window } from "happy-dom";
import { BranchQuickStartScreen } from "../../../components/screens/BranchQuickStartScreen.js";

describe("BranchQuickStartScreen", () => {
  beforeEach(() => {
    const window = new Window();
    globalThis.window = window as unknown as typeof globalThis.window;
    globalThis.document =
      window.document as unknown as typeof globalThis.document;
  });

  it("renders previous option details when available", () => {
    const { getByText, getAllByText, queryAllByText } = render(
      <BranchQuickStartScreen
        branchName="feature/foo"
        previousOptions={[
          {
            toolId: "codex-cli",
            toolLabel: "Codex",
            model: "gpt-5.1-codex",
            sessionId: "abc-123",
            inferenceLevel: "high",
            skipPermissions: true,
          },
        ]}
        onBack={() => {}}
        onSelect={() => {}}
      />,
    );

    const titleMatches = getAllByText(/Resume with previous settings/);
    expect(titleMatches.length).toBeGreaterThan(0);
    expect(
      getByText(
        /Model: gpt-5.1-codex \/ Reasoning: High \/ Skip: Yes \/ ID: abc-123/,
      ),
    ).toBeDefined();
    expect(queryAllByText(/ID: abc-123/)).toHaveLength(1);
    expect(
      getByText(/Model: gpt-5.1-codex \/ Reasoning: High \/ Skip: Yes$/),
    ).toBeDefined();
  });

  it("omits reasoning when tool does not support it", () => {
    const { getByText } = render(
      <BranchQuickStartScreen
        branchName="feature/foo"
        previousOptions={[
          {
            toolId: "claude-code",
            toolLabel: "Claude Code",
            model: "opus",
            sessionId: "abc-123",
            inferenceLevel: "xhigh",
            skipPermissions: false,
          },
        ]}
        onBack={() => {}}
        onSelect={() => {}}
      />,
    );

    expect(
      getByText(/Model: opus \/ Skip: No \/ ID: abc-123/),
    ).toBeDefined();
  });

  it("disables previous options when no history", () => {
    const { getAllByText } = render(
      <BranchQuickStartScreen
        branchName="feature/foo"
        previousOptions={[]}
        onBack={() => {}}
        onSelect={() => {}}
      />,
    );

    expect(getAllByText(/No previous settings/)).toHaveLength(1);
  });

  it("shows manual selection option", () => {
    const { getByText } = render(
      <BranchQuickStartScreen
        branchName="feature/foo"
        previousOptions={[
          {
            toolId: "codex-cli",
            toolLabel: "Codex",
            model: "gpt-5.1-codex",
            sessionId: "abc-123",
          },
        ]}
        onBack={() => {}}
        onSelect={() => {}}
      />,
    );

    expect(getByText(/Manual selection/)).toBeDefined();
  });

  it("renders multiple tools separately", () => {
    const { getByText } = render(
      <BranchQuickStartScreen
        branchName="feature/foo"
        previousOptions={[
          {
            toolId: "codex-cli",
            toolLabel: "Codex",
            model: "gpt-5.1-codex",
            sessionId: "codex-123",
            inferenceLevel: "high",
            skipPermissions: true,
          },
          {
            toolId: "claude-code",
            toolLabel: "Claude Code",
            model: "opus",
            sessionId: "claude-999",
            skipPermissions: false,
          },
        ]}
        onBack={() => {}}
        onSelect={() => {}}
      />,
    );

    expect(getByText(/\[Codex\]/i)).toBeDefined();
    expect(
      getByText(/Model: gpt-5.1-codex \/ Reasoning: High \/ Skip: Yes \/ ID: codex-123/),
    ).toBeDefined();
    expect(
      getByText(/\[Claude\]/i),
    ).toBeDefined();
    expect(
      getByText(/Model: opus \/ Skip: No \/ ID: claude-999/),
    ).toBeDefined();
  });
});
