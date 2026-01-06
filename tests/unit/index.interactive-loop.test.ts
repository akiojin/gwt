import { beforeEach, describe, expect, it, mock, spyOn } from "bun:test";
import type { SelectionResult } from "../../src/cli/ui/App.solid.js";
import { runInteractiveLoop } from "../../src/index.js";

// Vitest shim for environments lacking vi.hoisted (e.g., bun)
if (typeof (vi as Record<string, unknown>).hoisted !== "function") {
  // @ts-expect-error injected shim
}

const waitForUserAcknowledgementMock = (
  (mock<() => Promise<void>>()),
);

mock.module("../../src/utils/terminal.js", async () => {
  const actual = await import(
    typeof import("../../src/utils/terminal.js")
  >("../../src/utils/terminal.js");
  return {
    ...actual,
    waitForUserAcknowledgement: waitForUserAcknowledgementMock,
  };
});

describe("runInteractiveLoop", () => {
  const baseSelection: SelectionResult = {
    branch: "feature/example",
    displayName: "feature/example",
    branchType: "local",
    tool: "codex-cli",
    mode: "normal",
    skipPermissions: false,
    model: "gpt-5.2-codex",
  };

  let consoleLogSpy: Mock;
  let consoleErrorSpy: Mock;
  let consoleWarnSpy: Mock;

  beforeEach(() => {
    waitForUserAcknowledgementMock.mockReset();
    waitForUserAcknowledgementMock.mockResolvedValue(undefined);
    consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});
    consoleErrorSpy = spyOn(console, "error").mockImplementation(() => {});
    consoleWarnSpy = spyOn(console, "warn").mockImplementation(() => {});
  });

  it("re-renders the UI after workflow errors instead of exiting", async () => {
    const uiHandler = vi
      .fn<() => Promise<SelectionResult | undefined>>()
      .mockResolvedValueOnce(baseSelection)
      .mockResolvedValueOnce(undefined);

    const workflowHandler = vi
      .fn<(selection: SelectionResult) => Promise<void>>()
      .mockRejectedValueOnce(new Error("codex failed"));

    await runInteractiveLoop(uiHandler, workflowHandler);

    expect(uiHandler).toHaveBeenCalledTimes(2);
    expect(workflowHandler).toHaveBeenCalledTimes(1);
    expect(waitForUserAcknowledgementMock).toHaveBeenCalledTimes(1);
  });

  it("recovers when the UI handler throws and allows retry", async () => {
    const uiHandler = vi
      .fn<() => Promise<SelectionResult | undefined>>()
      .mockRejectedValueOnce(new Error("ui crash"))
      .mockResolvedValueOnce(undefined);

    const workflowHandler = mock<
      (selection: SelectionResult) => Promise<void>
    >(async () => {});

    await runInteractiveLoop(uiHandler, workflowHandler);

    expect(uiHandler).toHaveBeenCalledTimes(2);
    expect(workflowHandler).not.toHaveBeenCalled();
    expect(waitForUserAcknowledgementMock).toHaveBeenCalledTimes(1);
  });

  afterEach(() => {
    consoleLogSpy.mockRestore();
    consoleErrorSpy.mockRestore();
    consoleWarnSpy.mockRestore();
  });
});
