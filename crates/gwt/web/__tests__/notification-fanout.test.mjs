// SPEC #3206 v2 Phase 7 — every notification kind is recorded into the
// notification center (FR-011) while the transient alerts keep their firing
// conditions and lifetimes (Sc 3) and the controllers stay untouched (FR-016).
//
// app.js is not importable in node, so the seams are pinned as source
// contracts (the repo convention for app.js wiring); the sink behaviour
// (no id / no timeout forwarded, jump-to kept) is covered by
// notification-center.test.mjs.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { DEFAULT_COALESCE_KINDS } from "../socket-receive-dispatcher.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const boardLogsSource = readFileSync(resolve(here, "../board-logs-surface.js"), "utf8");
const completionSource = readFileSync(
  resolve(here, "../agent-completion-notifications.js"),
  "utf8",
);

// Extract the balanced `{ ... }` body of a `function name(` declaration.
function functionBody(source, name) {
  const start = source.indexOf(`function ${name}(`);
  assert.ok(start >= 0, `app.js must define function ${name}`);
  const open = source.indexOf("{", start);
  let depth = 0;
  for (let i = open; i < source.length; i += 1) {
    if (source[i] === "{") depth += 1;
    else if (source[i] === "}") {
      depth -= 1;
      if (depth === 0) return source.slice(open, i + 1);
    }
  }
  throw new Error(`unbalanced function ${name}`);
}

// Extract the balanced `case "<kind>": ... break;` block from receive().
function caseBlock(source, kind) {
  const start = source.indexOf(`case "${kind}":`);
  assert.ok(start >= 0, `app.js must handle ${kind}`);
  const end = source.indexOf("break;", start);
  return source.slice(start, end);
}

// Every notificationCenter.record({...}) call in a snippet, as raw text.
function recordCalls(snippet) {
  const calls = [];
  let from = 0;
  while (true) {
    const at = snippet.indexOf("notificationCenter.record(", from);
    if (at < 0) break;
    let depth = 0;
    for (let i = at; i < snippet.length; i += 1) {
      if (snippet[i] === "(") depth += 1;
      else if (snippet[i] === ")") {
        depth -= 1;
        if (depth === 0) {
          calls.push(snippet.slice(at, i + 1));
          from = i + 1;
          break;
        }
      }
    }
  }
  return calls;
}

// --- T-015: issue_monitor_toast fan-out (FR-011 / FR-012, Sc 2) ---

test("issue_monitor_toast records into the notification center with the Issue Monitor title and issue suffix", () => {
  const block = caseBlock(appSource, "issue_monitor_toast");
  const calls = recordCalls(block);
  assert.equal(calls.length, 1, "exactly one history record per event");
  const call = calls[0];
  assert.match(call, /kind:\s*"issue-monitor"/);
  assert.match(call, /level:\s*event\?\.level/);
  assert.match(call, /title:\s*"Issue Monitor"/, "backend sends no title; the literal is the only source");
  assert.match(call, /message:\s*event\?\.message/);
  assert.match(call, /issueNumber:\s*event\?\.issue_number/);
  assert.doesNotMatch(call, /event\?\.title/, "do not copy the dead `event?.title` read");
  // FR-011: the history record must not be gated by any display path.
  const recordAt = block.indexOf("notificationCenter.record(");
  const firstIf = block.indexOf("if (");
  assert.ok(firstIf < 0 || recordAt < firstIf, "record() runs before any display gating");
});

// --- T-017: completion / attention / board-mention seams (FR-011, Sc 3, Sc 7) ---

test("agent completion keeps its alerts lifetime + gating and additionally records history", () => {
  const body = functionBody(appSource, "showAgentCompletionToast");
  assert.match(body, /alertsToasts\.push\(/, "transient toast still renders");
  assert.match(body, /id:\s*"agent-completion"/, "alerts singleton id preserved");
  assert.match(body, /timeoutMs:\s*12_000/, "completion lifetime preserved (12s)");
  assert.match(body, /dismissible:\s*false/);
  const calls = recordCalls(body);
  assert.equal(calls.length, 1);
  assert.match(calls[0], /kind:\s*"agent-completion"/);
  assert.match(body, /onActivate/, "jump-to (project tab) carried by the shared notice");
  // Away-gating stays in the controller: app.js's renderer never checks it.
  assert.doesNotMatch(body, /isAttentionAway|visibilityState|hasFocus/);
});

test("attention keeps error=sticky / done=8s / needs_input=14s and per-window dedup, and records history", () => {
  const body = functionBody(appSource, "showAttentionToast");
  assert.match(
    body,
    /flavor === "error" \? 0 : flavor === "done" \? 8_000 : 14_000/,
    "attention lifetimes preserved",
  );
  assert.match(body, /id:\s*`attention-\$\{notice\.windowId\}`/, "per-window alerts dedup preserved");
  assert.match(body, /alertsToasts\.push\(/);
  const calls = recordCalls(body);
  assert.equal(calls.length, 1);
  assert.match(calls[0], /kind:\s*"attention"/);
  assert.match(body, /onActivate:\s*\(\)\s*=>\s*frameWindow\(notice\.windowId\)/, "jump-to window preserved");
});

test("board mention keeps its 8s alert and records history at the pushAlertToast seam", () => {
  // Controller side (board-logs-surface.js) is untouched: still fires the
  // same singleton 8s notice through the injected pushAlertToast seam.
  assert.match(boardLogsSource, /id:\s*"board-mention",\s*\n\s*level:\s*"info"/);
  assert.match(boardLogsSource, /timeoutMs:\s*8000/);
  // Display side (app.js) fans the same notice out to alerts + history.
  const seam = appSource.match(/pushAlertToast:\s*\(notice\)\s*=>\s*\{[\s\S]*?\n\s*\},/)?.[0];
  assert.ok(seam, "pushAlertToast seam is a block that fans out");
  assert.match(seam, /alertsToasts\.push\(notice\)/);
  const calls = recordCalls(seam);
  assert.equal(calls.length, 1);
  assert.match(calls[0], /kind:\s*"board-mention"/);
});

test("history records are additive: every record() passes a kind and the controllers stay untouched (FR-016)", () => {
  for (const call of recordCalls(appSource)) {
    assert.match(call, /kind:\s*"[a-z-]+"/, `record() without a kind: ${call}`);
  }
  // The controllers never learn about the center.
  assert.doesNotMatch(completionSource, /notificationCenter|notification-center/);
  assert.doesNotMatch(boardLogsSource, /notificationCenter|notification-center/);
  // The desktop-notification fan-out still goes through the same publish seam.
  assert.match(completionSource, /showToast\(notice\);\s*\n\s*if \(getDesktopNotificationPermission\(\) === "granted"\)/);
});

// --- T-019: non-regression note (FR-011) ---

test("issue_monitor_toast is never coalesced by the receive dispatcher (history must see every event)", () => {
  assert.equal(DEFAULT_COALESCE_KINDS.has("issue_monitor_toast"), false);
  assert.equal(DEFAULT_COALESCE_KINDS.has("issue_monitor_status"), true, "status stays latest-wins");
});

// --- FR-017: surface error seams into the Issue surface ---

test("app.js injects the FR-017 error seams into the Issue surface (report / resolve / open)", () => {
  const call = appSource.match(/createKnowledgeKanbanSurface\(\{[\s\S]*?\n {6}\}\);/)?.[0];
  assert.ok(call, "createKnowledgeKanbanSurface call found");
  assert.match(call, /reportSurfaceError:\s*\(error\)\s*=>\s*notificationCenter\.recordError\(error\)/);
  assert.match(call, /resolveSurfaceError:\s*\(key\)\s*=>\s*notificationCenter\.resolveError\(key\)/);
  assert.match(call, /openNotificationCenter:\s*\(\)\s*=>\s*notificationCenter\.open\(\)/);
});
