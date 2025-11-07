import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SelectionResult } from "../../src/ui/components/App.js";
import { runInteractiveLoop } from "../../src/index.js";

const waitForUserAcknowledgementMock = vi.hoisted(() =>
  vi.fn<() => Promise<void>>(),
);

vi.mock("../../src/utils/terminal.js", async () => {
  const actual = await vi.importActual<
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
    tool: "codex-cli" as any,
    mode: "normal" as any,
    skipPermissions: false,
  };

  let consoleLogSpy: ReturnType<typeof vi.spyOn>;
  let consoleErrorSpy: ReturnType<typeof vi.spyOn>;
  let consoleWarnSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    waitForUserAcknowledgementMock.mockReset();
    waitForUserAcknowledgementMock.mockResolvedValue(undefined);
    consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});
    consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    consoleWarnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
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

    const workflowHandler = vi.fn<(selection: SelectionResult) => Promise<void>>(
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
