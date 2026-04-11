#!/usr/bin/env node
// Codex Hook: Forward event payload to gwt hook handler.
// Best-effort only: this script never blocks Codex execution.

import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

const event = process.argv[2];
if (!event) process.exit(0);

const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const payload = Buffer.concat(chunks).toString();

function hookStatus(eventName) {
  switch (eventName) {
    case "SessionStart":
    case "Stop":
      return "WaitingInput";
    case "UserPromptSubmit":
    case "PreToolUse":
    case "PostToolUse":
      return "Running";
    default:
      return null;
  }
}

function writeRuntimeState() {
  const runtimePath = process.env.GWT_SESSION_RUNTIME_PATH;
  const sessionId = process.env.GWT_SESSION_ID;
  const status = hookStatus(event);
  if (!runtimePath || !sessionId || !status) return false;

  const now = new Date().toISOString();
  const runtime = {
    status,
    updated_at: now,
    last_activity_at: now,
    source_event: event,
  };

  try {
    mkdirSync(dirname(runtimePath), { recursive: true });
    writeFileSync(runtimePath, `${JSON.stringify(runtime, null, 2)}\n`);
    return true;
  } catch {
    return false;
  }
}

writeRuntimeState();

function runHook(executable) {
  if (!executable) return false;
  try {
    const child = spawn(executable, ["hook", event], {
      stdio: ["pipe", "ignore", "ignore"],
    });
    child.stdin.write(payload);
    child.stdin.end();
    child.unref();
    return true;
  } catch {
    return false;
  }
}

function which(name) {
  try {
    const cmd = process.platform === "win32" ? "where" : "which";
    return execFileSync(cmd, [name], { encoding: "utf8", stdio: ["pipe", "pipe", "ignore"] }).trim().split(/\r?\n/)[0];
  } catch {
    return null;
  }
}

// Optional explicit override.
if (process.env.GWT_HOOK_EXECUTABLE) {
  if (runHook(process.env.GWT_HOOK_EXECUTABLE)) process.exit(0);
}

// PATH candidates.
for (const name of ["gwt-tauri", "gwt"]) {
  const resolved = which(name);
  if (resolved && runHook(resolved)) process.exit(0);
}

// Common app-install locations.
const home = process.env.HOME || process.env.USERPROFILE || "";
const candidates = [];

if (process.platform === "darwin") {
  candidates.push(
    `${home}/Applications/gwt.app/Contents/MacOS/gwt-tauri`,
    "/Applications/gwt.app/Contents/MacOS/gwt-tauri",
  );
} else if (process.platform === "win32") {
  const localAppData = process.env.LOCALAPPDATA || "";
  const programFiles = process.env.PROGRAMFILES || "";
  if (localAppData) candidates.push(`${localAppData}/Programs/gwt/gwt.exe`);
  if (programFiles) candidates.push(`${programFiles}/gwt/gwt.exe`);
}

for (const candidate of candidates) {
  if (existsSync(candidate) && runHook(candidate)) process.exit(0);
}

process.exit(0);
