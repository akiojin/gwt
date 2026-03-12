#!/usr/bin/env node
// Claude Code PreToolUse Hook: Block cd command outside worktree

import { execSync } from "node:child_process";
import path from "node:path";

function block(reason, stopReason) {
  process.stdout.write(JSON.stringify({ decision: "block", reason, stopReason }));
  process.exit(2);
}

function getWorktreeRoot() {
  try {
    return execSync("git rev-parse --show-toplevel", {
      encoding: "utf8",
      stdio: ["pipe", "pipe", "ignore"],
    }).trim();
  } catch {
    return process.cwd();
  }
}

function isWithinWorktree(targetPath, worktreeRoot) {
  if (!targetPath || targetPath === "~") return false;

  let absPath;
  if (path.isAbsolute(targetPath)) {
    absPath = targetPath;
  } else {
    absPath = path.resolve(targetPath);
  }
  absPath = path.resolve(absPath);

  const normalizedRoot = path.resolve(worktreeRoot);
  const normalizedTarget = absPath;

  return (
    normalizedTarget === normalizedRoot ||
    normalizedTarget.startsWith(normalizedRoot + path.sep)
  );
}

function splitCommandSegments(command) {
  // Split on &&, ||, ;, |, |&, & while handling simple cases
  return command
    .replace(/\|&/g, "\n")
    .replace(/\|\|/g, "\n")
    .replace(/&&/g, "\n")
    .replace(/[;|&]/g, "\n")
    .split("\n")
    .map((s) => s.replace(/[<>].*/g, "").replace(/<<.*/g, "").trim())
    .filter(Boolean);
}

const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const input = JSON.parse(Buffer.concat(chunks).toString());

const toolName = input.tool_name;
if (toolName !== "Bash") process.exit(0);

const command = input.tool_input?.command ?? "";
const worktreeRoot = getWorktreeRoot();
const segments = splitCommandSegments(command);

for (const segment of segments) {
  const cdMatch = segment.match(/^(?:builtin\s+)?(?:command\s+)?cd\b\s*(.*)/);
  if (!cdMatch) continue;

  const targetPath = cdMatch[1].trim().split(/\s+/)[0] || "";
  if (!isWithinWorktree(targetPath, worktreeRoot)) {
    block(
      "\u{1F6AB} cd command outside worktree is not allowed",
      `Worktree is designed to complete work within the launched directory. Directory navigation outside the worktree using cd command cannot be executed.\n\nWorktree root: ${worktreeRoot}\nTarget path: ${targetPath}\nBlocked command: ${command}\n\nInstead, use absolute paths to execute commands, e.g., 'git -C /path/to/repo status' or '/path/to/script.sh'`
    );
  }
}

process.exit(0);
