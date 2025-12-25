import { execa } from "execa";
import { existsSync } from "fs";
import { homedir } from "os";
import { join } from "path";

/**
 * Known installation paths for common AI CLI tools.
 * These are checked as fallback when `which`/`where` fails.
 */
const KNOWN_INSTALL_PATHS: Record<string, { unix: string[]; win32: string[] }> =
  {
    claude: {
      unix: [
        join(homedir(), ".bun", "bin", "claude"),
        join(homedir(), ".local", "bin", "claude"),
        "/usr/local/bin/claude",
      ],
      win32: [
        join(
          process.env.LOCALAPPDATA ?? "",
          "Programs",
          "claude",
          "claude.exe",
        ),
        join(homedir(), ".bun", "bin", "claude.exe"),
      ],
    },
    codex: {
      unix: [
        join(homedir(), ".bun", "bin", "codex"),
        join(homedir(), ".local", "bin", "codex"),
        "/usr/local/bin/codex",
      ],
      win32: [join(homedir(), ".bun", "bin", "codex.exe")],
    },
    gemini: {
      unix: [
        join(homedir(), ".bun", "bin", "gemini"),
        join(homedir(), ".local", "bin", "gemini"),
        "/usr/local/bin/gemini",
      ],
      win32: [join(homedir(), ".bun", "bin", "gemini.exe")],
    },
  };

/**
 * Builtin AI tools with their command names and display names.
 */
const BUILTIN_TOOLS = [
  { id: "claude-code", commandName: "claude", displayName: "Claude" },
  { id: "codex-cli", commandName: "codex", displayName: "Codex" },
  { id: "gemini-cli", commandName: "gemini", displayName: "Gemini" },
] as const;

/**
 * Result of command lookup.
 */
export interface CommandLookupResult {
  available: boolean;
  path: string | null;
  source: "installed" | "bunx";
}

/**
 * Tool status information for display.
 */
export interface ToolStatus {
  id: string;
  name: string;
  status: "installed" | "bunx";
  path: string | null;
}

/**
 * Module-level cache for command lookup results.
 * This cache persists for the lifetime of the process (FR-020).
 */
const commandLookupCache = new Map<string, CommandLookupResult>();

/**
 * Clears the command lookup cache.
 * Primarily for testing purposes.
 */
export function clearCommandLookupCache(): void {
  commandLookupCache.clear();
}

/**
 * Finds a command by checking PATH first, then fallback paths.
 * Results are cached for the lifetime of the process (FR-020).
 *
 * @param commandName - Command name to look up (e.g. `claude`, `codex`, `gemini`)
 * @returns CommandLookupResult with availability, path, and source
 */
export async function findCommand(
  commandName: string,
): Promise<CommandLookupResult> {
  // Check cache first (FR-020: 再検出を行わない)
  const cached = commandLookupCache.get(commandName);
  if (cached) {
    return cached;
  }

  let lookupResult: CommandLookupResult | null = null;

  // Step 1: Try standard which/where lookup
  try {
    const lookupCommand = process.platform === "win32" ? "where" : "which";
    const execResult = await execa(lookupCommand, [commandName], {
      shell: true,
      stdin: "ignore",
      stdout: "pipe",
      stderr: "ignore",
    });
    const foundPath = execResult.stdout.trim().split("\n")[0];
    if (foundPath) {
      lookupResult = { available: true, path: foundPath, source: "installed" };
    }
  } catch {
    // which/where failed, try fallback paths
  }

  // Step 2: Check known installation paths as fallback
  if (!lookupResult) {
    const knownPaths = KNOWN_INSTALL_PATHS[commandName];
    if (knownPaths) {
      const pathsToCheck =
        process.platform === "win32" ? knownPaths.win32 : knownPaths.unix;

      for (const p of pathsToCheck) {
        if (p && existsSync(p)) {
          lookupResult = { available: true, path: p, source: "installed" };
          break;
        }
      }
    }
  }

  // Step 3: Fall back to bunx (always available for known tools)
  if (!lookupResult) {
    lookupResult = { available: true, path: null, source: "bunx" };
  }

  // Cache the result (FR-020)
  commandLookupCache.set(commandName, lookupResult);

  return lookupResult;
}

/**
 * Checks whether a command is available in the current PATH.
 *
 * Uses `where` on Windows and `which` on other platforms.
 * If the standard lookup fails, checks known installation paths
 * as a fallback for common tools.
 *
 * @param commandName - Command name to look up (e.g. `claude`, `npx`, `gemini`)
 * @returns true if the command exists in PATH or known paths
 */
export async function isCommandAvailable(
  commandName: string,
): Promise<boolean> {
  const result = await findCommand(commandName);
  return result.available;
}

/**
 * Detects installation status for all builtin AI tools.
 *
 * This function is designed to be called once at application startup
 * and cached for the duration of the session.
 *
 * @returns Array of ToolStatus for all builtin tools
 */
export async function detectAllToolStatuses(): Promise<ToolStatus[]> {
  const results = await Promise.all(
    BUILTIN_TOOLS.map(async (tool) => {
      const result = await findCommand(tool.commandName);
      return {
        id: tool.id,
        name: tool.displayName,
        status: result.source,
        path: result.path,
      };
    }),
  );
  return results;
}
