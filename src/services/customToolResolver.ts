import { execa } from "execa";
import type { CustomAITool, LaunchOptions } from "../types/tools.js";

export interface CustomToolExecutionPlan {
  command: string;
  args: string[];
  env?: NodeJS.ProcessEnv;
}

const WHICH_COMMAND = process.platform === "win32" ? "where" : "which";

export async function resolveCommandPath(commandName: string): Promise<string> {
  try {
    const { stdout } = await execa(WHICH_COMMAND, [commandName]);
    const resolvedPath = (stdout.split("\n")[0] ?? "").trim();

    if (!resolvedPath) {
      throw new Error(
        `Command "${commandName}" not found in PATH.\n` +
          "Please ensure it is installed and available in your PATH.",
      );
    }

    return resolvedPath;
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    throw new Error(
      `Failed to resolve command "${commandName}".\n${reason}\n` +
        "Please ensure the command is installed and available in your PATH.",
    );
  }
}

export function buildCustomToolArgs(
  tool: CustomAITool,
  options: LaunchOptions = {},
): string[] {
  const args: string[] = [];

  if (tool.defaultArgs?.length) {
    args.push(...tool.defaultArgs);
  }

  const mode = options.mode ?? "normal";
  const modeArgs = tool.modeArgs?.[mode];
  if (modeArgs?.length) {
    args.push(...modeArgs);
  }

  if (options.skipPermissions && tool.permissionSkipArgs?.length) {
    args.push(...tool.permissionSkipArgs);
  }

  if (options.extraArgs?.length) {
    args.push(...options.extraArgs);
  }

  return args;
}

export async function prepareCustomToolExecution(
  tool: CustomAITool,
  options: LaunchOptions = {},
): Promise<CustomToolExecutionPlan> {
  const args = buildCustomToolArgs(tool, options);
  const envOverrides: NodeJS.ProcessEnv | undefined = tool.env
    ? ({ ...tool.env } as NodeJS.ProcessEnv)
    : undefined;

  switch (tool.type) {
    case "path": {
      return {
        command: tool.command,
        args,
        ...(envOverrides ? { env: envOverrides } : {}),
      };
    }
    case "bunx": {
      return {
        command: "bunx",
        args: [tool.command, ...args],
        ...(envOverrides ? { env: envOverrides } : {}),
      };
    }
    case "command": {
      const resolved = await resolveCommandPath(tool.command);
      return {
        command: resolved,
        args,
        ...(envOverrides ? { env: envOverrides } : {}),
      };
    }
    default: {
      const exhaustive: never = tool.type;
      throw new Error(`Unknown custom tool type: ${exhaustive as string}`);
    }
  }
}
