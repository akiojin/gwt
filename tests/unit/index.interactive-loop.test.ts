import type { Mock } from "bun:test";
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

const waitForUserAcknowledgementMock = mock<() => Promise<void>>();

const mockTerminalStreams = {
  stdin: { isTTY: false, on: () => {} } as unknown as NodeJS.ReadStream,
  stdout: { write: () => {} } as unknown as NodeJS.WriteStream,
  stderr: { write: () => {} } as unknown as NodeJS.WriteStream,
  usingFallback: false,
  exitRawMode: mock(),
};

const mockChildStdio = {
  stdin: "inherit",
  stdout: "inherit",
  stderr: "inherit",
  cleanup: mock(),
};

mock.module("../../src/utils/terminal.js", () => ({
  getTerminalStreams: mock(() => mockTerminalStreams),
  resetTerminalModes: mock(),
  writeTerminalLine: mock(),
  createChildStdio: mock(() => mockChildStdio),
  waitForUserAcknowledgement: waitForUserAcknowledgementMock,
}));

// Import after mocks are set up
import { runInteractiveLoop } from "../../src/index.js";

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
