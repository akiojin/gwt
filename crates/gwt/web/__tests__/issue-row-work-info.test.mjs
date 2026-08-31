// SPEC-3671 P4 — Work information and Work actions on the Issue row.
//
// FR-012: the Issue row shows the Work lifecycle, the needs-attention reason, and
// the PR number / state. The data source is the active Work projection the frontend
// already receives; the Issue row only carries the backend's Issue -> Work
// correlation (`related_work_refs`), never a second copy of the Work state.
// FR-013: Continue work / Resume / Clean Up are reachable from the Issue row.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { parseHTML } from "linkedom";

import {
  attentionForWorkspace,
  formatLifecycleStateLabel,
} from "../workspace-kanban-surface.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

async function importSurfaceModule() {
  const source = readFileSync(
    resolve(here, "../knowledge-kanban-surface.js"),
    "utf8",
  ).replace(
    'from "/focus-trap.js"',
    'from "data:text/javascript,export function createFocusTrap(){return()=>{}}"',
  ).replace(
    'from "./launch-pending-controller.js"',
    'from "data:text/javascript,export function createLaunchOperationId(){return%20%22work-row-test%22}"',
  );
  return import(
    `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`
  );
}

function knowledgeEntry(number, refs = []) {
  return {
    number,
    title: `Issue ${number}`,
    state: "open",
    meta: "",
    labels: [],
    linked_branch_count: 0,
    related_work_count: refs.length,
    related_session_count: 0,
    related_work_refs: refs,
    match_score: null,
    phase: null,
    has_unknown_phase: false,
    is_spec: false,
    monitor_state: "launched",
    queue_position: null,
    exclusion_reason: null,
  };
}

function createNode(document, tagName, className, text) {
  const node = document.createElement(tagName);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

function workRow(overrides = {}) {
  return {
    id: "work-3671",
    title: "Issue window as the primary surface",
    status_category: "active",
    status_text: "Implementing P4",
    lifecycle_state: "active",
    active_agents: 1,
    blocked_agents: 0,
    branch: "work/issue-3671",
    worktree_path: "E:/gwt/work/issue-3671",
    pr_number: 3699,
    pr_url: "https://github.com/akiojin/gwt/pull/3699",
    pr_state: "open",
    cleanup_candidate: null,
    cleanup_blocked_reason: null,
    agents: [],
    works: [],
    ...overrides,
  };
}

async function makeFixture({ projection = null } = {}) {
  const mod = await importSurfaceModule();
  const { document, window } = parseHTML(
    "<!doctype html><html><head></head><body></body></html>",
  );
  globalThis.document = document;
  globalThis.window = window;
  const body = document.createElement("div");
  document.body.appendChild(body);
  const issueWindow = { id: "win-1", preset: "issue" };
  const sent = [];
  const continued = [];
  const resumed = [];
  const cleaned = [];
  const surface = mod.createKnowledgeKanbanSurface({
    send: (message) => sent.push(message),
    sendKnowledgeSemanticSearchNow: (message) => {
      sent.push(message);
      return true;
    },
    createNode: (...args) => createNode(document, ...args),
    createKnowledgeMarkdownBody: () => document.createElement("div"),
    windowMap: new Map([[issueWindow.id, body]]),
    workspaceWindowById: (id) => (id === issueWindow.id ? issueWindow : null),
    getWorkspaceWindows: () => [issueWindow],
    pendingIndexOpenTargetsByPreset: new Map(),
    knowledgeKindForPreset: () => "issue",
    focusWindowLocally() {},
    sendWindowFocus() {},
    focusOrSpawnPreset() {},
    openIssueLaunchWizard() {},
    visibleBounds: () => ({ x: 0, y: 0, width: 100, height: 100 }),
    launchPending: {},
    createTerminalRuntime() {},
    windowDisplayTitle: (windowData) => windowData?.title || windowData?.id,
    windowRoleBadgeLabel: () => "Agent",
    windowizeIssuePreviewWindow() {},
    getActiveWorkProjection: () => projection,
    workAttentionFor: attentionForWorkspace,
    formatWorkLifecycleLabel: formatLifecycleStateLabel,
    continueWork: (workId, bounds) => {
      continued.push({ workId, bounds });
      return true;
    },
    openWorkspaceResumePicker: (workspaceId) => resumed.push(workspaceId),
    openWorkspaceCleanup: (candidate, windowId) => cleaned.push({ candidate, windowId }),
    getResumeBounds: () => ({ x: 0, y: 0, width: 1280, height: 800 }),
  });
  surface.mountKnowledgeWindow(issueWindow, body);
  const load = sent.find((message) => message.kind === "load_knowledge_bridge");
  assert.ok(load, "Issue surface requests its cache-backed rows");
  return { body, cleaned, continued, document, load, mod, resumed, sent, surface, window };
}

function applyEntries(surface, load, entries) {
  surface.applyKnowledgeReceiveEvent({
    kind: "knowledge_entries",
    id: "win-1",
    knowledge_kind: "issue",
    request_id: load.request_id,
    entries,
    selected_number: entries[0]?.number ?? null,
    empty_message: "",
    refresh_enabled: true,
  });
}

test("issueWorkRowForEntry joins the Issue row to the active Work projection", async () => {
  const { issueWorkRowForEntry } = await importSurfaceModule();
  const byId = workRow({ id: "work-3671" });
  const byBranch = workRow({ id: "work-other", branch: "refs/heads/work/issue-3672" });
  const projection = { active_works: [byId, byBranch] };

  assert.equal(
    issueWorkRowForEntry(projection, knowledgeEntry(3671, [{ id: "work-3671" }]))?.id,
    "work-3671",
    "an exact Work id match wins",
  );
  assert.equal(
    issueWorkRowForEntry(
      projection,
      knowledgeEntry(3672, [{ id: "gone", branch: "work/issue-3672" }]),
    )?.id,
    "work-other",
    "a normalized branch match is the fallback join key",
  );
  assert.equal(issueWorkRowForEntry(projection, knowledgeEntry(3673, [])), null);
  assert.equal(issueWorkRowForEntry(null, knowledgeEntry(3671, [{ id: "work-3671" }])), null);
});

// FR-012.
test("the Issue row shows Work lifecycle, attention reason, and PR state", async (t) => {
  const fixture = await makeFixture({
    projection: {
      active_works: [
        workRow({ status_category: "blocked", blocked_reason: "Waiting on review" }),
      ],
    },
  });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [
    knowledgeEntry(3671, [{ id: "work-3671", branch: "work/issue-3671" }]),
  ]);

  const row = fixture.body.querySelector('[data-issue-number="3671"]');
  const work = row.querySelector(".knowledge-row-work");
  assert.ok(work, "the row carries a Work summary block");
  assert.equal(work.querySelector(".knowledge-work-lifecycle").textContent, "Active");
  assert.equal(
    work.querySelector(".knowledge-work-lifecycle").dataset.lifecycle,
    "active",
  );
  assert.equal(work.querySelector(".knowledge-work-attention").textContent, "Waiting on review");
  const pr = work.querySelector(".knowledge-work-pr");
  assert.equal(pr.textContent, "PR #3699 · open");
  assert.equal(pr.dataset.prState, "open");
});

test("an Issue with no correlated Work renders no Work block", async (t) => {
  const fixture = await makeFixture({ projection: { active_works: [workRow()] } });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [knowledgeEntry(9999, [])]);

  const row = fixture.body.querySelector('[data-issue-number="9999"]');
  assert.equal(row.querySelector(".knowledge-row-work"), null);
});

// FR-013.
test("Continue work, Resume, and Clean Up are reachable from the Issue row", async (t) => {
  const fixture = await makeFixture({
    projection: {
      active_works: [
        workRow({
          lifecycle_state: "paused",
          status_category: "idle",
          active_agents: 0,
          cleanup_candidate: {
            branch: "work/issue-3671",
            worktree_path: "E:/gwt/work/issue-3671",
            reason: "merged",
            default_delete_remote: false,
            remote_delete_available: true,
          },
        }),
      ],
    },
  });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [
    knowledgeEntry(3671, [{ id: "work-3671", branch: "work/issue-3671" }]),
  ]);

  const row = fixture.body.querySelector('[data-issue-number="3671"]');
  row.querySelector('[data-action="continue-work"]').click();
  assert.deepEqual(fixture.continued, [
    { workId: "work-3671", bounds: { x: 0, y: 0, width: 1280, height: 800 } },
  ]);

  row.querySelector('[data-action="resume-work"]').click();
  assert.deepEqual(fixture.resumed, ["work-3671"]);

  row.querySelector('[data-action="cleanup-work"]').click();
  assert.equal(fixture.cleaned.length, 1);
  assert.equal(fixture.cleaned[0].candidate.branch, "work/issue-3671");
  assert.equal(fixture.cleaned[0].windowId, "win-1");
});

test("Clean Up stays disabled while the backend reports a cleanup blocker", async (t) => {
  const fixture = await makeFixture({
    projection: {
      active_works: [
        workRow({ cleanup_candidate: null, cleanup_blocked_reason: "live_agent" }),
      ],
    },
  });
  t.after(() => fixture.surface.clearKnowledgeBridgeState("win-1"));
  applyEntries(fixture.surface, fixture.load, [
    knowledgeEntry(3671, [{ id: "work-3671", branch: "work/issue-3671" }]),
  ]);

  const cleanup = fixture.body.querySelector('[data-action="cleanup-work"]');
  assert.equal(cleanup.disabled, true);
  cleanup.click();
  assert.deepEqual(fixture.cleaned, []);
});

test("app.js feeds the Issue surface from the shared Work projection and helpers", () => {
  assert.match(
    appSource,
    /attentionForWorkspace[\s\S]*from "\/workspace-kanban-surface\.js"/,
    "the shared Work attention rules are imported, not re-derived",
  );
  assert.match(
    appSource,
    /workAttentionFor:\s*attentionForWorkspace/,
    "the Issue surface reuses the Work surface's attention derivation",
  );
  assert.match(
    appSource,
    /formatWorkLifecycleLabel:\s*formatLifecycleStateLabel/,
    "the Issue surface reuses the Work surface's lifecycle labels",
  );
  assert.match(
    appSource,
    /getActiveWorkProjection:\s*\(\)\s*=>\s*activeWorkProjection[\s\S]*windowizeIssuePreviewWindow|windowizeIssuePreviewWindow[\s\S]*getActiveWorkProjection:\s*\(\)\s*=>\s*activeWorkProjection/,
    "the Issue surface reads the already-broadcast active Work projection",
  );
});
