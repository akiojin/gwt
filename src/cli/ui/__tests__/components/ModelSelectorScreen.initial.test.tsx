/**
 * @vitest-environment happy-dom
 */
import React from "react";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, waitFor } from "@testing-library/react";
import { ModelSelectorScreen } from "../../components/screens/ModelSelectorScreen.js";
import type { ModelSelectionResult } from "../../components/screens/ModelSelectorScreen.js";
import { Window } from "happy-dom";

const selectMocks: any[] = [];

vi.mock("../../components/common/Select.js", () => {
  return {
    Select: (props: any) => {
      selectMocks.push(props);
      return React.createElement("div", {
        "data-testid": "select-mock",
        onClick: () => props.onSelect && props.onSelect(props.items[props.initialIndex ?? 0]),
      });
    },
  };
});

describe("ModelSelectorScreen initial selection", () => {
  beforeEach(() => {
    selectMocks.length = 0;
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  it("sets model list initialIndex based on previous selection", async () => {
    const initial: ModelSelectionResult = {
      model: "gpt-5.1-codex-max",
      inferenceLevel: "high",
    };

    render(
      <ModelSelectorScreen
        tool="codex-cli"
        onBack={() => {}}
        onSelect={() => {}}
        initialSelection={initial}
      />,
    );

    await waitFor(() => expect(selectMocks.length).toBeGreaterThan(0));
    const modelSelect = selectMocks.at(-1);
    const index = modelSelect.initialIndex as number;
    // codex-cli models: [gpt-5.1-codex, gpt-5.1-codex-max, gpt-5.1-codex-mini, gpt-5.1]
    expect(index).toBe(1);
  });

  it("sets inference list initialIndex based on previous reasoning level", async () => {
    const initial: ModelSelectionResult = {
      model: "gpt-5.1-codex-max",
      inferenceLevel: "high",
    };

    render(
      <ModelSelectorScreen
        tool="codex-cli"
        onBack={() => {}}
        onSelect={() => {}}
        initialSelection={initial}
      />,
    );

    // trigger onSelect for model to render inference step
    await waitFor(() => expect(selectMocks.length).toBeGreaterThan(0));
    const modelSelect = selectMocks[0];
    modelSelect.onSelect(modelSelect.items[modelSelect.initialIndex]);

    await waitFor(() => expect(selectMocks.length).toBeGreaterThan(1));
    const inferenceSelect = selectMocks[1];
    const index = inferenceSelect.initialIndex as number;
    // inference order for codex-max: [xhigh, high, medium, low]; "high" should be index 1
    expect(index).toBe(1);
  });
});
