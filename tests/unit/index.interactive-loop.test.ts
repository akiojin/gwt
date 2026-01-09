import {
  beforeEach,
  afterEach,
  describe,
  expect,
  it,
  mock,
  spyOn,
} from "bun:test";
import type { SelectionResult } from "../../src/cli/ui/App.solid.js";
import { runInteractiveLoop } from "../../src/index.js";

const waitForUserAcknowledgementMock = mock<() => Promise<void>>();

mock.module("../../src/utils/terminal.js", async () => {
  const actual = await import("../../src/utils/terminal.js");
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
    const uiHandler = mock<() => Promise<SelectionResult | undefined>>();
    uiHandler
      .mockResolvedValueOnce(baseSelection)
      .mockResolvedValueOnce(undefined);

    const workflowHandler =
      mock<(selection: SelectionResult) => Promise<void>>();
    workflowHandler.mockRejectedValueOnce(new Error("codex failed"));

    await runInteractiveLoop(uiHandler, workflowHandler);

    expect(uiHandler).toHaveBeenCalledTimes(2);
    expect(workflowHandler).toHaveBeenCalledTimes(1);
    expect(waitForUserAcknowledgementMock).toHaveBeenCalledTimes(1);
  });

  it("recovers when the UI handler throws and allows retry", async () => {
    const uiHandler = mock<() => Promise<SelectionResult | undefined>>();
    uiHandler
      .mockRejectedValueOnce(new Error("ui crash"))
      .mockResolvedValueOnce(undefined);

    const workflowHandler = mock<(selection: SelectionResult) => Promise<void>>(
      async () => {},
    );

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
