// SPEC #1924 T-614 — Project / Global Logs scope facet contract.
//
// Keep scope selection orthogonal to the existing severity, query, and
// process facets. The live-event isolation regression is covered separately
// in logs-process-facet.test.mjs; this file owns the selector, request, and
// Project-vs-Global semantics.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const boardLogsSurfaceSource = readFileSync(
  resolve(here, "../board-logs-surface.js"),
  "utf8",
);
const appCssSource = readFileSync(
  resolve(here, "../styles/app.css"),
  "utf8",
);

async function importBoardLogsSurface() {
  const source = boardLogsSurfaceSource.replace(
    'from "/board-surface.js"',
    `from "${pathToFileURL(resolve(here, "../board-surface.js")).href}"`,
  );
  return import(
    `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`
  );
}

function createNode(document, tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function createFixture(createBoardLogsSurface) {
  const { document, window } = parseHTML(`
    <html>
      <body>
        <div class="workspace-window">
          <div class="window-body"></div>
        </div>
      </body>
    </html>
  `);
  // linkedom exposes HTMLSelectElement.value as getter-only, while the real
  // browser API is writable and renderLogs synchronizes select state through
  // assignments. Restore that browser contract for this behavioral fixture.
  const selectPrototype = Object.getPrototypeOf(document.createElement("select"));
  const selectValue = Object.getOwnPropertyDescriptor(selectPrototype, "value");
  Object.defineProperty(selectPrototype, "value", {
    configurable: true,
    get() {
      return this.__testValue ?? selectValue?.get?.call(this) ?? "";
    },
    set(value) {
      this.__testValue = String(value);
    },
  });
  const windowElement = document.querySelector(".workspace-window");
  const body = windowElement.querySelector(".window-body");
  const sent = [];
  let activeProjectScope = "project-alpha";
  const surface = createBoardLogsSurface({
    send(message) {
      sent.push(message);
    },
    createNode: (tag, className, text) =>
      createNode(document, tag, className, text),
    createKnowledgeMarkdownBody() {
      return document.createElement("div");
    },
    windowMap: new Map([["logs-1", windowElement]]),
    focusWindowLocally() {},
    pushAlertToast() {},
    sendWindowFocus() {},
    focusOrSpawnPreset() {},
    activeWorkspace: () => ({ windows: [] }),
    activeProjectTab: () => ({ project_scope: activeProjectScope }),
    visibleBounds: () => ({ x: 0, y: 0, width: 100, height: 100 }),
    getActiveWorkProjection: () => null,
  });
  return {
    body,
    sent,
    surface,
    window,
    setActiveProjectScope(value) {
      activeProjectScope = value;
    },
  };
}

test("Logs scope selector exposes Project and Global and sends its state in load requests", async () => {
  const { createBoardLogsSurface } = await importBoardLogsSurface();
  const { body, sent, surface, window } = createFixture(createBoardLogsSurface);

  surface.mountLogsWindow({ id: "logs-1", preset: "logs" }, body);

  const selector = body.querySelector(".logs-scope-select");
  assert.ok(selector, "Logs filter chrome must expose a scope selector");
  assert.ok(
    selector.closest(".logs-filter-field"),
    "scope must reuse the existing Logs filter-field chrome",
  );
  assert.equal(
    selector.closest(".logs-filter-field").querySelector("span")?.textContent,
    "Scope",
  );
  assert.deepEqual(
    Array.from(selector.querySelectorAll("option")).map((option) => [
      option.value,
      option.textContent,
    ]),
    [
      ["project", "Project"],
      ["global", "Global"],
    ],
  );

  const state = surface.ensureLogState("logs-1");
  assert.equal(state.scope, "project", "Project is the safe default scope");
  assert.equal(sent[0]?.kind, "load_logs");
  assert.equal(sent[0]?.id, "logs-1");
  assert.equal(sent[0]?.scope, "project");

  surface.applyBoardLogsReceiveEvent({
    kind: "log_entries",
    id: "logs-1",
    entries: [],
  });
  selector.value = "global";
  selector.dispatchEvent(new window.Event("change", { bubbles: true }));

  assert.equal(state.scope, "global");
  assert.equal(sent.at(-1)?.kind, "load_logs");
  assert.equal(sent.at(-1)?.id, "logs-1");
  assert.equal(sent.at(-1)?.scope, "global");
});

test("Project and Global modes accept mutually exclusive live log scopes", async () => {
  const { createBoardLogsSurface } = await importBoardLogsSurface();
  const { surface } = createFixture(createBoardLogsSurface);
  const projectState = surface.ensureLogState("logs-project");
  const globalState = surface.ensureLogState("logs-global");
  projectState.scope = "project";
  globalState.scope = "global";

  const projectEntry = {
    id: "project-entry",
    project_scope: "project-alpha",
    severity: "info",
    source: "gwt",
    message: "project event",
  };
  const otherProjectEntry = {
    ...projectEntry,
    id: "other-project-entry",
    project_scope: "project-beta",
  };
  const globalEntry = {
    id: "global-entry",
    severity: "info",
    source: "gwtd",
    message: "machine event",
  };

  for (const entry of [projectEntry, otherProjectEntry, globalEntry]) {
    surface.applyBoardLogsReceiveEvent({ kind: "log_entry_appended", entry });
  }

  assert.deepEqual(
    projectState.entries,
    [projectEntry],
    "Project mode accepts only its matching project_scope",
  );
  assert.deepEqual(
    globalState.entries,
    [globalEntry],
    "Global mode accepts only entries without project_scope",
  );
});

test("a Project reload refreshes the window scope after project migration", async () => {
  const { createBoardLogsSurface } = await importBoardLogsSurface();
  const { body, sent, surface, setActiveProjectScope } = createFixture(
    createBoardLogsSurface,
  );
  surface.mountLogsWindow({ id: "logs-1", preset: "logs" }, body);
  const state = surface.ensureLogState("logs-1");
  assert.equal(state.projectScope, "project-alpha");

  setActiveProjectScope("project-migrated");
  surface.requestLogs("logs-1");

  assert.equal(state.projectScope, "project-migrated");
  assert.equal(sent.at(-1)?.scope, "project");
});

test("scope composes with severity, query, and process filters", async () => {
  const { createBoardLogsSurface } = await importBoardLogsSurface();
  const { body, surface } = createFixture(createBoardLogsSurface);
  surface.mountLogsWindow({ id: "logs-1", preset: "logs" }, body);
  const state = surface.ensureLogState("logs-1");
  state.scope = "project";
  state.severity = "warn";
  state.query = "needle";
  state.processKind = "git";

  const entry = (id, overrides = {}) => ({
    id,
    project_scope: "project-alpha",
    severity: "warn",
    source: "gwt",
    message: "needle",
    fields: { kind: "git" },
    ...overrides,
  });
  surface.applyBoardLogsReceiveEvent({
    kind: "log_entries",
    id: "logs-1",
    entries: [
      entry("matching-all-facets"),
      entry("wrong-scope", { project_scope: "project-beta" }),
      entry("below-severity", { severity: "info" }),
      entry("wrong-query", { message: "haystack" }),
      entry("wrong-process", { fields: { kind: "docker" } }),
      entry("unscoped-machine", { project_scope: undefined }),
    ],
  });

  const visibleRows = Array.from(body.querySelectorAll(".logs-entry"));
  assert.equal(visibleRows.length, 1);
  assert.match(visibleRows[0].textContent, /needle/);
});

test("scope selector reuses Logs chrome backed by Operator tokens", () => {
  const selectRule = appCssSource.match(
    /\.logs-filter-field select,\s*\.logs-filter-field input\s*\{([^}]*)\}/,
  )?.[1];
  assert.ok(selectRule, "existing Logs select/input chrome must remain defined");
  assert.match(selectRule, /border-radius:\s*var\(--radius-md\)/);
  assert.match(selectRule, /border:\s*1px solid var\(--color-border-strong\)/);
  assert.match(selectRule, /background:\s*var\(--color-surface\)/);
  assert.match(selectRule, /color:\s*var\(--color-text\)/);
  assert.doesNotMatch(
    appCssSource,
    /\.logs-scope-select\s*\{/,
    "scope selector must not fork a custom chrome instead of reusing Logs/Operator tokens",
  );
});
