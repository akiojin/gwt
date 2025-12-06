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
    const { getByText, getAllByText } = render(
      <BranchQuickStartScreen
        branchName="feature/foo"
        previousOption={{
          toolLabel: "Codex",
          model: "gpt-5.1-codex",
          sessionId: "abc-123",
          inferenceLevel: "high",
        }}
        onBack={() => {}}
        onSelect={() => {}}
      />,
    );

    const titleMatches = getAllByText(/Resume with previous settings/);
    expect(titleMatches.length).toBeGreaterThan(0);
    expect(
      getByText(/Codex \/ gpt-5.1-codex \/ Reasoning: high \/ ID: abc-123/),
    ).toBeDefined();
  });

  it("disables previous options when no history", () => {
    const { getAllByText } = render(
      <BranchQuickStartScreen
        branchName="feature/foo"
        previousOption={null}
        onBack={() => {}}
        onSelect={() => {}}
      />,
    );

    expect(getAllByText(/No previous settings/)).toHaveLength(2);
  });

  it("shows manual selection option", () => {
    const { getByText } = render(
      <BranchQuickStartScreen
        branchName="feature/foo"
        previousOption={{
          toolLabel: "Codex",
          model: "gpt-5.1-codex",
          sessionId: "abc-123",
        }}
        onBack={() => {}}
        onSelect={() => {}}
      />,
    );

    expect(getByText("Choose manually")).toBeDefined();
  });
});
