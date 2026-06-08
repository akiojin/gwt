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

test("Unified Work surface merges Work and Branches into one view without a tab toggle (SPEC-2359 W-13/US-67)", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, sampleProjection());

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  assert.equal(
    fixture.body.querySelectorAll("[data-work-tab]").length,
    0,
    "the Work/Git Branches tab toggle must be gone",
  );
  assert.equal(fixture.body.querySelector(".workspace-tab-group"), null);
  assert.equal(fixture.body.querySelector("[data-work-section='branches']"), null);
  assert.ok(fixture.body.querySelector(".workspace-overview-root"));
  assert.ok(fixture.body.querySelector(".workspace-overview-detail-pane"));
});

test("Branch backbone nests active works under their branch and keeps non-work branches (1 agent:1 Work)", () => {
  const fixture = createFixture();
  const branches = fakeBranchesSurface([
    branchEntry("work/feature-a", { is_head: true }),
    branchEntry("main"),
  ]);
  const surface = createSurface(
    fixture,
    {
      id: "proj",
      title: "Project",
      status_category: "active",
      active_works: [
        makeWork("w1", "Parser cleanup", "active", "work/feature-a"),
        makeWork("w2", "UI polish", "active", "work/feature-a"),
      ],
      unassigned_agents: [],
    },
    { branchesSurface: branches },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const featureRow = fixture.body.querySelector(
    '.workspace-branch-row[data-branch-name="work/feature-a"]',
  );
  assert.ok(featureRow, "the work branch is a row in the backbone");
  const nested = Array.from(
    featureRow.querySelectorAll(".workspace-overview-row[data-workspace-id]"),
  ).map((node) => node.dataset.workspaceId);
  assert.deepEqual(
    nested,
    ["w1", "w2"],
    "both agent-session works are nested under their branch (identity stays per agent session)",
  );

  const mainRow = fixture.body.querySelector(
    '.workspace-branch-row[data-branch-name="main"]',
  );
  assert.ok(mainRow, "a non-work branch still appears in the backbone");
  assert.equal(
    mainRow.querySelectorAll(".workspace-overview-row[data-workspace-id]").length,
    0,
    "a non-work branch has no nested works",
  );
});

test("Branch backbone requests branches on mount and routes branch cleanup to the SPEC-2009 modal", () => {
  const fixture = createFixture();
  const requested = [];
  const cleanups = [];
  const branches = fakeBranchesSurface([], {
    requestBranches: (id) => requested.push(id),
    openBranchCleanupModal: (id) => cleanups.push(id),
  });
  const surface = createSurface(
    fixture,
    {
      id: "proj",
      title: "Project",
      status_category: "active",
      active_works: [],
      unassigned_agents: [],
    },
    { branchesSurface: branches },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  assert.deepEqual(
    requested,
    [fixture.windowData.id],
    "the unified view requests branches once on mount when empty",
  );

  const cleanupBtn = fixture.body.querySelector("[data-action='open-branch-cleanup']");
  assert.ok(cleanupBtn, "branch cleanup action is present in the unified view");
  cleanupBtn.click();
  assert.deepEqual(
    cleanups,
    [fixture.windowData.id],
    "branch cleanup opens the SPEC-2009 cleanup modal (distinct from Work close)",
  );
});

test("Branch backbone Launch and Resume reuse the existing branch protocol", () => {
  const fixture = createFixture();
  const sent = [];
  const branches = fakeBranchesSurface([
    branchEntry("work/feature-a", { resume: { available: true, reason: "" } }),
  ]);
  const surface = createSurface(
    fixture,
    {
      id: "proj",
      title: "Project",
      status_category: "active",
      active_works: [makeWork("w1", "Parser", "active", "work/feature-a")],
      unassigned_agents: [],
    },
    { branchesSurface: branches, send: (message) => sent.push(message) },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const branchRow = fixture.body.querySelector(
    '.workspace-branch-row[data-branch-name="work/feature-a"]',
  );
  branchRow.querySelector("[data-branch-row-action='launch']").click();
  branchRow.querySelector("[data-branch-row-action='resume']").click();

  const launch = sent.find((m) => m.kind === "open_launch_wizard");
  const resume = sent.find((m) => m.kind === "resume_branch_latest_agent");
  assert.ok(launch, "launch sends open_launch_wizard");
  assert.equal(launch.branch_name, "work/feature-a");
  assert.ok(resume, "resume sends resume_branch_latest_agent");
  assert.equal(resume.branch_name, "work/feature-a");
});

test("Detached works without a branch stay visible in the unified view", () => {
  const fixture = createFixture();
  const branches = fakeBranchesSurface([branchEntry("main")]);
  const surface = createSurface(
    fixture,
    {
      id: "proj",
      title: "Project",
      status_category: "active",
      active_works: [makeWork("w-detached", "No branch work", "active", "")],
      unassigned_agents: [],
    },
    { branchesSurface: branches },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  assert.ok(
    fixture.body.querySelector('.workspace-overview-row[data-workspace-id="w-detached"]'),
    "a branchless work remains visible",
  );
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

function branchEntry(name, extra = {}) {
  return {
    name,
    scope: "local",
    is_head: false,
    upstream: null,
    last_commit_date: null,
    ahead: 0,
    behind: 0,
    cleanup_ready: false,
    cleanup: {},
    resume: { available: false, reason: "No resumable session" },
    ...extra,
  };
}

function makeWork(id, title, status, branch) {
  return {
    id,
    title,
    status_category: status,
    branch,
    agents: [
      {
        session_id: `agent-${id}`,
        display_name: "Agent",
        status_category: status,
        title_summary: title,
      },
    ],
  };
}

function fakeBranchesSurface(entries, overrides = {}) {
  const states = new Map();
  return {
    ensureBranchListState(windowId) {
      if (!states.has(windowId)) {
        states.set(windowId, {
          entries: entries.slice(),
          filter: "all",
          loading: false,
          error: "",
          notice: "",
          cleanupSelected: new Set(),
          selectedBranchName: "",
        });
      }
      return states.get(windowId);
    },
    requestBranches() {},
    renderBranches() {},
    openBranchCleanupModal() {},
    ...overrides,
  };
}
