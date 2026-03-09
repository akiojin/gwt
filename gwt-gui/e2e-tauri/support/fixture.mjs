import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

function runChecked(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    stdio: "pipe",
    ...options,
  });
  if (result.status === 0) {
    return result;
  }

  const stdout = result.stdout?.trim() ?? "";
  const stderr = result.stderr?.trim() ?? "";
  throw new Error(
    [
      `Command failed: ${command} ${args.join(" ")}`,
      stdout ? `stdout:\n${stdout}` : "",
      stderr ? `stderr:\n${stderr}` : "",
    ]
      .filter(Boolean)
      .join("\n\n"),
  );
}

function escapeTomlString(value) {
  return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

function writeRecentProjects(homeDir, projectRoot) {
  const gwtDir = path.join(homeDir, ".gwt");
  mkdirSync(gwtDir, { recursive: true });
  const content = [
    "[[projects]]",
    `path = "${escapeTomlString(projectRoot)}"`,
    "last_opened = 2026-03-09T00:00:00Z",
    "",
  ].join("\n");
  writeFileSync(path.join(gwtDir, "recent-projects.toml"), content, "utf8");
}

function writeProjectConfig(projectRoot) {
  const gwtDir = path.join(projectRoot, ".gwt");
  mkdirSync(gwtDir, { recursive: true });
  writeFileSync(
    path.join(gwtDir, "project.toml"),
    'bare_repo_name = "repo.git"\n',
    "utf8",
  );
}

function createBareProject(projectRoot) {
  const bareRepo = path.join(projectRoot, "repo.git");
  const worktree = path.join(path.dirname(projectRoot), "worktree");

  mkdirSync(projectRoot, { recursive: true });
  runChecked("git", ["init", "--bare", bareRepo]);
  runChecked("git", ["clone", bareRepo, worktree]);
  runChecked("git", ["-C", worktree, "config", "user.name", "gwt e2e"]);
  runChecked("git", ["-C", worktree, "config", "user.email", "gwt-e2e@example.com"]);

  runChecked("git", ["-C", worktree, "checkout", "-b", "main"]);
  writeFileSync(path.join(worktree, "README.md"), "# gwt tauri e2e\n", "utf8");
  runChecked("git", ["-C", worktree, "add", "README.md"]);
  runChecked("git", ["-C", worktree, "commit", "-m", "init"]);
  runChecked("git", ["-C", worktree, "push", "origin", "main"]);
  runChecked("git", ["--git-dir", bareRepo, "symbolic-ref", "HEAD", "refs/heads/main"]);

  runChecked("git", ["-C", worktree, "checkout", "-b", "feature/tauri-e2e"]);
  writeFileSync(path.join(worktree, "feature.txt"), "tauri e2e\n", "utf8");
  runChecked("git", ["-C", worktree, "add", "feature.txt"]);
  runChecked("git", ["-C", worktree, "commit", "-m", "feature"]);
  runChecked("git", ["-C", worktree, "push", "origin", "feature/tauri-e2e"]);

  writeProjectConfig(projectRoot);
}

function writeFakeCodex(binDir) {
  const agentScriptPath = path.join(binDir, "fake-codex-agent.mjs");
  const cmdScriptPath = path.join(binDir, "codex.cmd");

  writeFileSync(
    agentScriptPath,
    `const args = process.argv.slice(2);
if (args.includes("--version")) {
  process.stdout.write("codex-cli 0.99.0\\n");
  process.exit(0);
}

const cmdline = (process.env.GWT_E2E_CMDLINE ?? "").toLowerCase();
const wrapper =
  cmdline.includes("chcp 65001 > nul") && cmdline.includes("codex")
    ? "cmd"
    : "other";

process.stdout.write("LONG-LINE-THAT-WILL-BE-CLEARED-BY-ANSI\\r");
setTimeout(() => {
  process.stdout.write("\\u001b[2K\\rWRAPPER:" + wrapper + "\\r\\n");
  process.stdout.write("E2E-AGENT-READY>\\r\\n");
}, 50);

process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  const text = chunk.replace(/\\r?\\n/g, "").trim();
  if (!text) return;
  process.stdout.write("ECHO:" + text + "\\r\\n");
  if (text === "exit") {
    process.exit(0);
  }
});

setInterval(() => {}, 1000);
`,
    "utf8",
  );

  writeFileSync(
    cmdScriptPath,
    `@echo off
setlocal
set "GWT_E2E_CMDLINE=%CMDCMDLINE%"
node "%~dp0fake-codex-agent.mjs" %*
`,
    "utf8",
  );
}

export function createWindowsTauriFixture() {
  const rootDir = mkdtempSync(path.join(os.tmpdir(), "gwt-tauri-e2e-"));
  const homeDir = path.join(rootDir, "home");
  const binDir = path.join(rootDir, "bin");
  const projectRoot = path.join(rootDir, "fixture-project");

  mkdirSync(homeDir, { recursive: true });
  mkdirSync(binDir, { recursive: true });

  createBareProject(projectRoot);
  writeRecentProjects(homeDir, projectRoot);
  writeFakeCodex(binDir);

  return {
    rootDir,
    homeDir,
    binDir,
    projectRoot,
    cleanup() {
      rmSync(rootDir, { force: true, recursive: true });
    },
  };
}
