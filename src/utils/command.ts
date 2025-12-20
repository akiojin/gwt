import { execa } from "execa";

/**
 * Checks whether a command is available in the current PATH.
 *
 * Uses `where` on Windows and `which` on other platforms.
 *
 * @param commandName - Command name to look up (e.g. `claude`, `npx`, `gemini`)
 * @returns true if the command exists in PATH
 */
export async function isCommandAvailable(
  commandName: string,
): Promise<boolean> {
  try {
    const command = process.platform === "win32" ? "where" : "which";
    await execa(command, [commandName], {
      shell: true,
      stdin: "ignore",
      stdout: "ignore",
      stderr: "ignore",
    });
    return true;
  } catch {
    return false;
  }
}
