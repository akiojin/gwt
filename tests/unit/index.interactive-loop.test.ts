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
const writeMock = mock();
const mockStreams = {
  stdin: process.stdin,
  stdout: { write: writeMock } as NodeJS.WriteStream,
  stderr: { write: writeMock } as NodeJS.WriteStream,
  stdinFd: undefined as number | undefined,
  stdoutFd: undefined as number | undefined,
  stderrFd: undefined as number | undefined,
  usingFallback: false,
  exitRawMode: mock(),
};
const mockChildStdio = {
  stdin: "inherit" as const,
  stdout: "inherit" as const,
  stderr: "inherit" as const,
  cleanup: mock(),
};

mock.module("../../src/utils/terminal.js", () => ({
  getTerminalStreams: mock(() => mockStreams),
  resetTerminalModes: mock(),
  waitForUserAcknowledgement: waitForUserAcknowledgementMock,
  writeTerminalLine: mock(),
  createChildStdio: mock(() => mockChildStdio),
}));

// Import after mocks are set up (using underscore prefix to indicate intentional non-use at top level)
import { runInteractiveLoop as _runInteractiveLoop } from "../../src/index.js";

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
    const { runInteractiveLoop } = await import("../../src/index.js");
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
    const { runInteractiveLoop } = await import("../../src/index.js");
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
