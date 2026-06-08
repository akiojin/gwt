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

test("Work lists its live sessions flat, focus-led with the agent type as a tag (Option A)", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, {
    id: "proj",
    title: "Project",
    status_category: "active",
    active_works: [
      makeWork("w1", "Parser cleanup", "active", "work/feature-a", [
        makeAgent("codex", "session-a", "active", "parser refactor"),
        makeAgent("codex", "session-b", "active", "add unit tests"),
        makeAgent("claude", "session-c", "blocked", "waiting on review"),
        makeAgent("codex", "session-done", "done", "old work"),
      ]),
    ],
    unassigned_agents: [],
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const work = fixture.body.querySelector('.workspace-work-group[data-workspace-id="w1"]');
  assert.ok(work, "the Work is the spine row");
  assert.ok(
    work.querySelector('.workspace-overview-row[data-workspace-id="w1"]'),
    "the Work keeps a selectable header row",
  );

  // No agent-type grouping layer — the agent type alone does not identify work.
  assert.equal(work.querySelector(".workspace-work-agent"), null, "no agent-type grouping");

  // Only the 3 LIVE sessions are listed flat; the terminated one is summarized.
  const sessions = Array.from(
    work.querySelectorAll(".workspace-work-session[data-session-id]"),
  );
  assert.deepEqual(
    sessions.map((node) => node.dataset.sessionId),
    ["session-a", "session-b", "session-c"],
    "live sessions are listed flat, terminated ones are not",
  );

  // Each session is focus-led (work content), with status + agent-type tag.
  const first = sessions[0];
  assert.match(
    first.querySelector(".workspace-work-session-focus").textContent,
    /parser refactor/,
  );
  assert.match(first.querySelector(".workspace-work-session-agent").textContent, /Codex/);
  assert.ok(first.querySelector(".workspace-work-session-status"), "status is shown");
  assert.equal(first.dataset.status, "active");
  assert.equal(sessions[2].dataset.status, "blocked");
  assert.match(work.textContent, /1 completed session/);
});

test("A Work whose sessions are all terminated shows a completed summary, not a bare header", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, {
    id: "proj",
    title: "Project",
    status_category: "active",
    active_works: [
      makeWork("w1", "Done work", "done", "work/x", [
        makeAgent("codex", "s1", "done", "shipped"),
      ]),
    ],
    unassigned_agents: [],
  });
  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });
  const work = fixture.body.querySelector('.workspace-work-group[data-workspace-id="w1"]');
  assert.equal(
    work.querySelectorAll(".workspace-work-session[data-session-id]").length,
    0,
    "no live session rows",
  );
  assert.match(work.textContent, /1 completed session/);
});

test("Idle branches collapse below the spine; work branches and their remote counterparts are excluded", () => {
  const fixture = createFixture();
  const branches = fakeBranchesSurface([
    branchEntry("work/feature-a"),
    branchEntry("origin/work/feature-a", { scope: "remote" }),
    branchEntry("main", { is_head: true }),
    branchEntry("develop"),
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
    { branchesSurface: branches },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  assert.equal(
    fixture.body.querySelectorAll(".workspace-work-group[data-workspace-id]").length,
    1,
    "only Works are on the spine",
  );

  const idle = fixture.body.querySelector(".workspace-idle-branches");
  assert.ok(idle, "idle branches have their own collapsible section");
  const toggle = idle.querySelector("[data-action='toggle-idle-branches']");
  // feature-a (work) and origin/work/feature-a (its remote counterpart) excluded.
  assert.match(toggle.textContent, /Other branches \(idle\) \(2\)/);
  assert.equal(
    idle.querySelectorAll(".workspace-branch-row.is-idle").length,
    0,
    "collapsed by default",
  );

  toggle.click();
  const expanded = fixture.body.querySelector(".workspace-idle-branches");
  // The scope filter lives inside the idle section, not the global toolbar.
  assert.ok(
    expanded.querySelector(".branch-filter-group [data-branch-filter='local']"),
    "the Local/Remote/All filter is scoped to the idle section",
  );
  const idleNames = Array.from(
    expanded.querySelectorAll(".workspace-branch-row.is-idle[data-branch-name]"),
    (node) => node.dataset.branchName,
  ).sort();
  assert.deepEqual(idleNames, ["develop", "main"]);
  assert.equal(
    expanded.querySelector('[data-branch-name="origin/work/feature-a"]'),
    null,
    "a work branch's remote counterpart is never in the idle section",
  );
});

test("Branches load on mount; the Work toolbar has no branch-cleanup button (cleanup lives in the Branches surface)", () => {
  const fixture = createFixture();
  const requested = [];
  const branches = fakeBranchesSurface([], {
    requestBranches: (id) => requested.push(id),
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
    "branches load once on mount so the idle section can fill",
  );
  assert.equal(
    fixture.body.querySelector("[data-action='open-branch-cleanup']"),
    null,
    "branch cleanup is not in the Work surface (it belongs to the Branches surface)",
  );
});

test("Per-Work Launch and Resume reuse the existing branch protocol", () => {
  const fixture = createFixture();
  const sent = [];
  const surface = createSurface(
    fixture,
    {
      id: "proj",
      title: "Project",
      status_category: "active",
      active_works: [makeWork("w1", "Parser", "active", "work/feature-a")],
      unassigned_agents: [],
    },
    { send: (message) => sent.push(message) },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const work = fixture.body.querySelector('.workspace-work-group[data-workspace-id="w1"]');
  work.querySelector("[data-branch-row-action='launch']").click();
  work.querySelector("[data-branch-row-action='resume']").click();

  const launch = sent.find((m) => m.kind === "open_launch_wizard");
  const resume = sent.find((m) => m.kind === "resume_branch_latest_agent");
  assert.ok(launch && launch.branch_name === "work/feature-a", "launch sends open_launch_wizard");
  assert.ok(
    resume && resume.branch_name === "work/feature-a",
    "resume sends resume_branch_latest_agent",
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

function makeAgent(agentId, sessionId, status, focus) {
  const names = { codex: "Codex", claude: "Claude Code", gemini: "Gemini" };
  return {
    session_id: sessionId,
    agent_id: agentId,
    display_name: names[agentId] || agentId,
    status_category: status,
    title_summary: focus,
    current_focus: focus,
  };
}

function makeWork(id, title, status, branch, agents) {
  return {
    id,
    title,
    status_category: status,
    branch,
    agents: agents || [makeAgent("codex", `session-${id}`, status, title)],
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
