#!/usr/bin/env node
// Claude Code PreToolUse Hook: Block git branch operations

function block(reason, stopReason) {
  process.stdout.write(JSON.stringify({ decision: "block", reason, stopReason }));
  process.exit(2);
}

function isReadOnlyGitBranch(branchArgs) {
  const trimmed = branchArgs.trim();
  if (!trimmed) return true; // No args = list branches (allowed)

  const readOnlyFlags = /^(--list|--show-current|--all|-a|--remotes|-r|--contains|--merged|--no-merged|--points-at|--format|--sort|--abbrev|-v|-vv|--verbose)/;
  return readOnlyFlags.test(trimmed);
}

// Pre-compiled regexes for file-level checkout detection (used inside the loop)
const RE_CHECKOUT_EXPLICIT_SEP = /\bcheckout\b.*\s--\s/;
const RE_CHECKOUT_CONFLICT_FLAG = /\bcheckout\b.*\s--(theirs|ours)\b/;
const RE_CHECKOUT_BROAD_TARGET = /\s--\s+[.*]/;

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

const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const input = JSON.parse(Buffer.concat(chunks).toString());

const toolName = input.tool_name;
if (toolName !== "Bash") process.exit(0);

const command = input.tool_input?.command ?? "";
const segments = splitCommandSegments(command);

for (const segment of segments) {
  // Interactive rebase block
  if (/^git\s+rebase\b/.test(segment)) {
    if (
      /(?:^|\s)(-i|--interactive)(?:\s|$)/.test(segment) &&
      /(?:^|\s)origin\/main(?:\s|$)/.test(segment)
    ) {
      block(
        "\u{1F6AB} Interactive rebase against origin/main is not allowed",
        `Interactive rebase against origin/main initiated by LLMs is blocked because it frequently fails and disrupts sessions.\n\nBlocked command: ${command}`
      );
    }
  }

  if (!/^git\b/.test(segment)) continue;

  // checkout/switch: block branch switching but allow file-level operations
  // Allowed: git checkout --theirs/--ours -- <file>, git checkout <ref> -- <file>
  // Blocked: git checkout <branch>, git checkout -- . , git checkout -- *
  if (/\b(checkout|switch)\b/.test(segment)) {
    const hasExplicitSeparator = RE_CHECKOUT_EXPLICIT_SEP.test(segment);
    const hasConflictFlag = RE_CHECKOUT_CONFLICT_FLAG.test(segment);
    const hasBroadTarget = RE_CHECKOUT_BROAD_TARGET.test(segment);
    const isFileCheckout = (hasConflictFlag || hasExplicitSeparator) && !hasBroadTarget;
    if (!isFileCheckout) {
      block(
        "\u{1F6AB} Branch switching commands (checkout/switch) are not allowed",
        `Worktree is designed to complete work on the launched branch. Branch operations such as git checkout and git switch cannot be executed.\n\nBlocked command: ${command}`
      );
    }
  }

  // branch subcommand: only read-only allowed
  // Must match git [options...] branch pattern (not filename containing "branch")
  const branchMatch = segment.match(
    /^git\s+(?:(?:-[a-zA-Z]|--[a-z-]+)\s+)*branch\b(.*)/
  );
  if (branchMatch) {
    const branchArgs = branchMatch[1];
    if (!isReadOnlyGitBranch(branchArgs)) {
      block(
        "\u{1F6AB} Branch modification commands are not allowed",
        `Worktree is designed to complete work on the launched branch. Destructive branch operations such as git branch -d, git branch -m cannot be executed.\n\nBlocked command: ${command}`
      );
    }
    continue;
  }

  // worktree subcommand: block
  const worktreeMatch = segment.match(
    /^git\s+(?:(?:-[a-zA-Z]|--[a-z-]+)\s+)*worktree\b/
  );
  if (worktreeMatch) {
    block(
      "\u{1F6AB} Worktree commands are not allowed",
      `Worktree management operations such as git worktree add/remove cannot be executed from within a worktree.\n\nBlocked command: ${command}`
    );
  }
}

process.exit(0);
