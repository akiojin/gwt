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
  assert.match(queue.textContent, /No Workspace selected/);
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

  assert.match(fixture.body.textContent, /Workspaces/);
  assert.match(fixture.body.textContent, /Unassigned Agents/);
  assert.match(
    fixture.body.querySelector(".workspace-overview-status-line").textContent,
    /2 Workspaces · 1 Unassigned Agents/,
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
  assert.match(queue.textContent, /No Workspace selected/);
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
    "Work",
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

test("Workspace detail renders Sessions under a Work, highlighting the active one (SPEC-2359)", () => {
  const projection = sampleProjection();
  // A single Work (launch) whose conversation split into two Sessions; the
  // latest is active.
  projection.works[0].agents[0].sessions = [
    { agent_session_id: "conv-aaaa1111", started_at: "2026-05-21T03:20:00Z", is_active: false },
    { agent_session_id: "conv-bbbb2222", started_at: "2026-05-21T04:00:00Z", is_active: true },
  ];
  const fixture = createFixture();
  const surface = createSurface(fixture, projection);
  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const sessions = Array.from(fixture.body.querySelectorAll(".workspace-detail-session"));
  assert.equal(sessions.length, 2, "one row per conversation Session");
  const active = sessions.filter((row) => row.dataset.active === "true");
  assert.equal(active.length, 1, "exactly one active Session");
  // The active row carries the latest conversation's (truncated) id and a clear
  // "Current" badge; past rows are badged "Past".
  assert.match(active[0].textContent, /conv-bbb/);
  const activeBadge = active[0].querySelector(".workspace-detail-session-badge");
  assert.equal(activeBadge.textContent, "Current");
  assert.equal(activeBadge.dataset.sessionState, "current");
  const pastRow = sessions.find((row) => row.dataset.active !== "true");
  const pastBadge = pastRow.querySelector(".workspace-detail-session-badge");
  assert.equal(pastBadge.textContent, "Past");
  assert.equal(pastBadge.dataset.sessionState, "past");
  // The full conversation id is available on hover even though the visible id is
  // truncated.
  assert.equal(
    active[0].querySelector(".workspace-detail-session-id").title,
    "conv-bbbb2222",
  );
  // Each Work renders exactly one Agent header (the agent/tool name), always
  // shown, so two Sessions of one Work never look like two Agents. The Session
  // rows are labelled "Session ...", not with the agent name.
  const headings = fixture.body.querySelectorAll(".workspace-detail-work-heading");
  assert.equal(headings.length, 1, "one Agent header per Work");
  assert.match(headings[0].textContent, /Codex/);
  assert.match(sessions[0].textContent, /Session/);
  // Persistent data renders, never the stale "No assigned agents" placeholder.
  assert.doesNotMatch(fixture.body.textContent, /No assigned agents/);
});

test("Workspace detail shows a Work heading per launch when a Workspace has multiple Works", () => {
  const projection = sampleProjection();
  projection.works[0].agents = [
    {
      session_id: "launch-1",
      agent_id: "codex",
      display_name: "Codex",
      sessions: [
        { agent_session_id: "conv-1", started_at: "2026-05-21T03:20:00Z", is_active: true },
      ],
    },
    {
      session_id: "launch-2",
      agent_id: "claude-code",
      display_name: "Claude Code",
      sessions: [
        { agent_session_id: "conv-2", started_at: "2026-05-21T05:00:00Z", is_active: false },
      ],
    },
  ];
  const fixture = createFixture();
  const surface = createSurface(fixture, projection);
  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const headings = Array.from(
    fixture.body.querySelectorAll(".workspace-detail-work-heading"),
    (node) => node.textContent,
  );
  assert.deepEqual(headings, ["Codex", "Claude Code"]);
  assert.equal(fixture.body.querySelectorAll(".workspace-detail-session").length, 2);
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

test("Per-Work Resume resumes that Work's own session directly (SPEC-2359)", () => {
  const projection = sampleProjection();
  // A Paused (resumable) Work — the active/running Work has nothing to resume.
  projection.works[0].agents[0].status_category = "idle";
  projection.works[0].agents[0].session_id = "work-launch-1";
  const fixture = createFixture();
  const sent = [];
  const surface = createSurface(fixture, projection, {
    send: (message) => sent.push(message),
    getResumeBounds: () => ({ x: 0, y: 0, width: 800, height: 600 }),
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  // Resume lives on the list elements, not on the Workspace header. A Work with
  // no recorded conversation still exposes Resume on its placeholder row.
  assert.equal(
    fixture.body.querySelector("[data-action='resume-workspace']"),
    null,
    "Workspace header no longer carries a Resume button",
  );
  const resume = fixture.body.querySelector("[data-action='resume-work']");
  assert.ok(resume, "a session-less Work still exposes a Resume action");
  assert.equal(resume.dataset.sessionId, "work-launch-1");
  resume.click();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, "resume_workspace_agent");
  assert.equal(sent[0].session_id, "work-launch-1");
  assert.ok(sent[0].bounds, "resume carries viewport bounds for the new window");
});

test("Each Session row carries its own Resume that resumes that conversation (SPEC-2359)", () => {
  const projection = sampleProjection();
  // A Paused (resumable) Work whose conversation split into two Sessions. Each
  // Session row is a list element, so each gets its own Resume control.
  projection.works[0].agents[0].status_category = "idle";
  projection.works[0].agents[0].session_id = "work-launch-1";
  projection.works[0].agents[0].sessions = [
    { agent_session_id: "conv-older1111", started_at: "2026-05-21T03:20:00Z", is_active: false },
    { agent_session_id: "conv-latest2222", started_at: "2026-05-21T04:00:00Z", is_active: true },
  ];
  const fixture = createFixture();
  const sent = [];
  const surface = createSurface(fixture, projection, {
    send: (message) => sent.push(message),
    getResumeBounds: () => ({ x: 0, y: 0, width: 800, height: 600 }),
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  // One Resume per Session row (list element), not one per Work and not on the
  // Workspace header.
  assert.equal(fixture.body.querySelector("[data-action='resume-workspace']"), null);
  assert.equal(fixture.body.querySelector("[data-action='resume-work']"), null);
  const resumes = Array.from(fixture.body.querySelectorAll("[data-action='resume-session']"));
  assert.equal(resumes.length, 2, "one Resume per conversation Session");
  // Every Resume targets the same Work (gwt session id) but a distinct
  // conversation (agent_session_id).
  assert.deepEqual(
    resumes.map((node) => node.dataset.sessionId),
    ["work-launch-1", "work-launch-1"],
  );
  assert.deepEqual(
    resumes.map((node) => node.dataset.agentSessionId),
    ["conv-older1111", "conv-latest2222"],
  );

  // Resuming the older Session resumes that exact conversation, not the latest.
  resumes[0].click();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, "resume_workspace_agent");
  assert.equal(sent[0].session_id, "work-launch-1");
  assert.equal(sent[0].agent_session_id, "conv-older1111");
  assert.ok(sent[0].bounds, "resume carries viewport bounds for the new window");
});

test("Non-resumable Sessions are history-only; a Start Fresh control keeps the Work launchable (SPEC-2359)", () => {
  const projection = sampleProjection();
  // A Paused Work whose only conversations cannot be resumed here (e.g. pruned
  // or placeholder handles). Each Session row must render without a Resume, and
  // the Work must still expose a way to launch a fresh conversation.
  projection.works[0].agents[0].status_category = "idle";
  projection.works[0].agents[0].session_id = "work-launch-1";
  projection.works[0].agents[0].sessions = [
    { agent_session_id: "conv-old", started_at: "2026-05-21T03:20:00Z", is_active: false, resumable: false },
    { agent_session_id: "conv-new", started_at: "2026-05-21T04:00:00Z", is_active: true, resumable: false },
  ];
  const fixture = createFixture();
  const sent = [];
  const surface = createSurface(fixture, projection, {
    send: (message) => sent.push(message),
    getResumeBounds: () => ({ x: 0, y: 0, width: 800, height: 600 }),
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  // Both Session rows render (history is visible) but carry no Resume control.
  assert.equal(fixture.body.querySelectorAll(".workspace-detail-session").length, 2);
  assert.equal(fixture.body.querySelector("[data-action='resume-session']"), null);

  // A single Start Fresh fallback launches a new conversation on the Work.
  const fresh = fixture.body.querySelector(".workspace-detail-session-fresh [data-action='resume-work']");
  assert.ok(fresh, "Start Fresh control appears when no Session is resumable");
  assert.equal(fresh.textContent, "Start Fresh");
  assert.equal(fresh.dataset.sessionId, "work-launch-1");
  fresh.click();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, "resume_workspace_agent");
  assert.equal(sent[0].session_id, "work-launch-1");
  // Start Fresh carries no specific conversation → backend resolves latest/fresh.
  assert.equal(sent[0].agent_session_id, undefined);
});

test("Workspace surface is a single fused view with no Work/Git Branches tab toggle (SPEC-2359)", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, sampleProjection());

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  // The Work / Git Branches tab toggle and the separate branches section are gone.
  assert.equal(
    fixture.body.querySelectorAll("[data-work-tab]").length,
    0,
    "the Work/Git Branches tab toggle must be removed",
  );
  assert.equal(fixture.body.querySelector(".workspace-tab-group"), null);
  assert.equal(fixture.body.querySelector("[data-work-section='branches']"), null);
  assert.equal(fixture.body.querySelector(".workspace-branches-shell"), null);

  // The single fused surface keeps the Workspace List + Detail.
  assert.ok(fixture.body.querySelector(".workspace-overview-root"));
  assert.ok(fixture.body.querySelector(".workspace-overview-list-pane"));
  assert.ok(fixture.body.querySelector(".workspace-overview-detail-pane"));
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

test("Work surface renders a lifecycle_state badge on each Work row (SPEC-2359 W-12 FR-351)", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, {
    id: "lifecycle-projection",
    title: "Lifecycle projection",
    status_category: "active",
    active_work_count: 2,
    active_works: [
      {
        id: "work-active",
        title: "Active Work",
        status_category: "active",
        lifecycle_state: "active",
        agents: [],
      },
      {
        id: "work-paused",
        title: "Paused Work",
        status_category: "idle",
        lifecycle_state: "paused",
        agents: [],
      },
    ],
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const activeRow = fixture.body.querySelector(
    '.workspace-overview-row[data-workspace-id="work-active"]',
  );
  const pausedRow = fixture.body.querySelector(
    '.workspace-overview-row[data-workspace-id="work-paused"]',
  );
  const activeBadge = activeRow.querySelector(".workspace-overview-lifecycle");
  const pausedBadge = pausedRow.querySelector(".workspace-overview-lifecycle");
  assert.ok(activeBadge, "expected each Work row to render a lifecycle_state badge");
  assert.equal(activeBadge.textContent, "Active");
  assert.equal(activeBadge.dataset.lifecycle, "active");
  assert.equal(pausedBadge.textContent, "Paused");
  assert.equal(pausedBadge.dataset.lifecycle, "paused");
});

test("Work surface Done action sends close_work with close_kind done (SPEC-2359 W-12 FR-351)", () => {
  const fixture = createFixture();
  const sent = [];
  const surface = createSurface(fixture, {
    id: "lifecycle-projection",
    title: "Lifecycle projection",
    status_category: "active",
    active_work_count: 1,
    active_works: [
      {
        id: "work-active",
        title: "Active Work",
        status_category: "active",
        lifecycle_state: "active",
        agents: [],
      },
    ],
  }, { send: (message) => sent.push(message) });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const doneButton = fixture.body.querySelector("[data-action='close-work-done']");
  assert.ok(doneButton, "expected a Done action on the selected Work detail");
  doneButton.click();
  assert.deepEqual(sent, [
    { kind: "close_work", work_id: "work-active", close_kind: "done" },
  ]);
});

test("Work surface Discard action sends close_work with close_kind discarded (SPEC-2359 W-12 FR-351)", () => {
  const fixture = createFixture();
  const sent = [];
  const surface = createSurface(fixture, {
    id: "lifecycle-projection",
    title: "Lifecycle projection",
    status_category: "active",
    active_work_count: 1,
    active_works: [
      {
        id: "work-active",
        title: "Active Work",
        status_category: "active",
        lifecycle_state: "active",
        agents: [],
      },
    ],
  }, { send: (message) => sent.push(message) });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const discardButton = fixture.body.querySelector("[data-action='close-work-discard']");
  assert.ok(discardButton, "expected a Discard action on the selected Work detail");
  discardButton.click();
  assert.deepEqual(sent, [
    { kind: "close_work", work_id: "work-active", close_kind: "discarded" },
  ]);
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

// SPEC-2359 W-15 (FR-379 follow-up, user verification 2026-06-10): a
// Workspace whose record has no Work (e.g. a backfilled worktree) must stay
// actionable — the detail offers a Launch control that opens the launch
// wizard prefilled with the Workspace's branch (a new Work joining this
// Workspace), instead of a dead "No Work yet" placeholder.
test("sessionless Workspace offers a Launch control that opens the launch wizard for its branch", () => {
  const fixture = createFixture();
  const sent = [];
  const surface = createSurface(
    fixture,
    {
      id: "proj-1",
      title: "projection",
      status_category: "idle",
      active_work_count: 1,
      active_works: [
        {
          id: "work-work-foo-12345678",
          title: "work/foo",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/foo",
          active_agents: 0,
          blocked_agents: 0,
          agents: [],
        },
      ],
      agents: [],
    },
    { send: (message) => sent.push(message) },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const launch = fixture.body.querySelector('[data-action="launch-workspace"]');
  assert.ok(launch, "sessionless Workspace detail must offer a Launch control");
  launch.click();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, "open_active_work_launch_wizard");
  assert.equal(sent[0].branch_name, "work/foo");
});

// Layout feedback (2026-06-10 user verification): the Launch control must sit
// in the flex empty-state row (same as the Resume placeholder), not overlap
// the "No Work yet" text; and a backfilled row whose title IS the branch name
// must not repeat the same string as subtitle meta.
test("Launch control uses the flex empty-state row and duplicate branch meta is suppressed", () => {
  const fixture = createFixture();
  const surface = createSurface(
    fixture,
    {
      id: "proj-1",
      title: "projection",
      status_category: "idle",
      active_work_count: 1,
      active_works: [
        {
          id: "work-work-foo-12345678",
          title: "work/foo",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/foo",
          active_agents: 0,
          blocked_agents: 0,
          agents: [],
        },
      ],
      agents: [],
    },
    { send() {} },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const launch = fixture.body.querySelector('[data-action="launch-workspace"]');
  assert.ok(launch, "Launch control must exist");
  assert.ok(
    launch.parentElement.classList.contains("workspace-detail-session-empty"),
    "Launch must sit in the flex empty-state row so it does not overlap the text",
  );

  const row = fixture.body.querySelector(".workspace-overview-row[data-workspace-id]");
  const title = row.querySelector(".workspace-overview-row-title").textContent.trim();
  const metaTexts = Array.from(
    row.querySelectorAll(".workspace-overview-row-meta span"),
  ).map((el) => el.textContent.trim());
  assert.equal(title, "work/foo");
  assert.ok(
    !metaTexts.includes("work/foo"),
    `branch meta must be suppressed when identical to the title: ${JSON.stringify(metaTexts)}`,
  );
});

// SPEC-2359 W-15 (user design decision 2026-06-10): the Workspace list is a
// branch list — the row title and the detail heading show the branch (the
// place), while the record's own title (work summary) moves to the meta line
// and the detail subtitle. Work / Session contents live inside the detail.
test("Workspace rows and detail are titled by branch with the record title as meta", () => {
  const fixture = createFixture();
  const surface = createSurface(
    fixture,
    {
      id: "proj-1",
      title: "projection",
      status_category: "idle",
      active_work_count: 1,
      active_works: [
        {
          id: "work-develop-7ea5aa57",
          title: "gwt-manage-pr",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "develop",
          owner: "SPEC-2359",
          active_agents: 0,
          blocked_agents: 0,
          agents: [],
        },
      ],
      agents: [],
    },
    { send() {} },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const row = fixture.body.querySelector(".workspace-overview-row[data-workspace-id]");
  assert.equal(
    row.querySelector(".workspace-overview-row-title").textContent.trim(),
    "develop",
    "row title is the branch (Workspace = place)",
  );
  const metaTexts = Array.from(
    row.querySelectorAll(".workspace-overview-row-meta span"),
  ).map((el) => el.textContent.trim());
  assert.ok(
    metaTexts.includes("gwt-manage-pr"),
    `record title moves to the meta line: ${JSON.stringify(metaTexts)}`,
  );

  const heading = fixture.body.querySelector(".workspace-detail-title");
  assert.equal(heading.textContent.trim(), "develop", "detail heading is the branch");
  const subtitleText = fixture.body
    .querySelector(".workspace-detail-subtitle")
    .textContent;
  assert.match(subtitleText, /gwt-manage-pr/, "detail subtitle carries the record title");
});

// SPEC-2359 W-16 (FR-402): the agents list is capped on the wire; the detail
// Work section renders "+N more sessions" from session_agent_total so the
// user can tell more ledger sessions exist beyond the rendered ones.
test("detail shows '+N more sessions' when session_agent_total exceeds rendered agents", () => {
  const fixture = createFixture();
  const surface = createSurface(
    fixture,
    {
      id: "proj-1",
      title: "projection",
      status_category: "idle",
      active_work_count: 1,
      active_works: [
        {
          id: "work-develop-7ea5aa57",
          title: "develop",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "develop",
          active_agents: 0,
          blocked_agents: 0,
          session_agent_total: 20,
          agents: Array.from({ length: 8 }, (_, index) => ({
            session_id: `sess-${index}`,
            agent_id: "claude",
            display_name: `Claude ${index}`,
            affiliation_status: "assigned",
            status_category: "idle",
            updated_at: "2026-06-10T12:00:00Z",
            sessions: [],
          })),
        },
      ],
      agents: [],
    },
    { send() {} },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const more = fixture.body.querySelector(".workspace-detail-more-sessions");
  assert.ok(more, "expected the more-sessions label");
  assert.equal(more.textContent.trim(), "+12 more sessions");
});

// User verification (2026-06-11): a Workspace WITH existing Works must still
// offer a way to launch a NEW agent on its branch (a new Work joining the
// Workspace) — previously the Launch control only existed in the empty state.
test("Workspace with existing Works still offers a Launch control", () => {
  const fixture = createFixture();
  const sent = [];
  const surface = createSurface(
    fixture,
    {
      id: "proj-1",
      title: "projection",
      status_category: "idle",
      active_work_count: 1,
      active_works: [
        {
          id: "work-develop-7ea5aa57",
          title: "develop",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "develop",
          active_agents: 0,
          blocked_agents: 0,
          agents: [
            {
              session_id: "sess-1",
              agent_id: "claude",
              display_name: "Claude Code",
              affiliation_status: "assigned",
              status_category: "idle",
              updated_at: "2026-06-10T12:00:00Z",
              sessions: [],
            },
          ],
        },
      ],
      agents: [],
    },
    { send: (message) => sent.push(message) },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const launch = fixture.body.querySelector('[data-action="launch-workspace"]');
  assert.ok(launch, "Launch control must exist even when Works are present");
  launch.click();
  assert.equal(sent.at(-1)?.kind, "open_active_work_launch_wizard");
  assert.equal(sent.at(-1)?.branch_name, "develop");
});

// User request (2026-06-11): ArrowUp / ArrowDown switches the Workspace list
// selection from the keyboard, updating the detail pane.
test("ArrowDown / ArrowUp move the Workspace list selection", () => {
  const fixture = createFixture();
  const works = ["work/a", "work/b", "work/c"].map((branch, index) => ({
    id: `work-${index}`,
    title: branch,
    status_category: "idle",
    lifecycle_state: "paused",
    branch,
    active_agents: 0,
    blocked_agents: 0,
    agents: [],
  }));
  const surface = createSurface(
    fixture,
    {
      id: "proj-1",
      title: "projection",
      status_category: "idle",
      active_work_count: works.length,
      active_works: works,
      agents: [],
    },
    { send() {} },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const pressOnList = (key) => {
    const list = fixture.body.querySelector(".workspace-overview-list");
    const event = new fixture.document.defaultView.Event("keydown", { bubbles: true });
    event.key = key;
    list.dispatchEvent(event);
  };

  const selectedTitle = () =>
    fixture.body
      .querySelector('.workspace-overview-row[aria-selected="true"] .workspace-overview-row-title')
      ?.textContent.trim();

  assert.equal(selectedTitle(), "work/a", "first row selected by default");
  pressOnList("ArrowDown");
  assert.equal(selectedTitle(), "work/b", "ArrowDown selects the next row");
  pressOnList("ArrowDown");
  assert.equal(selectedTitle(), "work/c");
  pressOnList("ArrowDown");
  assert.equal(selectedTitle(), "work/c", "clamped at the last row");
  pressOnList("ArrowUp");
  assert.equal(selectedTitle(), "work/b", "ArrowUp selects the previous row");
  assert.match(
    fixture.body.querySelector(".workspace-detail-title").textContent,
    /work\/b/,
    "detail follows the keyboard selection",
  );
});
