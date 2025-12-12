import { PassThrough } from "node:stream";
import { describe, expect, it, vi } from "vitest";

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

describe("waitForEnter", () => {
  it("uses terminal stdin/stdout and resolves after newline on TTY", async () => {
    vi.resetModules();
    for (const key of Object.keys(terminalStreams)) {
      delete terminalStreams[key];
    }

    const stdin = new PassThrough() as unknown as NodeJS.ReadStream;
    const stdout = new PassThrough() as unknown as NodeJS.WriteStream;
    Object.defineProperty(stdin, "isTTY", { value: true });

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

    const exitRawMode = vi.fn();

    Object.assign(terminalStreams, {
      stdin,
      stdout,
      stderr: stdout,
      usingFallback: false,
      exitRawMode,
    });

    const { waitForEnter } = await import("../prompt.js");

    const waiting = withTimeout(waitForEnter("prompt"), 200);
    stdin.write("hello\n");

    await expect(waiting).resolves.toBeUndefined();
    expect(resumed).toBe(true);
    expect(paused).toBe(true);
    expect(exitRawMode).toHaveBeenCalled();
  });

  it("returns immediately on non-TTY stdin", async () => {
    vi.resetModules();
    for (const key of Object.keys(terminalStreams)) {
      delete terminalStreams[key];
    }

    const stdin = new PassThrough() as unknown as NodeJS.ReadStream;
    const stdout = new PassThrough() as unknown as NodeJS.WriteStream;
    Object.defineProperty(stdin, "isTTY", { value: false });

    Object.assign(terminalStreams, {
      stdin,
      stdout,
      stderr: stdout,
      usingFallback: false,
      exitRawMode: vi.fn(),
    });

    const { waitForEnter } = await import("../prompt.js");

    const start = Date.now();
    await waitForEnter("prompt");
    expect(Date.now() - start).toBeLessThan(50);
  });
});
