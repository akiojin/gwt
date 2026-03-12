#!/usr/bin/env node
// Claude Code PreToolUse Hook: Block GIT_DIR environment variable override

function block(reason, stopReason) {
  process.stdout.write(JSON.stringify({ decision: "block", reason, stopReason }));
  process.exit(2);
}

const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const input = JSON.parse(Buffer.concat(chunks).toString());

const toolName = input.tool_name;
if (toolName !== "Bash") process.exit(0);

const command = input.tool_input?.command ?? "";

// GIT_DIR override check
if (
  /(^|[;&|]|\s)(export\s+)?GIT_DIR\s*=|env\s+[^;]*GIT_DIR\s*=|declare\s+-x\s+GIT_DIR\s*=/.test(
    command
  )
) {
  block(
    "\u{1F6AB} GIT_DIR environment variable override is not allowed",
    `Modifying GIT_DIR in a worktree environment can cause unintended repository operations.\n\nBlocked command: ${command}\n\nWorktrees have their own .git file pointing to the main repository worktree directory. Overriding GIT_DIR may break this relationship and cause git commands to operate on the wrong repository.`
  );
}

// GIT_WORK_TREE override check
if (
  /(^|[;&|]|\s)(export\s+)?GIT_WORK_TREE\s*=|env\s+[^;]*GIT_WORK_TREE\s*=|declare\s+-x\s+GIT_WORK_TREE\s*=/.test(
    command
  )
) {
  block(
    "\u{1F6AB} GIT_WORK_TREE environment variable override is not allowed",
    `Modifying GIT_WORK_TREE in a worktree environment can cause unintended repository operations.\n\nBlocked command: ${command}\n\nWorktrees have their own working directory configuration. Overriding GIT_WORK_TREE may cause git commands to operate on the wrong directory.`
  );
}

process.exit(0);
