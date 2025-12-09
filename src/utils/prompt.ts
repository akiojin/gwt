import readline from "node:readline";
import { getTerminalStreams } from "./terminal.js";

/**
 * Wait for Enter using the same terminal streams as Ink.
 * Falls back to no-op on non-interactive stdin to avoid blocking pipelines.
 */
export async function waitForEnter(promptMessage: string): Promise<void> {
  const terminal = getTerminalStreams();
  const stdin = terminal.stdin as NodeJS.ReadStream | undefined;
  const stdout = terminal.stdout as NodeJS.WriteStream | undefined;

  if (!stdin || typeof stdin.on !== "function" || !stdin.isTTY) {
    return;
  }

  terminal.exitRawMode?.();

  if (typeof stdin.resume === "function") {
    stdin.resume();
  }

  if ((stdin as NodeJS.ReadStream & { isRaw?: boolean }).isRaw) {
    try {
      (stdin as NodeJS.ReadStream & { setRawMode?: (flag: boolean) => void }).setRawMode?.(false);
    } catch {
      // Ignore raw mode errors
    }
  }

  await new Promise<void>((resolve) => {
    const rl = readline.createInterface({ input: stdin, output: stdout });

    const cleanup = () => {
      rl.removeAllListeners();
      rl.close();
      const remover = (method: "off" | "removeListener") =>
        (stdin as unknown as Record<string, (event: string, fn: () => void) => void>)[method]?.(
          "end",
          onEnd,
        );
      remover("off");
      remover("removeListener");
      const removerErr = (method: "off" | "removeListener") =>
        (stdin as unknown as Record<string, (event: string, fn: () => void) => void>)[method]?.(
          "error",
          onEnd,
        );
      removerErr("off");
      removerErr("removeListener");
      if (typeof stdin.pause === "function") {
        stdin.pause();
      }
    };

    const onEnd = () => {
      cleanup();
      resolve();
    };

    rl.on("SIGINT", () => {
      cleanup();
      process.exit(0);
    });

    rl.question(`${promptMessage}\n`, () => {
      cleanup();
      resolve();
    });

    stdin.once("end", onEnd);
    stdin.once("error", onEnd);
  });
}
