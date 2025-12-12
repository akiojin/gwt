/**
 * カスタムツール起動機能
 *
 * カスタムAIツールの起動処理を管理します。
 * 3つの実行タイプ（path, bunx, command）をサポートします。
 */

import { execa } from "execa";
import type { CustomAITool, LaunchOptions } from "./types/tools.js";

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
      throw new Error(
        `Command "${commandName}" not found in PATH.\n` +
          `Please ensure the command is installed and available in your PATH environment variable.`,
      );
    }

    return resolvedPath;
  } catch (error) {
    // which/whereコマンド自体が失敗した場合
    if (error instanceof Error) {
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
 * @param tool - カスタムツール定義
 * @param options - 起動オプション
 * @returns 結合された引数配列
 */
function buildArgs(tool: CustomAITool, options: LaunchOptions): string[] {
  const args: string[] = [];

  // 1. defaultArgs
  if (tool.defaultArgs) {
    args.push(...tool.defaultArgs);
  }

  // 2. modeArgs[mode]
  const mode = options.mode || "normal";
  const modeArgs = tool.modeArgs[mode];
  if (modeArgs) {
    args.push(...modeArgs);
  }

  // 3. extraArgs
  if (options.extraArgs) {
    args.push(...options.extraArgs);
  }

  return args;
}

/**
 * カスタムAIツールを起動
 *
 * ツールの実行タイプ（path/bunx/command）に応じて適切な方法で起動します。
 * stdio: "inherit" で起動するため、ツールの入出力は親プロセスに継承されます。
 *
 * @param tool - カスタムツール定義
 * @param options - 起動オプション
 * @throws 起動に失敗した場合
 */
export async function launchCustomAITool(
  tool: CustomAITool,
  options: LaunchOptions = {},
): Promise<void> {
  const args = buildArgs(tool, options);

  const env = {
    ...process.env,
    ...(options.sharedEnv ?? {}),
    ...(tool.env ?? {}),
  };

  // execa共通オプション（cwdがundefinedの場合は含めない）
  const execaOptions = {
    stdio: "inherit" as const,
    ...(options.cwd ? { cwd: options.cwd } : {}),
    env,
  };

  switch (tool.type) {
    case "path": {
      // 絶対パスで直接実行
      await execa(tool.command, args, execaOptions);
      break;
    }

    case "bunx": {
      // bunx経由でパッケージ実行
      // bunx [package] [args...]
      await execa("bunx", [tool.command, ...args], execaOptions);
      break;
    }

    case "command": {
      // PATH解決 → 実行
      const resolvedPath = await resolveCommand(tool.command);
      await execa(resolvedPath, args, execaOptions);
      break;
    }

    default: {
      // TypeScriptの型チェックで到達不可能だが、実行時の安全性のため
      const exhaustiveCheck: never = tool.type;
      throw new Error(
        `Unknown tool execution type: ${exhaustiveCheck as string}`,
      );
    }
  }
}
