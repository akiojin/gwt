/**
 * Error Utilities
 *
 * 統一されたエラーハンドリングユーティリティ。
 * カスタムエラークラスの型判定と分類を提供します。
 */

import { GitError } from "../git.js";
import { WorktreeError } from "../worktree.js";
import { ClaudeError } from "../claude.js";
import { CodexError } from "../codex.js";
import { GeminiError } from "../gemini.js";
import { DependencyInstallError } from "../services/dependency-installer.js";

/**
 * エラー名のリスト定義
 */
const GIT_RELATED_ERROR_NAMES = ["GitError", "WorktreeError"] as const;
const RECOVERABLE_ERROR_NAMES = [
  "GitError",
  "WorktreeError",
  "CodexError",
  "GeminiError",
  "DependencyInstallError",
] as const;
const CODING_AGENT_ERROR_NAMES = [
  "ClaudeError",
  "CodexError",
  "GeminiError",
] as const;

/**
 * 型ガード: エラー名で判定
 */
function getErrorName(error: unknown): string | undefined {
  if (!error) return undefined;

  if (error instanceof Error) {
    return error.name;
  }

  if (
    typeof error === "object" &&
    "name" in (error as Record<string, unknown>)
  ) {
    return (error as { name?: string }).name;
  }

  return undefined;
}

/**
 * 型ガード: 指定のエラークラスまたはエラー名に該当するか判定
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type ErrorConstructor = new (...args: any[]) => Error;

function isErrorOfType(
  error: unknown,
  constructors: ErrorConstructor[],
  names: readonly string[],
): boolean {
  if (!error) return false;

  // instanceof チェック
  for (const ctor of constructors) {
    if (error instanceof ctor) return true;
  }

  // 名前チェック（異なるモジュールからのインスタンス用）
  const errorName = getErrorName(error);
  if (errorName && names.includes(errorName)) {
    return true;
  }

  return false;
}

/**
 * Git関連エラー（GitError, WorktreeError）かどうかを判定
 */
export function isGitRelatedError(error: unknown): boolean {
  return isErrorOfType(
    error,
    [GitError, WorktreeError],
    GIT_RELATED_ERROR_NAMES,
  );
}

/**
 * 回復可能なエラー（リトライ/続行可能）かどうかを判定
 */
export function isRecoverableError(error: unknown): boolean {
  return isErrorOfType(
    error,
    [GitError, WorktreeError, CodexError, GeminiError, DependencyInstallError],
    RECOVERABLE_ERROR_NAMES,
  );
}

/**
 * コーディングエージェント関連エラー（Claude, Codex, Gemini）かどうかを判定
 */
export function isCodingAgentError(error: unknown): boolean {
  return isErrorOfType(
    error,
    [ClaudeError, CodexError, GeminiError],
    CODING_AGENT_ERROR_NAMES,
  );
}

/**
 * エラーメッセージを安全に取得
 */
export function getErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return String(error);
}

/**
 * エラーの根本原因を取得
 */
export function getErrorCause(error: unknown): unknown {
  if (error instanceof Error && "cause" in error) {
    return error.cause;
  }
  return undefined;
}
