import { PassThrough } from "node:stream";
import { describe, expect, it, vi, beforeEach } from "vitest";

// Shared mock target to avoid hoisting issues
const terminalStreams: Record<string, unknown> = {};

vi.mock("../terminal.js", () => ({
  getTerminalStreams: () => terminalStreams,
}));

const withTimeout = <T>(promise: Promise<T>, ms = 500): Promise<T> =>
  Promise.race([
    promise,
    new Promise<T>((_, reject) =>
      setTimeout(() => reject(new Error("timeout")), ms),
    ),
  ]);

const resetTerminalStreams = () => {
  vi.resetModules();
  for (const key of Object.keys(terminalStreams)) {
    delete terminalStreams[key];
  }
};

const setupTerminalStreams = (isTTY: boolean) => {
  const stdin = new PassThrough() as unknown as NodeJS.ReadStream;
  const stdout = new PassThrough() as unknown as NodeJS.WriteStream;
  Object.defineProperty(stdin, "isTTY", { value: isTTY, configurable: true });
  const exitRawMode = vi.fn();
  Object.assign(terminalStreams, {
    stdin,
    stdout,
    stderr: stdout,
    usingFallback: false,
    exitRawMode,
  });
  return { stdin, stdout, exitRawMode };
};

describe("waitForEnter", () => {
  it("uses terminal stdin/stdout and resolves after newline on TTY", async () => {
    resetTerminalStreams();
    const { stdin, exitRawMode } = setupTerminalStreams(true);

    let resumed = false;
    let paused = false;
    const originalResume = stdin.resume.bind(stdin);
    const originalPause = stdin.pause.bind(stdin);
    // Track resume/pause calls
    stdin.resume = (() => {
      resumed = true;
      return originalResume();
    }) as typeof stdin.resume;
    stdin.pause = (() => {
      paused = true;
      return originalPause();
    }) as typeof stdin.pause;

    const { waitForEnter } = await import("../prompt.js");

    const waiting = withTimeout(waitForEnter("prompt"), 200);
    stdin.write("hello\n");

    await expect(waiting).resolves.toBeUndefined();
    expect(resumed).toBe(true);
    expect(paused).toBe(true);
    expect(exitRawMode).toHaveBeenCalled();
  });

  it("returns immediately on non-TTY stdin", async () => {
    resetTerminalStreams();
    setupTerminalStreams(false);

    const { waitForEnter } = await import("../prompt.js");

    const start = Date.now();
    await waitForEnter("prompt");
    expect(Date.now() - start).toBeLessThan(50);
  });
});

describe("confirmYesNo", () => {
  let stdin: NodeJS.ReadStream;
  let exitRawMode: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    resetTerminalStreams();
    const setup = setupTerminalStreams(true);
    stdin = setup.stdin;
    exitRawMode = setup.exitRawMode;
  });

  it("resolves true when user inputs y on TTY", async () => {
    const { confirmYesNo } = await import("../prompt.js");

    const waiting = withTimeout(confirmYesNo("push?"), 200);
    stdin.write("y\n");

    await expect(waiting).resolves.toBe(true);
    expect(exitRawMode).toHaveBeenCalled();
  });

  it("uses default value when input is empty on TTY", async () => {
    const { confirmYesNo } = await import("../prompt.js");

    const waiting = withTimeout(
      confirmYesNo("push?", { defaultValue: true }),
      200,
    );
    stdin.write("\n");

    await expect(waiting).resolves.toBe(true);
  });

  it("returns default immediately on non-TTY stdin", async () => {
    Object.defineProperty(stdin, "isTTY", { value: false });

    const { confirmYesNo } = await import("../prompt.js");

    const start = Date.now();
    const result = await confirmYesNo("push?", { defaultValue: false });
    expect(result).toBe(false);
    expect(Date.now() - start).toBeLessThan(50);
  });
});
