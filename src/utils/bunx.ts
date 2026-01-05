import { existsSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";

const WINDOWS_EXTENSIONS = [".exe", ".cmd", ".bat", ".ps1"] as const;

type WindowsExtension = (typeof WINDOWS_EXTENSIONS)[number];

function findWindowsCommand(
  commandName: string,
  extensions: readonly WindowsExtension[] = WINDOWS_EXTENSIONS,
): string | null {
  const pathEntries = (process.env.PATH ?? "")
    .split(path.delimiter)
    .filter(Boolean);

  for (const entry of pathEntries) {
    for (const ext of extensions) {
      const candidate = path.join(entry, `${commandName}${ext}`);
      if (existsSync(candidate)) {
        return candidate;
      }
    }
  }

  return null;
}

function resolveWindowsBunCommand(commandName: "bun" | "bunx"): string | null {
  const fromPath = findWindowsCommand(commandName);
  if (fromPath) {
    return fromPath;
  }

  const bunInstall = process.env.BUN_INSTALL ?? path.join(homedir(), ".bun");
  const bunBin = path.join(bunInstall, "bin");

  for (const ext of WINDOWS_EXTENSIONS) {
    const candidate = path.join(bunBin, `${commandName}${ext}`);
    if (existsSync(candidate)) {
      return candidate;
    }
  }

  return null;
}

function buildPowerShellInvocation(
  scriptPath: string,
  args: string[],
): { command: string; args: string[] } {
  return {
    command: "powershell.exe",
    args: [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      scriptPath,
      ...args,
    ],
  };
}

export function buildBunxInvocation(args: string[]): {
  command: string;
  args: string[];
} {
  if (process.platform !== "win32") {
    return { command: "bunx", args };
  }

  const bunxPath = resolveWindowsBunCommand("bunx");
  if (!bunxPath) {
    return { command: "bunx", args };
  }

  if (bunxPath.toLowerCase().endsWith(".ps1")) {
    return buildPowerShellInvocation(bunxPath, args);
  }

  return { command: bunxPath, args };
}

export function buildBunInvocation(args: string[]): {
  command: string;
  args: string[];
} {
  if (process.platform !== "win32") {
    return { command: "bun", args };
  }

  const bunPath = resolveWindowsBunCommand("bun");
  if (!bunPath) {
    return { command: "bun", args };
  }

  if (bunPath.toLowerCase().endsWith(".ps1")) {
    return buildPowerShellInvocation(bunPath, args);
  }

  return { command: bunPath, args };
}

export function isBunxAvailable(): boolean {
  if (process.platform !== "win32") {
    return true;
  }

  return resolveWindowsBunCommand("bunx") !== null;
}
