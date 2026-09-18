// SPEC #3885 Phase 2b (T-015 / T-016 / T-017) — the Issue window on the canvas.
//
// Windowize used to hand the canvas a bare agent terminal: a window with no
// Issue number, no state badge, and no way back to the row it came from. FR-011
// makes the Windowized window one composite piece — Issue header (one primary
// badge, "#N", the Issue title, at most two state-dependent actions) above an
// interactive terminal — and FR-012 gives it a control that folds it back into
// the list. FR-013 keeps a session with no Issue behind it a bare terminal.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const appCss = readFileSync(resolve(here, "../styles/app.css"), "utf8");
const tokensCss = readFileSync(resolve(here, "../styles/tokens.css"), "utf8");
const typographyCss = readFileSync(resolve(here, "../styles/typography.css"), "utf8");
const appJs = readFileSync(resolve(here, "../app.js"), "utf8");

async function importSurfaceModule() {
  const source = readFileSync(resolve(here, "../knowledge-kanban-surface.js"), "utf8")
    .replace(
      'from "/focus-trap.js"',
      'from "data:text/javascript,export function createFocusTrap(){return()=>{}}"',
    )
    .replace(
      'from "./launch-pending-controller.js"',
      'from "data:text/javascript,export function createLaunchOperationId(){return%20%22row-test%22}"',
    );
  return import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);
}

function knowledgeEntry(number, overrides = {}) {
  return {
    number,
    title: `Issue ${number}`,
    state: "open",
    labels: ["gwt-spec"],
    is_spec: true,
    monitor_state: null,
    queue_position: null,
    exclusion_reason: null,
    related_work_refs: [],
    ...overrides,
  };
}

function canvasAgentWindow(overrides = {}) {
  return {
    id: "win-agent-1",
    preset: "agent",
    title: "Agent win-agent-1",
    agent_id: "codex",
    status: "running",
    placement: { kind: "canvas" },
    linked_issue_number: 3885,
    ...overrides,
  };
}

function makeDocument() {
  const { document, window } = parseHTML("<!doctype html><html><head></head><body></body></html>");
  globalThis.document = document;
  globalThis.window = window;
  return document;
}

test("a canvas agent window bound to an Issue produces a header model", async () => {
  const { issueWindowHeaderModel } = await importSurfaceModule();

  const model = issueWindowHeaderModel({
    windowData: canvasAgentWindow(),
    entry: knowledgeEntry(3885, { title: "Issue window as the only work surface" }),
  });

  assert.ok(model, "a Windowized agent window has an Issue header");
  assert.equal(model.issueNumber, 3885);
  assert.equal(model.title, "Issue window as the only work surface");
  assert.equal(model.primary.label, "Running", "the badge reuses the shared state vocabulary");
  assert.ok(model.actions.length >= 1 && model.actions.length <= 2, "at most two actions");
  assert.deepEqual(
    model.actions.map((action) => action.action),
    ["return-to-list", "open-issue"],
  );
});

test("a session with no Issue behind it has no header model", async () => {
  const { issueWindowHeaderModel } = await importSurfaceModule();

  assert.equal(
    issueWindowHeaderModel({ windowData: canvasAgentWindow({ linked_issue_number: null }) }),
    null,
    "FR-013: a non-Issue session stays a bare terminal window",
  );
  assert.equal(
    issueWindowHeaderModel({ windowData: canvasAgentWindow({ linked_issue_number: undefined }) }),
    null,
  );
});

test("an Issue-bound window that is still in the list has no canvas header", async () => {
  const { issueWindowHeaderModel } = await importSurfaceModule();

  assert.equal(
    issueWindowHeaderModel({
      windowData: canvasAgentWindow({
        placement: { kind: "issue_preview", issue_window_id: "win-1", issue_number: 3885 },
      }),
      entry: knowledgeEntry(3885),
    }),
    null,
    "the header belongs to the canvas face, not to the row's status row",
  );
});

test("the header falls back to the Issue number when no entry is loaded", async () => {
  const { issueWindowHeaderModel } = await importSurfaceModule();

  const model = issueWindowHeaderModel({ windowData: canvasAgentWindow() });

  assert.ok(model, "an unloaded Issue still gets a header");
  assert.equal(model.issueNumber, 3885);
  assert.equal(model.title, "", "the title stays empty rather than inventing one");
  assert.equal(model.primary.label, "Running");
});

test("the rendered header carries one badge and at most two actions", async () => {
  const { issueWindowHeaderModel, renderIssueWindowHeader } = await importSurfaceModule();
  const document = makeDocument();
  const actions = [];

  const model = issueWindowHeaderModel({
    windowData: canvasAgentWindow(),
    entry: knowledgeEntry(3885, { title: "Issue window as the only work surface" }),
  });
  const header = renderIssueWindowHeader(document, model, (action) => actions.push(action));

  assert.equal(header.className, "issue-window-header");
  assert.equal(header.getAttribute("data-issue-number"), "3885");
  assert.equal(
    header.querySelectorAll(".issue-window-header-badge").length,
    1,
    "exactly one primary badge",
  );
  assert.equal(
    header.querySelector(".issue-window-header-number").textContent,
    "#3885",
  );
  assert.equal(
    header.querySelector(".issue-window-header-title").textContent,
    "Issue window as the only work surface",
  );
  const buttons = [...header.querySelectorAll("button[data-action]")];
  assert.ok(buttons.length <= 2, "at most two visible actions");
  assert.ok(
    header.querySelectorAll(".terminal-root").length === 0,
    "the header never owns the terminal element",
  );
  for (const button of buttons) {
    assert.ok(button.getAttribute("aria-label"), "every action names its Issue");
  }

  buttons[0].click();
  assert.deepEqual(actions, ["return-to-list"], "the return control reports its action");
});

test("app.js mounts the header above an interactive terminal and folds it back", () => {
  // The wiring lives in app.js, which no test can import; assert on its source the
  // way the sibling Issue-preview contract tests do. `ok` rather than `match` keeps
  // a failure from dumping the whole module into the report.
  assert.ok(
    /issueWindowHeaderModel/.test(appJs),
    "the canvas terminal body asks the Issue surface for its header",
  );
  assert.ok(
    /renderIssueWindowHeader/.test(appJs),
    "the canvas terminal body renders the shared header",
  );
  assert.ok(
    /kind:\s*"dock_agent_window_to_issue"/.test(appJs),
    "FR-012: returning to the list is the inverse of the undock transition",
  );
  assert.ok(
    !/createRuntime\(\s*windowData\.id,\s*terminalRoot,\s*\{\s*readOnly:\s*true/.test(appJs),
    "FR-003: the canvas terminal stays interactive",
  );
});

test("Issue window header CSS uses Operator tokens only", () => {
  const selectors = [
    ".issue-window-header",
    ".issue-window-header-main",
    ".issue-window-header-number",
    ".issue-window-header-title",
    ".issue-window-header-badge",
    ".issue-window-header-actions",
  ];
  const defined = new Set();
  for (const source of [tokensCss, typographyCss]) {
    for (const match of source.matchAll(/(--[a-z0-9-]+)\s*:/g)) {
      defined.add(match[1]);
    }
  }
  for (const selector of selectors) {
    const blocks = blocksFor(appCss, selector);
    assert.ok(blocks.length > 0, `${selector} is styled in app.css`);
    for (const block of blocks) {
      assert.doesNotMatch(block, /#[0-9a-fA-F]{3,8}\b/, `${selector}: no raw hex colors`);
      assert.doesNotMatch(block, /\brgba?\(/, `${selector}: no raw rgb colors`);
      for (const match of block.matchAll(/var\(\s*(--[a-z0-9-]+)/g)) {
        assert.ok(defined.has(match[1]), `${selector}: token ${match[1]} is defined`);
      }
    }
  }
});

function blocksFor(css, selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = new RegExp(`(^|[\\s,}])${escaped}(?=[\\s,:[{>.])`, "g");
  const blocks = [];
  for (const match of css.matchAll(pattern)) {
    const open = css.indexOf("{", match.index);
    if (open < 0) continue;
    let depth = 0;
    for (let index = open; index < css.length; index += 1) {
      if (css[index] === "{") depth += 1;
      if (css[index] === "}") {
        depth -= 1;
        if (depth === 0) {
          blocks.push(css.slice(match.index, index + 1));
          break;
        }
      }
    }
  }
  return blocks;
}
