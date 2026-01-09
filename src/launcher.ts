/**
 * コーディングエージェント起動機能
 *
 * コーディングエージェントの起動処理を管理します。
 * 3つの実行タイプ（path, bunx, command）をサポートします。
 */

import { execa } from "execa";
import chalk from "chalk";
import type { CodingAgent, CodingAgentLaunchOptions } from "./types/tools.js";
import { createLogger } from "./logging/logger.js";
import {
  parsePackageCommand,
  resolveVersionSuffix,
} from "./utils/npmRegistry.js";
import { writeTerminalLine } from "./utils/terminal.js";

const logger = createLogger({ category: "launcher" });

/**
 * コマンド名をPATH環境変数から解決
 *
 * Unix/Linuxではwhichコマンド、Windowsではwhereコマンドを使用して、
 * コマンド名を絶対パスに解決します。
 *
 * @param commandName - 解決するコマンド名
 * @returns コマンドの絶対パス
 * @throws コマンドが見つからない場合
 */
export async function resolveCommand(commandName: string): Promise<string> {
  const whichCommand = process.platform === "win32" ? "where" : "which";

  try {
    const result = await execa(whichCommand, [commandName]);

    // where（Windows）は複数行返す可能性があるため、最初の行のみ取得
    const resolvedPath = (result.stdout.split("\n")[0] ?? "").trim();

    if (!resolvedPath) {
      logger.error({ commandName }, "Command not found in PATH");
      throw new Error(
        `Command "${commandName}" not found in PATH.\n` +
          `Please ensure the command is installed and available in your PATH environment variable.`,
      );
    }

    logger.debug({ commandName, resolvedPath }, "Command resolved");
    return resolvedPath;
  } catch (error) {
    // which/whereコマンド自体が失敗した場合
    if (error instanceof Error) {
      logger.error(
        { commandName, error: error.message },
        "Command resolution failed",
      );
      throw new Error(
        `Failed to resolve command "${commandName}".\n` +
          `Error: ${error.message}\n` +
          `Please ensure the command is installed and available in your PATH environment variable.`,
      );
    }
    throw error;
  }
}

/**
 * 引数配列を構築
 *
 * defaultArgs + modeArgs[mode] + extraArgs の順で引数を結合します。
 * 未定義のフィールドは空配列として扱います。
 *
 * @param agent - コーディングエージェント定義
 * @param options - 起動オプション
 * @returns 結合された引数配列
 */
function buildArgs(
  agent: CodingAgent,
  options: CodingAgentLaunchOptions,
): string[] {
  const args: string[] = [];

  // 1. defaultArgs
  if (agent.defaultArgs) {
    args.push(...agent.defaultArgs);
  }

  // 2. modeArgs[mode]
  const mode = options.mode || "normal";
  const modeArgs = agent.modeArgs[mode];
  if (modeArgs) {
    args.push(...modeArgs);
  }

  // 3. extraArgs
  if (options.extraArgs) {
    args.push(...options.extraArgs);
  }

  logger.debug(
    {
      agentId: agent.id,
      mode: options.mode ?? "normal",
      argsCount: args.length,
    },
    "Args built",
  );
  return args;
}

/**
 * コーディングエージェントを起動
 *
 * エージェントの実行タイプ（path/bunx/command）に応じて適切な方法で起動します。
 * stdio: "inherit" で起動するため、エージェントの入出力は親プロセスに継承されます。
 *
 * @param agent - コーディングエージェント定義
 * @param options - 起動オプション
 * @throws 起動に失敗した場合
 */
export async function launchCodingAgent(
  agent: CodingAgent,
  options: CodingAgentLaunchOptions = {},
): Promise<void> {
  const args = buildArgs(agent, options);

  const env = {
    ...process.env,
    ...(options.sharedEnv ?? {}),
    ...(agent.env ?? {}),
  };

  // execa共通オプション（cwdがundefinedの場合は含めない）
  const execaOptions = {
    stdio: "inherit" as const,
    ...(options.cwd ? { cwd: options.cwd } : {}),
    env,
  };

  logger.info(
    {
      agentId: agent.id,
      agentType: agent.type,
      command: agent.command,
      mode: options.mode ?? "normal",
    },
    "Launching coding agent",
  );

  switch (agent.type) {
    case "path": {
      // 絶対パスで直接実行
      await execa(agent.command, args, execaOptions);
      logger.info({ agentId: agent.id }, "Coding agent completed (path)");
      break;
    }

    case "bunx": {
      // bunx経由でパッケージ実行
      // バージョン指定がある場合はパッケージ名に付与
      const { packageName, version: embeddedVersion } = parsePackageCommand(
        agent.command,
      );
      const selectedVersion = options.version ?? embeddedVersion ?? "latest";
      const versionSuffix = resolveVersionSuffix(selectedVersion);
      const packageWithVersion = `${packageName}${versionSuffix}`;

      // FR-072: Log version information
      if (selectedVersion === "installed") {
        writeTerminalLine(chalk.green(`   📦 Version: installed`));
      } else {
        writeTerminalLine(chalk.green(`   📦 Version: @${selectedVersion}`));
      }
      writeTerminalLine(chalk.cyan(`   🔄 Using bunx ${packageWithVersion}`));

      // bunx [package@version] [args...]
      await execa("bunx", [packageWithVersion, ...args], execaOptions);
      logger.info(
        { agentId: agent.id, version: selectedVersion },
        "Coding agent completed (bunx)",
      );
      break;
    }

    case "command": {
      // PATH解決 → 実行
      const resolvedPath = await resolveCommand(agent.command);
      await execa(resolvedPath, args, execaOptions);
      logger.info({ agentId: agent.id }, "Coding agent completed (command)");
      break;
    }

    default: {
      // TypeScriptの型チェックで到達不可能だが、実行時の安全性のため
      const exhaustiveCheck: never = agent.type;
      throw new Error(
        `Unknown agent execution type: ${exhaustiveCheck as string}`,
      );
    }
  }
}
