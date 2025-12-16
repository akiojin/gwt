/**
 * Qwen CLI session parser
 *
 * Handles session detection for Qwen CLI.
 * Session files are stored in ~/.qwen/tmp/<project_hash>/
 * and checkpoints in ~/.qwen/tmp/<project_hash>/checkpoints/
 */

import path from "node:path";
import { homedir } from "node:os";

import {
  findLatestNestedSessionFile,
  readSessionIdFromFile,
} from "../common.js";

/**
 * Finds the latest Qwen session ID.
 *
 * Search order:
 * 1. ~/.qwen/tmp/<hash>/*.json or *.jsonl
 * 2. ~/.qwen/tmp/<hash>/checkpoints/*.json or *.ckpt
 *
 * Falls back to filename (without extension) if no session ID is found in content.
 */
export async function findLatestQwenSessionId(
  _cwd: string,
): Promise<string | null> {
  const baseDir = path.join(homedir(), ".qwen", "tmp");

  // Try root level first, then checkpoints subdirectory
  const latest =
    (await findLatestNestedSessionFile(
      baseDir,
      [],
      (name) => name.endsWith(".json") || name.endsWith(".jsonl"),
    )) ??
    (await findLatestNestedSessionFile(
      baseDir,
      ["checkpoints"],
      (name) => name.endsWith(".json") || name.endsWith(".ckpt"),
    ));

  if (!latest) return null;

  const fromContent = await readSessionIdFromFile(latest);
  if (fromContent) return fromContent;

  // Fallback: use filename (without extension) as tag
  return path.basename(latest).replace(/\.[^.]+$/, "");
}
