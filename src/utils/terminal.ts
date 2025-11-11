import fs from "node:fs";
import { platform } from "node:os";
import { ReadStream, WriteStream } from "node:tty";

export interface TerminalStreams {
  stdin: NodeJS.ReadStream;
  stdout: NodeJS.WriteStream;
  stderr: NodeJS.WriteStream;
  stdinFd?: number;
  stdoutFd?: number;
  stderrFd?: number;
  usingFallback: boolean;
  exitRawMode: () => void;
}

const DEV_TTY_PATH = "/dev/tty";

let cachedStreams: TerminalStreams | null = null;

export interface ChildStdio {
  stdin: "inherit" | number;
  stdout: "inherit" | number;
  stderr: "inherit" | number;
  cleanup: () => void;
}

const DEFAULT_ACK_MESSAGE =
  "Review the error details, then press Enter to continue...";

function isProcessTTY(): boolean {
  return Boolean(
    process.stdin.isTTY &&
      process.stdout.isTTY &&
      process.stderr.isTTY &&
      typeof (process.stdin as NodeJS.ReadStream).setRawMode === "function",
  );
}

function createTerminalStreams(): TerminalStreams {
  if (isProcessTTY()) {
    const exitRawMode = () => {
      const stream = process.stdin as NodeJS.ReadStream;
      if (typeof stream.setRawMode === "function") {
        try {
          stream.setRawMode(false);
        } catch {
          // Ignore errors when resetting raw mode.
        }
      }
    };

    return {
      stdin: process.stdin,
      stdout: process.stdout,
      stderr: process.stderr,
      usingFallback: false,
      exitRawMode,
    };
  }

  // Windows では /dev/tty が利用できないため、そのまま返す。
  if (platform() === "win32") {
    return {
      stdin: process.stdin,
      stdout: process.stdout,
      stderr: process.stderr,
      usingFallback: false,
      exitRawMode: () => {
        const stream = process.stdin as NodeJS.ReadStream;
        if (typeof stream.setRawMode === "function") {
          try {
            stream.setRawMode(false);
          } catch {
            // Ignore errors when resetting raw mode.
          }
        }
      },
    };
  }

  try {
    const fdIn = fs.openSync(DEV_TTY_PATH, "r");
    const fdOut = fs.openSync(DEV_TTY_PATH, "w");
    const fdErr = fs.openSync(DEV_TTY_PATH, "w");

    const stdin = new ReadStream(fdIn);
    const stdout = new WriteStream(fdOut);
    const stderr = new WriteStream(fdErr);

    const exitRawMode = () => {
      if (typeof stdin.setRawMode === "function") {
        try {
          stdin.setRawMode(false);
        } catch {
          // Ignore errors when resetting raw mode.
        }
      }
    };

    const cleanup = () => {
      exitRawMode();
      try {
        stdin.destroy();
      } catch {
        // Ignore stdin destroy errors.
      }
      try {
        stdout.destroy();
      } catch {
        // Ignore stdout destroy errors.
      }
      try {
        stderr.destroy();
      } catch {
        // Ignore stderr destroy errors.
      }
      try {
        fs.closeSync(fdIn);
      } catch {
        // Ignore close errors.
      }
      try {
        fs.closeSync(fdOut);
      } catch {
        // Ignore close errors.
      }
      try {
        fs.closeSync(fdErr);
      } catch {
        // Ignore close errors.
      }
    };

    process.once("exit", cleanup);

    return {
      stdin,
      stdout,
      stderr,
      stdinFd: fdIn,
      stdoutFd: fdOut,
      stderrFd: fdErr,
      usingFallback: true,
      exitRawMode,
    };
  } catch {
    const exitRawMode = () => {
      const stream = process.stdin as NodeJS.ReadStream;
      if (typeof stream.setRawMode === "function") {
        try {
          stream.setRawMode(false);
        } catch {
          // Ignore errors when resetting raw mode.
        }
      }
    };

    return {
      stdin: process.stdin,
      stdout: process.stdout,
      stderr: process.stderr,
      usingFallback: false,
      exitRawMode,
    };
  }
}

export function getTerminalStreams(): TerminalStreams {
  if (!cachedStreams) {
    cachedStreams = createTerminalStreams();
  }
  return cachedStreams;
}

export function createChildStdio(): ChildStdio {
  const terminal = getTerminalStreams();

  if (!terminal.usingFallback) {
    return {
      stdin: "inherit",
      stdout: "inherit",
      stderr: "inherit",
      cleanup: () => {},
    };
  }

  let fdIn: number | null = null;
  let fdOut: number | null = null;
  let fdErr: number | null = null;

  const cleanup = () => {
    for (const fd of [fdIn, fdOut, fdErr]) {
      if (fd !== null) {
        try {
          fs.closeSync(fd);
        } catch {
          // Ignore close errors.
        }
      }
    }
  };

  try {
    fdIn = fs.openSync(DEV_TTY_PATH, "r");
    fdOut = fs.openSync(DEV_TTY_PATH, "w");
    fdErr = fs.openSync(DEV_TTY_PATH, "w");

    return {
      stdin: fdIn,
      stdout: fdOut,
      stderr: fdErr,
      cleanup,
    };
  } catch {
    cleanup();
    return {
      stdin: "inherit",
      stdout: "inherit",
      stderr: "inherit",
      cleanup: () => {},
    };
  }
}

function isInteractive(stream: NodeJS.ReadStream): boolean {
  return Boolean(stream.isTTY);
}

export async function waitForUserAcknowledgement(
  message: string = DEFAULT_ACK_MESSAGE,
): Promise<void> {
  const terminal = getTerminalStreams();
  const stdin = terminal.stdin as NodeJS.ReadStream;
  const stdout = terminal.stdout as NodeJS.WriteStream;

  if (!stdin || typeof stdin.on !== "function") {
    return;
  }

  if (!isInteractive(stdin)) {
    return;
  }

  terminal.exitRawMode();

  await new Promise<void>((resolve) => {
    const cleanup = () => {
      stdin.removeListener("data", onData);
      if (typeof stdin.pause === "function") {
        stdin.pause();
      }
    };

    const onData = (chunk: Buffer | string) => {
      const data = typeof chunk === "string" ? chunk : chunk.toString("utf8");
      if (data.includes("\n") || data.includes("\r")) {
        cleanup();
        resolve();
      }
    };

    if (typeof stdout?.write === "function") {
      stdout.write(`\n${message}\n`);
    }

    if (typeof stdin.resume === "function") {
      stdin.resume();
    }

    stdin.on("data", onData);
  });
}
