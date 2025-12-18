import fs from "node:fs";
import { platform } from "node:os";
import { ReadStream, WriteStream } from "node:tty";

/**
 * Terminal streams used by the CLI (stdin/stdout/stderr) and their raw-mode
 * teardown helper.
 *
 * When the current process is not a TTY, this may fall back to `/dev/tty` so
 * interactive child processes can still read/write correctly.
 */
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
// 端末モードのリセット:
// - ESC[?1l: application cursor keys (DECCKM) を無効化
// - ESC>: keypad mode を normal/numeric に戻す
// WSL2/Windowsの矢印キー入力が壊れるケースを抑止する。
const TERMINAL_RESET_SEQUENCE = "\u001b[?1l\u001b>";

let cachedStreams: TerminalStreams | null = null;

/**
 * Stdio configuration for launching an interactive child process (via `execa`),
 * plus a cleanup hook to be called after the child exits.
 */
export interface ChildStdio {
  stdin: "inherit" | { file: string; append?: boolean };
  stdout: "inherit" | { file: string; append?: boolean };
  stderr: "inherit" | { file: string; append?: boolean };
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

/**
 * Returns cached terminal streams and falls back to `/dev/tty` when needed.
 */
export function getTerminalStreams(): TerminalStreams {
  if (!cachedStreams) {
    cachedStreams = createTerminalStreams();
  }
  return cachedStreams;
}

/**
 * Best-effort terminal mode reset for interactive sessions.
 *
 * This writes a small ANSI sequence to restore cursor-key/keypad modes, which
 * helps prevent broken arrow-key behavior on some Windows/WSL2 terminals after
 * interactive CLIs exit.
 */
export function resetTerminalModes(
  stdout: NodeJS.WriteStream | undefined,
): void {
  if (!stdout || typeof stdout.write !== "function") {
    return;
  }
  if (!("isTTY" in stdout) || !stdout.isTTY) {
    return;
  }
  try {
    stdout.write(TERMINAL_RESET_SEQUENCE);
  } catch {
    // Ignore terminal reset errors.
  }
}

/**
 * Creates stdio settings for launching a child process.
 *
 * When terminal streams are backed by `/dev/tty`, forwards those file
 * descriptors to the child so it remains interactive.
 */
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

  return {
    stdin: { file: DEV_TTY_PATH },
    stdout: { file: DEV_TTY_PATH },
    stderr: { file: DEV_TTY_PATH },
    cleanup: () => {},
  };
}

function isInteractive(stream: NodeJS.ReadStream): boolean {
  return Boolean(stream.isTTY);
}

/**
 * Prints a message and waits for the user to press Enter when running in an
 * interactive terminal.
 *
 * Useful for pausing on errors while ensuring raw mode is disabled.
 */
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
