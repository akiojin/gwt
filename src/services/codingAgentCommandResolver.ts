import { execa } from "execa";
import type { CodingAgent, CodingAgentLaunchOptions } from "../types/tools.js";
import { createLogger } from "../logging/logger.js";

const logger = createLogger({ category: "agent-resolver" });

export interface CodingAgentExecutionPlan {
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

    logger.debug({ commandName, resolvedPath }, "Command path resolved");
    return resolvedPath;
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    throw new Error(
      `Failed to resolve command "${commandName}".\n${reason}\n` +
        "Please ensure the command is installed and available in your PATH.",
    );
  }
}

export function buildCodingAgentArgs(
  agent: CodingAgent,
  options: CodingAgentLaunchOptions = {},
): string[] {
  const args: string[] = [];

  if (agent.defaultArgs?.length) {
    args.push(...agent.defaultArgs);
  }

  const mode = options.mode ?? "normal";
  const modeArgs = agent.modeArgs?.[mode];
  if (modeArgs?.length) {
    args.push(...modeArgs);
  }

  if (options.skipPermissions && agent.permissionSkipArgs?.length) {
    args.push(...agent.permissionSkipArgs);
  }

  if (options.extraArgs?.length) {
    args.push(...options.extraArgs);
  }

  logger.debug(
    { agentId: agent.id, argsCount: args.length },
    "Coding agent args built",
  );
  return args;
}

export async function prepareCodingAgentExecution(
  agent: CodingAgent,
  options: CodingAgentLaunchOptions = {},
): Promise<CodingAgentExecutionPlan> {
  const baseArgs = buildCodingAgentArgs(agent, options);
  const envOverrides: NodeJS.ProcessEnv | undefined = agent.env
    ? ({ ...agent.env } as NodeJS.ProcessEnv)
    : undefined;

  let command: string;
  let args: string[];

  switch (agent.type) {
    case "path": {
      command = agent.command;
      args = baseArgs;
      break;
    }
    case "bunx": {
      command = "bunx";
      args = [agent.command, ...baseArgs];
      break;
    }
    case "command": {
      command = await resolveCommandPath(agent.command);
      args = baseArgs;
      break;
    }
    default: {
      const exhaustive: never = agent.type;
      throw new Error(`Unknown coding agent type: ${exhaustive as string}`);
    }
  }

  logger.debug(
    {
      agentId: agent.id,
      agentType: agent.type,
      command,
      hasEnv: !!envOverrides,
    },
    "Coding agent execution prepared",
  );

  return {
    command,
    args,
    ...(envOverrides ? { env: envOverrides } : {}),
  };
}
