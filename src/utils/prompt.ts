import readline from "node:readline";
import { getTerminalStreams } from "./terminal.js";

type ReadlinePromptOptions<T> = {
  fallback: T;
  onAnswer: (answer: string) => T | undefined;
  shouldSkip?: (terminal: ReturnType<typeof getTerminalStreams>) => boolean;
};

async function runReadlinePrompt<T>(
  promptText: string,
  { fallback, onAnswer, shouldSkip }: ReadlinePromptOptions<T>,
): Promise<T> {
  const terminal = getTerminalStreams();
  const stdin = terminal.stdin as NodeJS.ReadStream | undefined;
  const stdout = terminal.stdout as NodeJS.WriteStream | undefined;

  if (!stdin || typeof stdin.on !== "function" || !stdin.isTTY) {
    return fallback;
  }

  if (shouldSkip?.(terminal)) {
    return fallback;
  }

  terminal.exitRawMode?.();

  if (typeof stdin.resume === "function") {
    stdin.resume();
  }

  if ((stdin as NodeJS.ReadStream & { isRaw?: boolean }).isRaw) {
    try {
      (
        stdin as NodeJS.ReadStream & { setRawMode?: (flag: boolean) => void }
      ).setRawMode?.(false);
    } catch {
      // Ignore raw mode errors
    }
  }

  return new Promise<T>((resolve) => {
    const rl = readline.createInterface({ input: stdin, output: stdout });
    let finished = false;

    const cleanup = () => {
      if (finished) {
        return;
      }
      finished = true;
      rl.removeAllListeners();
      rl.close();
      const remover = (method: "off" | "removeListener") =>
        (
          stdin as unknown as Record<
            string,
            (event: string, fn: () => void) => void
          >
        )[method]?.("end", onEnd);
      remover("off");
      remover("removeListener");
      const removerErr = (method: "off" | "removeListener") =>
        (
          stdin as unknown as Record<
            string,
            (event: string, fn: () => void) => void
          >
        )[method]?.("error", onEnd);
      removerErr("off");
      removerErr("removeListener");
      if (typeof stdin.pause === "function") {
        stdin.pause();
      }
    };

    const onEnd = () => {
      cleanup();
      resolve(fallback);
    };

    rl.on("SIGINT", () => {
      cleanup();
      process.exit(0);
    });

    rl.question(`${promptText}\n`, (answer) => {
      const result = onAnswer(answer);
      cleanup();
      resolve(result !== undefined ? result : fallback);
    });

    stdin.once("end", onEnd);
    stdin.once("error", onEnd);
  });
}

/**
 * Wait for Enter using the same terminal streams as Ink.
 * Falls back to no-op on non-interactive stdin to avoid blocking pipelines.
 */
export async function waitForEnter(promptMessage: string): Promise<void> {
  await runReadlinePrompt(promptMessage, {
    fallback: undefined,
    onAnswer: () => undefined,
  });
}

/**
 * Prompts the user for a yes/no confirmation in the terminal.
 * Falls back to the default value on non-interactive stdin or fallback terminals.
 *
 * @param promptMessage - The message to display to the user
 * @param options - Configuration options
 * @param options.defaultValue - The default value when input is empty or stdin is non-interactive
 * @returns A promise that resolves to true for yes, false for no
 */
export async function confirmYesNo(
  promptMessage: string,
  options: { defaultValue?: boolean } = {},
): Promise<boolean> {
  const fallback = options.defaultValue ?? false;

  const suffix =
    options.defaultValue === undefined
      ? "[y/n]"
      : options.defaultValue
        ? "[Y/n]"
        : "[y/N]";

  const promptText = `${promptMessage} ${suffix}`.trim();

  return runReadlinePrompt(promptText, {
    fallback,
    shouldSkip: (terminal) => terminal.usingFallback,
    onAnswer: (answer) => {
      const normalized = answer.trim().toLowerCase();
      if (normalized === "y" || normalized === "yes") {
        return true;
      }
      if (normalized === "n" || normalized === "no") {
        return false;
      }
      if (normalized.length === 0 && options.defaultValue !== undefined) {
        return options.defaultValue;
      }
      return undefined;
    },
  });
}
