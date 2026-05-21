// SPEC-2809 — DOM-level unit test for the Console window controller.
//
// Covers: 5 fixed kind tabs render, empty hints visible initially, push()
// removes the empty hint and appends a line under the right kind, ingestSnapshot
// replays historical lines without scrambling per-kind order, activate()
// switches active tab + body visibility, and invocation header is inserted on
// spawn_id change. Uses linkedom because Console window uses
// document.createElement only (no xterm/vt100 / no animation timers).
import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";

import { createConsoleWindow } from "../console-window.js";

const KINDS = ["gh", "git", "docker", "agent", "runner"];

function freshDocument() {
  const { document } = parseHTML("<!doctype html><body><div id='host'></div></body>");
  return document;
}

test("Console window renders 5 fixed kind tabs", () => {
  const document = freshDocument();
  const controller = createConsoleWindow({ document });
  const host = document.getElementById("host");
  controller.mount(host);

  const tabs = host.querySelectorAll(".console-window__tab");
  assert.equal(tabs.length, KINDS.length);
  for (let i = 0; i < KINDS.length; i++) {
    assert.equal(tabs[i].dataset.kind, KINDS[i]);
    assert.equal(tabs[i].textContent, KINDS[i]);
  }
  const panes = host.querySelectorAll(".console-window__pane");
  assert.equal(panes.length, KINDS.length);
});

test("Console window starts with empty-state hints in every pane", () => {
  const document = freshDocument();
  const controller = createConsoleWindow({ document });
  const host = document.getElementById("host");
  controller.mount(host);

  for (const kind of KINDS) {
    const pane = host.querySelector(`.console-window__pane[data-kind='${kind}']`);
    const hint = pane.querySelector(".console-window__empty");
    assert.ok(hint, `${kind} pane should have an empty hint`);
    assert.match(
      hint.textContent,
      new RegExp(`Waiting for ${kind} process output`),
      `${kind} hint text should mention the kind`,
    );
  }
});

test("Console window push() removes the empty hint and appends the line", () => {
  const document = freshDocument();
  const controller = createConsoleWindow({ document });
  const host = document.getElementById("host");
  controller.mount(host);

  controller.push({
    kind: "gh",
    spawn_id: 1,
    stream: "stdout",
    message: "PR #2810 created",
    timestamp: new Date().toISOString(),
  });

  const ghPane = host.querySelector(".console-window__pane[data-kind='gh']");
  assert.equal(
    ghPane.querySelector(".console-window__empty"),
    null,
    "empty hint should be removed once a line arrives",
  );
  const lines = ghPane.querySelectorAll(".console-window__line");
  assert.equal(lines.length, 1);
  assert.equal(lines[0].textContent, "PR #2810 created");
  // Plain messages are rendered as regular lines — the backend pushes a
  // synthetic "$ <command>" banner separately when an actual spawn starts.
  const headers = ghPane.querySelectorAll(".console-window__invocation-header");
  assert.equal(headers.length, 0);
});

test("Console window stderr line uses the dedicated stderr class", () => {
  const document = freshDocument();
  const controller = createConsoleWindow({ document });
  const host = document.getElementById("host");
  controller.mount(host);

  controller.push({
    kind: "git",
    spawn_id: 5,
    stream: "stderr",
    message: "fatal: not a repo",
    timestamp: new Date().toISOString(),
  });

  const gitPane = host.querySelector(".console-window__pane[data-kind='git']");
  const stderrLine = gitPane.querySelector(".console-window__line--stderr");
  assert.ok(stderrLine);
  assert.equal(stderrLine.textContent, "fatal: not a repo");
});

test("Console window renders backend-pushed $ command banners as invocation headers", () => {
  const document = freshDocument();
  const controller = createConsoleWindow({ document });
  const host = document.getElementById("host");
  controller.mount(host);

  controller.push({
    kind: "docker",
    spawn_id: 10,
    stream: "stdout",
    message: "$ docker pull alpine",
  });
  controller.push({
    kind: "docker",
    spawn_id: 10,
    stream: "stdout",
    message: "Pulling layer A",
  });
  controller.push({
    kind: "docker",
    spawn_id: 11,
    stream: "stdout",
    message: "$ docker pull busybox",
  });
  controller.push({
    kind: "docker",
    spawn_id: 11,
    stream: "stdout",
    message: "→ exit=0 (42ms)",
  });

  const pane = host.querySelector(".console-window__pane[data-kind='docker']");
  const headers = pane.querySelectorAll(".console-window__invocation-header");
  assert.equal(
    headers.length,
    3,
    "two `$ ...` banners + one `→ ...` footer should render as headers",
  );
  const lines = pane.querySelectorAll(".console-window__line");
  assert.equal(lines.length, 1, "only the non-banner middle line is a regular line");
});

test("Console window keeps tee-layer [target] lines as regular lines, not banners", () => {
  // Regression for the user-reported issue (2026-05-21): ConsoleTeeLayer
  // emits lines like `[gwt::index] project index status runner completed
  // (...)`. These are operational log lines, not command banners, and
  // should render with the regular line CSS so the runner tab stays
  // dense and readable instead of getting block-level margins on every
  // line.
  const document = freshDocument();
  const controller = createConsoleWindow({ document });
  const host = document.getElementById("host");
  controller.mount(host);

  controller.push({
    kind: "runner",
    spawn_id: 0,
    stream: "stdout",
    message: "[gwt::index] project index status runner ensured (project_root=/tmp ms=12)",
  });
  controller.push({
    kind: "runner",
    spawn_id: 0,
    stream: "stdout",
    message: "[gwt_core::index::runtime] refreshed worktree (elapsed_ms=4)",
  });
  controller.push({
    kind: "agent",
    spawn_id: 7,
    stream: "stdout",
    message: "[wizard] launching codex",
  });

  const runnerPane = host.querySelector(".console-window__pane[data-kind='runner']");
  assert.equal(
    runnerPane.querySelectorAll(".console-window__invocation-header").length,
    0,
    "[gwt::...] tee lines must not be treated as headers",
  );
  assert.equal(
    runnerPane.querySelectorAll(".console-window__line").length,
    2,
    "both tee lines render as regular lines",
  );

  const agentPane = host.querySelector(".console-window__pane[data-kind='agent']");
  assert.equal(
    agentPane.querySelectorAll(".console-window__invocation-header").length,
    1,
    "short single-word [stage] banners are still treated as headers",
  );
});

test("Console window ingestSnapshot replays per-kind buffers", () => {
  const document = freshDocument();
  const controller = createConsoleWindow({ document });
  const host = document.getElementById("host");
  controller.mount(host);

  controller.ingestSnapshot([
    { kind: "gh", spawn_id: 1, stream: "stdout", message: "gh A" },
    { kind: "git", spawn_id: 2, stream: "stdout", message: "git A" },
    { kind: "gh", spawn_id: 1, stream: "stderr", message: "gh B" },
    { kind: "docker", spawn_id: 3, stream: "stdout", message: "docker A" },
  ]);

  const ghLines = host
    .querySelector(".console-window__pane[data-kind='gh']")
    .querySelectorAll(".console-window__line");
  assert.equal(ghLines.length, 2);
  assert.equal(ghLines[0].textContent, "gh A");
  assert.equal(ghLines[1].textContent, "gh B");

  const gitLines = host
    .querySelector(".console-window__pane[data-kind='git']")
    .querySelectorAll(".console-window__line");
  assert.equal(gitLines.length, 1);
  assert.equal(gitLines[0].textContent, "git A");

  const dockerLines = host
    .querySelector(".console-window__pane[data-kind='docker']")
    .querySelectorAll(".console-window__line");
  assert.equal(dockerLines.length, 1);

  // Untouched kinds keep their empty hints.
  for (const kind of ["agent", "runner"]) {
    const hint = host
      .querySelector(`.console-window__pane[data-kind='${kind}']`)
      .querySelector(".console-window__empty");
    assert.ok(hint, `${kind} hint should survive when snapshot does not touch it`);
  }
});

test("Console window activate() switches active tab and pane visibility", () => {
  const document = freshDocument();
  const controller = createConsoleWindow({ document });
  const host = document.getElementById("host");
  controller.mount(host);

  controller.activate("docker");

  const root = host.querySelector(".console-window");
  assert.equal(root.dataset.activeKind, "docker");

  const tabs = host.querySelectorAll(".console-window__tab");
  for (const tab of tabs) {
    const expected = tab.dataset.kind === "docker" ? "true" : "false";
    assert.equal(tab.getAttribute("aria-selected"), expected);
  }
  const panes = host.querySelectorAll(".console-window__pane");
  for (const pane of panes) {
    if (pane.dataset.kind === "docker") {
      assert.equal(pane.hidden, false, "active pane is visible");
    } else {
      assert.equal(pane.hidden, true, `${pane.dataset.kind} pane is hidden`);
    }
  }
});

test("Console window emits load_process_console on mount when send + windowId provided", () => {
  const document = freshDocument();
  const sent = [];
  const controller = createConsoleWindow({
    document,
    send: (payload) => sent.push(payload),
    windowId: "console-1",
  });
  controller.mount(document.getElementById("host"));

  assert.equal(sent.length, 1);
  assert.deepEqual(sent[0], { kind: "load_process_console", id: "console-1" });
});

test("Console window ignores push() with unknown kind", () => {
  const document = freshDocument();
  const controller = createConsoleWindow({ document });
  controller.mount(document.getElementById("host"));

  controller.push({ kind: "unknown", spawn_id: 1, stream: "stdout", message: "x" });
  for (const kind of KINDS) {
    const lines = document
      .querySelector(`.console-window__pane[data-kind='${kind}']`)
      .querySelectorAll(".console-window__line");
    assert.equal(lines.length, 0, `${kind} pane must stay empty for unknown kind`);
  }
});

test("Console window close() removes the root from the DOM", () => {
  const document = freshDocument();
  const controller = createConsoleWindow({ document });
  const host = document.getElementById("host");
  controller.mount(host);
  assert.ok(host.querySelector(".console-window"));
  controller.close();
  assert.equal(host.querySelector(".console-window"), null);
  assert.equal(controller.isOpen(), false);
});
