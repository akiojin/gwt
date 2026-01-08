import * as pty from "node-pty";
import type { IPty } from "node-pty";
import { createLogger } from "./logger.js";
import { resolveLogDir } from "./reader.js";
import { getTerminalStreams } from "../utils/terminal.js";

export const CAPTURE_AGENT_OUTPUT_ENV = "GWT_CAPTURE_AGENT_OUTPUT";

export function shouldCaptureAgentOutput(
  env: NodeJS.ProcessEnv = process.env,
): boolean {
  const raw = env[CAPTURE_AGENT_OUTPUT_ENV];
  if (!raw) {
    return false;
  }
  const normalized = String(raw).trim().toLowerCase();
  return normalized === "true" || normalized === "1";
}

// eslint-disable-next-line no-control-regex
const ANSI_PATTERN = new RegExp("\\u001b\\[[0-9;]*[A-Za-z]", "g");

export function stripAnsi(value: string): string {
  return value.replace(ANSI_PATTERN, "");
}

export interface AgentOutputLineBuffer {
  push: (chunk: string) => void;
  flush: () => void;
}

export function createAgentOutputLineBuffer(
  onLine: (line: string) => void,
): AgentOutputLineBuffer {
  let buffer = "";

  const push = (chunk: string) => {
    const normalized = chunk.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
    buffer += normalized;
    while (true) {
      const index = buffer.indexOf("\n");
      if (index === -1) {
        break;
      }
      const line = buffer.slice(0, index);
      buffer = buffer.slice(index + 1);
      onLine(line);
    }
  };

  const flush = () => {
    if (!buffer) {
      return;
    }
    const line = buffer;
    buffer = "";
    onLine(line);
  };

  return { push, flush };
}

export interface AgentOutputCaptureOptions {
  command: string;
  args: string[];
  cwd: string;
  env: NodeJS.ProcessEnv;
  agentId: string;
}

export interface AgentOutputCaptureResult {
  exitCode: number | null;
  signal?: number | null;
}

const normalizeEnv = (env: NodeJS.ProcessEnv): Record<string, string> =>
  Object.fromEntries(
    Object.entries(env).filter(
      (entry): entry is [string, string] => typeof entry[1] === "string",
    ),
  );

const getTerminalSize = (terminal: ReturnType<typeof getTerminalStreams>) => {
  const stdout = terminal.stdout as
    | (NodeJS.WriteStream & { columns?: number; rows?: number })
    | undefined;
  const cols = stdout?.columns ?? process.stdout.columns ?? 80;
  const rows = stdout?.rows ?? process.stdout.rows ?? 24;
  return { cols, rows };
};

export async function runAgentWithPty(
  options: AgentOutputCaptureOptions,
): Promise<AgentOutputCaptureResult> {
  const terminal = getTerminalStreams();
  const { cols, rows } = getTerminalSize(terminal);
  const logDir = resolveLogDir(options.cwd);
  const stdoutLogger = createLogger({
    category: "agent.stdout",
    logDir,
    base: { agentId: options.agentId },
  });
  const stderrLogger = createLogger({
    category: "agent.stderr",
    logDir,
    base: { agentId: options.agentId },
  });

  const normalizedEnv = normalizeEnv(options.env);
  const ptyProcess: IPty = pty.spawn(options.command, options.args, {
    name: process.env.TERM ?? "xterm-256color",
    cols,
    rows,
    cwd: options.cwd,
    env: normalizedEnv,
  });

  const lineBuffer = createAgentOutputLineBuffer((line) => {
    const cleaned = stripAnsi(line).trimEnd();
    if (!cleaned) {
      return;
    }
    stdoutLogger.info(cleaned);
  });

  const stdout = terminal.stdout;
  const writeToTerminal =
    stdout && typeof stdout.write === "function"
      ? stdout.write.bind(stdout)
      : null;

  ptyProcess.onData((data) => {
    if (writeToTerminal) {
      try {
        writeToTerminal(data);
      } catch {
        // Ignore terminal write errors.
      }
    }
    lineBuffer.push(data);
  });

  const stdin = terminal.stdin;
  const handleInput = (chunk: Buffer | string) => {
    const data = typeof chunk === "string" ? chunk : chunk.toString("utf8");
    ptyProcess.write(data);
  };

  const stdinWasRaw =
    stdin &&
    typeof (stdin as NodeJS.ReadStream & { isRaw?: boolean }).isRaw ===
      "boolean"
      ? (stdin as NodeJS.ReadStream & { isRaw?: boolean }).isRaw
      : undefined;

  if (stdin && typeof stdin.on === "function") {
    if (stdin.isTTY && typeof stdin.setRawMode === "function") {
      try {
        stdin.setRawMode(true);
      } catch {
        // Ignore raw mode errors.
      }
    }
    if (typeof stdin.resume === "function") {
      stdin.resume();
    }
    stdin.on("data", handleInput);
  }

  const handleResize = () => {
    const next = getTerminalSize(terminal);
    try {
      ptyProcess.resize(next.cols, next.rows);
    } catch {
      // Ignore resize errors.
    }
  };
  if (process.stdout && typeof process.stdout.on === "function") {
    process.stdout.on("resize", handleResize);
  }

  return await new Promise<AgentOutputCaptureResult>((resolve) => {
    ptyProcess.onExit(({ exitCode, signal }) => {
      lineBuffer.flush();
      if (stdin && typeof stdin.off === "function") {
        stdin.off("data", handleInput);
      }
      if (stdin && typeof stdin.pause === "function") {
        stdin.pause();
      }
      if (stdin && stdin.isTTY && typeof stdin.setRawMode === "function") {
        try {
          stdin.setRawMode(Boolean(stdinWasRaw));
        } catch {
          // Ignore raw mode restore errors.
        }
      }
      if (process.stdout && typeof process.stdout.off === "function") {
        process.stdout.off("resize", handleResize);
      }
      if (exitCode !== null && exitCode !== 0) {
        stderrLogger.error(
          { exitCode, signal },
          "Agent exited with non-zero code",
        );
      }
      resolve({ exitCode, signal });
    });
  });
}
