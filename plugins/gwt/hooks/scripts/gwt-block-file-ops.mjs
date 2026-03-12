#!/usr/bin/env node
// Claude Code PreToolUse Hook: Block file operations outside worktree

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
  return (
    absPath === normalizedRoot ||
    absPath.startsWith(normalizedRoot + path.sep)
  );
}

function splitCommandSegments(command) {
  return command
    .replace(/\|&/g, "\n")
    .replace(/\|\|/g, "\n")
    .replace(/&&/g, "\n")
    .replace(/[;|&]/g, "\n")
    .split("\n")
    .map((s) => s.replace(/[<>].*/g, "").replace(/<<.*/g, "").trim())
    .filter(Boolean);
}

function extractFilePaths(cmd) {
  const tokens = cmd.split(/\s+/);
  // Skip first token (command name), filter out options starting with -
  return tokens.slice(1).filter((t) => !t.startsWith("-"));
}

const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const input = JSON.parse(Buffer.concat(chunks).toString());

const toolName = input.tool_name;
if (toolName !== "Bash") process.exit(0);

const command = input.tool_input?.command ?? "";
const worktreeRoot = getWorktreeRoot();
const segments = splitCommandSegments(command);

const FILE_OPS_PATTERN = /^(mkdir|rmdir|rm|touch|cp|mv)\b/;

for (const segment of segments) {
  if (!FILE_OPS_PATTERN.test(segment)) continue;

  const filePaths = extractFilePaths(segment);
  for (const filePath of filePaths) {
    if (!filePath) continue;
    if (!isWithinWorktree(filePath, worktreeRoot)) {
      block(
        "\u{1F6AB} File operations outside worktree are not allowed",
        `Worktree is designed to complete work within the launched directory. File operations outside the worktree cannot be executed.\n\nWorktree root: ${worktreeRoot}\nTarget path: ${filePath}\nBlocked command: ${command}\n\nInstead, use absolute paths within worktree, e.g., 'mkdir ./new-dir' or 'rm ./file.txt'`
      );
    }
  }
}

process.exit(0);
