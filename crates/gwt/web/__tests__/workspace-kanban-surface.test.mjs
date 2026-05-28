import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { createWorkspaceKanbanSurface } from "../workspace-kanban-surface.js";

test("Workspace Overview renders a quiet List + Detail shell instead of status Kanban columns", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, sampleProjection());

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  assert.ok(fixture.body.querySelector(".workspace-overview-root"));
  assert.ok(fixture.body.querySelector(".workspace-overview-list-pane"));
  assert.ok(fixture.body.querySelector(".workspace-overview-detail-pane"));
  assert.equal(fixture.body.querySelector(".workspace-kanban-board"), null);
  assert.equal(fixture.body.querySelector("[data-workspace-column]"), null);

  const rows = Array.from(
    fixture.body.querySelectorAll(".workspace-overview-row[data-workspace-id]"),
  );
  assert.equal(rows.length, 2);
  assert.equal(rows[0].dataset.workspaceId, "workspace-current");
  assert.equal(rows[0].getAttribute("aria-selected"), "true");
  assert.match(rows[0].textContent, /Release Notes cleanup/);
  assert.match(rows[0].textContent, /SPEC-2356/);
  assert.match(rows[0].textContent, /PR #2847/);
});

test("Workspace Overview keeps unassigned agents in an explicit queue outside Workspace rows", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, sampleProjection());

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const queue = fixture.body.querySelector(".workspace-agent-queue");
  assert.ok(queue, "unassigned agents should have a dedicated queue");
  assert.match(queue.textContent, /Unassigned Agents/);
  assert.match(queue.textContent, /No Work selected/);
  assert.match(queue.textContent, /Codex/);
  assert.equal(
    queue.querySelectorAll(".workspace-overview-agent-row").length,
    1,
  );
});

test("Workspace Overview renders Active Works from active_works and keeps Unassigned Agents separate", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, {
    id: "legacy-current",
    title: "Legacy current projection",
    status_category: "active",
    active_work_count: 2,
    active_works: [
      {
        id: "work-parser",
        title: "Parser cleanup",
        status_category: "active",
        summary: "Parser Work summary",
        owner: "SPEC-2359",
        agents: [
          {
            session_id: "agent-parser",
            display_name: "Codex",
            status_category: "active",
            title_summary: "Parser cleanup",
          },
        ],
      },
      {
        id: "work-ui",
        title: "UI polish",
        status_category: "blocked",
        blocked_agents: 1,
        agents: [
          {
            session_id: "agent-ui",
            display_name: "Claude Code",
            status_category: "blocked",
            title_summary: "UI polish",
          },
        ],
      },
    ],
    unassigned_agents: [
      {
        session_id: "agent-unassigned",
        display_name: "Codex",
        status_category: "active",
        affiliation_status: "unassigned",
      },
    ],
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  assert.match(fixture.body.textContent, /Active Works/);
  assert.match(fixture.body.textContent, /Unassigned Agents/);
  assert.match(
    fixture.body.querySelector(".workspace-overview-status-line").textContent,
    /2 Active Works · 1 Unassigned Agents/,
  );
  const rows = Array.from(
    fixture.body.querySelectorAll(".workspace-overview-row[data-workspace-id]"),
  );
  assert.deepEqual(
    rows.map((row) => row.dataset.workspaceId),
    ["work-parser", "work-ui"],
  );
  assert.match(rows[0].textContent, /Parser cleanup/);
  assert.match(rows[1].textContent, /UI polish/);
  const queue = fixture.body.querySelector(".workspace-agent-queue");
  assert.ok(queue);
  assert.equal(queue.querySelectorAll(".workspace-overview-agent-row").length, 1);
  assert.match(queue.textContent, /No Work selected/);
});

test("Workspace detail renders structured body sections without preformatted dumps", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, sampleProjection());

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const detail = fixture.body.querySelector(".workspace-overview-detail-pane");
  assert.ok(detail);
  assert.equal(detail.querySelector("pre"), null);

  const sectionTitles = Array.from(
    detail.querySelectorAll(".workspace-detail-section-title"),
    (node) => node.textContent,
  );
  assert.deepEqual(sectionTitles, [
    "Summary",
    "Agents",
    "Lifecycle",
    "Work Context",
    "Coordination",
  ]);

  const text = detail.textContent.replace(/\s+/g, " ").trim();
  assert.match(text, /Quiet Work UI redesign/);
  assert.match(text, /Mona Sans body copy/);
  assert.match(text, /work\/20260521-0234/);
  assert.match(text, /repo\/work\/20260521-0234/);
  assert.match(text, /board-claim-1/);
});

test("Workspace list selection updates the detail pane", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, sampleProjection());

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const second = fixture.body.querySelector(
    '.workspace-overview-row[data-workspace-id="workspace-done"]',
  );
  second.click();

  const selected = fixture.body.querySelector(
    '.workspace-overview-row[data-workspace-id="workspace-done"]',
  );
  assert.equal(selected.getAttribute("aria-selected"), "true");
  const detailText = fixture.body
    .querySelector(".workspace-overview-detail-pane")
    .textContent.replace(/\s+/g, " ");
  assert.match(detailText, /Completed Workspace/);
  assert.match(detailText, /Already merged/);
});

test("Workspace resume action asks backend for resumable agents", () => {
  const fixture = createFixture();
  const sent = [];
  const surface = createSurface(fixture, sampleProjection(), {
    send: (message) => sent.push(message),
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const resume = fixture.body.querySelector("[data-action='resume-workspace']");
  assert.ok(resume, "selected workspace should expose a resume action");
  resume.click();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, "list_resumable_agents");
  assert.equal(sent[0].workspace_id, "workspace-current");
});

test("Tab switcher remains visible after switching to Git Branches", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, sampleProjection());

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const tabs = fixture.body.querySelectorAll("[data-work-tab]");
  assert.equal(tabs.length, 2, "should have Work and Git Branches tabs");

  const branchTab = fixture.body.querySelector("[data-work-tab='branches']");
  branchTab.click();

  const workSection = fixture.body.querySelector("[data-work-section='work']");
  const branchSection = fixture.body.querySelector("[data-work-section='branches']");
  assert.equal(workSection.hidden, true, "work section should be hidden");
  assert.equal(branchSection.hidden, false, "branches section should be visible");

  const tabGroupAfter = fixture.body.querySelector(".workspace-tab-group");
  assert.ok(tabGroupAfter, "tab group should still exist in DOM");
  assert.equal(tabGroupAfter.hidden, false, "tab group should not be hidden");

  const workTabAfter = fixture.body.querySelector("[data-work-tab='work']");
  assert.ok(workTabAfter, "Work tab should remain accessible");
  assert.equal(workTabAfter.classList.contains("is-active"), false);
  assert.equal(branchTab.classList.contains("is-active"), true);

  workTabAfter.click();
  assert.equal(workSection.hidden, false, "work section should reappear");
  assert.equal(branchSection.hidden, true, "branches section should hide");
  assert.equal(workTabAfter.classList.contains("is-active"), true);
});

test("Workspace refresh action rerenders locally without inventing a protocol event", () => {
  const fixture = createFixture();
  const sent = [];
  const surface = createSurface(fixture, sampleProjection(), {
    send: (message) => sent.push(message),
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const refresh = fixture.body.querySelector("[data-action='refresh-workspace-overview']");
  assert.ok(refresh);
  refresh.click();
  assert.deepEqual(sent, []);
  assert.ok(fixture.body.querySelector(".workspace-overview-detail-pane"));
});

test("Workspace renderWindows refreshes legacy workspace preset windows", () => {
  const fixture = createFixture();
  let projection = null;
  const surface = createSurface(fixture, projection, {
    getActiveWorkProjection: () => projection,
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });
  assert.equal(
    fixture.body.querySelectorAll(".workspace-overview-row[data-workspace-id]").length,
    0,
  );

  projection = sampleProjection();
  surface.renderWindows();

  const rows = Array.from(
    fixture.body.querySelectorAll(".workspace-overview-row[data-workspace-id]"),
  );
  assert.deepEqual(
    rows.map((row) => row.dataset.workspaceId),
    ["workspace-current", "workspace-done"],
  );
});

function sampleProjection() {
  return {
    id: "workspace-current",
    title: "Quiet Work UI redesign",
    status_category: "active",
    status_text: "Current work is active",
    summary: "Mona Sans body copy should carry the work summary.",
    owner: "SPEC-2356",
    branch: "work/20260521-0234",
    worktree_path: "/repo/work/20260521-0234",
    pr_number: 2847,
    pr_url: "https://github.com/akiojin/gwt/pull/2847",
    pr_state: "open",
    board_refs: ["board-claim-1"],
    lifecycle_stage: "active",
    agents: [
      {
        session_id: "agent-current",
        display_name: "Codex",
        status_category: "active",
        title_summary: "Workspace list detail",
        current_focus: "Implement list detail shell",
      },
    ],
    events: [
      {
        id: "evt-start",
        kind: "start",
        title: "Start Workspace",
        summary: "Started Quiet Work UI implementation.",
        updated_at: "2026-05-21T03:20:00Z",
        board_entry_id: "board-claim-1",
      },
    ],
    works: [
      {
        id: "workspace-current",
        title: "Release Notes cleanup",
        intent: "Quiet Work UI redesign",
        summary: "Mona Sans body copy should carry the work summary.",
        owner: "SPEC-2356",
        status_category: "active",
        lifecycle_stage: "active",
        branch: "work/20260521-0234",
        worktree_path: "/repo/work/20260521-0234",
        pr_number: 2847,
        pr_url: "https://github.com/akiojin/gwt/pull/2847",
        pr_state: "open",
        board_refs: ["board-claim-1"],
        agents: [
          {
            session_id: "agent-current",
            display_name: "Codex",
            status_category: "active",
            title_summary: "Workspace list detail",
            current_focus: "Implement list detail shell",
          },
        ],
        events: [
          {
            id: "evt-start",
            kind: "start",
            title: "Start Workspace",
            summary: "Started Quiet Work UI implementation.",
            updated_at: "2026-05-21T03:20:00Z",
            board_entry_id: "board-claim-1",
          },
        ],
      },
      {
        id: "workspace-done",
        title: "Completed Workspace",
        summary: "Already merged.",
        owner: "Issue #2780",
        status_category: "done",
        lifecycle_stage: "done",
        agents: [],
        events: [],
      },
    ],
    unassigned_agents: [
      {
        session_id: "session-unassigned",
        display_name: "Codex",
        status_category: "active",
        affiliation_status: "unassigned",
        branch: "work/20260511-0100",
      },
    ],
  };
}

function createFixture() {
  const { document } = parseHTML(`
    <div id="workspace-window">
      <div class="window-body"></div>
    </div>
  `);
  const windowElement = document.getElementById("workspace-window");
  const body = windowElement.querySelector(".window-body");
  const windowData = { id: "workspace-1", preset: "workspace" };
  return {
    document,
    body,
    windowData,
    windowMap: new Map([[windowData.id, windowElement]]),
  };
}

function createSurface(fixture, projection, overrides = {}) {
  const workspace = {
    title: "gwt",
    windows: [fixture.windowData],
  };
  return createWorkspaceKanbanSurface({
    activeWorkspace: () => workspace,
    agentStatusLabel: (status) => String(status || "unknown"),
    appendMeta(container, value) {
      if (!value) return;
      container.appendChild(createNode(fixture.document, "span", "", value));
    },
    createWorkspacePrMeta: (entry) => {
      if (!entry?.pr_number) return null;
      const node = createNode(fixture.document, "span", "workspace-pr-meta");
      node.textContent = `PR #${entry.pr_number}`;
      return node;
    },
    createNode: (tag, className, text) =>
      createNode(fixture.document, tag, className, text),
    getActiveWorkProjection: () => projection,
    openWorkspaceCleanup() {},
    send() {},
    windowMap: fixture.windowMap,
    workspaceWindowById: (windowId) =>
      workspace.windows.find((window) => window.id === windowId) || null,
    ...overrides,
  });
}

function createNode(document, tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
