/**
 * カスタムツール起動機能
 *
 * カスタムAIツールの起動処理を管理します。
 * 3つの実行タイプ（path, bunx, command）をサポートします。
 */

import { execa } from "execa";
import type { CustomAITool, LaunchOptions } from "./types/tools.js";
import {
  prepareCustomToolExecution,
  resolveCommandPath,
} from "./services/customToolResolver.js";

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
  const execution = await prepareCustomToolExecution(tool, options);
  const childEnv =
    execution.env && Object.keys(execution.env).length > 0
      ? { ...process.env, ...execution.env }
      : undefined;

  const env = {
    ...process.env,
    ...(options.sharedEnv ?? {}),
    ...(tool.env ?? {}),
  };

  // execa共通オプション（cwdがundefinedの場合は含めない）
  const execaOptions = {
    stdio: "inherit" as const,
    ...(options.cwd ? { cwd: options.cwd } : {}),
    ...(childEnv ? { env: childEnv } : {}),
  });
}
export { resolveCommandPath as resolveCommand };
