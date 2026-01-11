export interface BunReexecInput {
  hasBunGlobal: boolean;
  bunExecPath?: string | null;
  argv: string[];
  scriptPath: string;
}

export interface BunReexecCommand {
  command: string;
  args: string[];
}

export function buildBunReexecCommand(
  input: BunReexecInput,
): BunReexecCommand | null {
  if (input.hasBunGlobal) {
    return null;
  }

  const scriptPath = input.scriptPath?.trim();
  if (!scriptPath) {
    return null;
  }

  const bunCommand = input.bunExecPath?.trim() || "bun";
  const args = [scriptPath, ...(input.argv?.slice(2) ?? [])];

  return { command: bunCommand, args };
}
