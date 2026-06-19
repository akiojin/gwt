import { test } from "node:test";
import { createLaunchPendingController } from "../launch-pending-controller.js";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { createWorkspaceKanbanSurface } from "../workspace-kanban-surface.js";

test("Workspace Overview renders a readable Workspace list with compact filters", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, sampleProjection());

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  assert.ok(fixture.body.querySelector(".workspace-overview-root"));
  assert.ok(fixture.body.querySelector(".workspace-overview-list-pane"));
  assert.ok(fixture.body.querySelector(".workspace-overview-filter-bar"));
  assert.ok(fixture.body.querySelector(".workspace-overview-list"));
  assert.ok(fixture.body.querySelector(".workspace-overview-detail-pane"));
  assert.equal(
    fixture.body.querySelectorAll(".workspace-attention-lane[data-attention-lane]").length,
    0,
    "Workspace overview must not render mini Kanban lanes in the persistent pane",
  );
  assert.deepEqual(
    Array.from(fixture.body.querySelectorAll("[data-workspace-filter]"), (filter) => [
      filter.dataset.workspaceFilter,
      filter.querySelector(".workspace-overview-filter-label")?.textContent.trim(),
      filter.querySelector(".workspace-overview-filter-count")?.textContent.trim(),
    ]),
    [
      ["all", "All", "2"],
      ["needs_attention", "Needs Attention", "1"],
      ["running", "Running", "0"],
      ["paused", "Paused", "0"],
      ["closed", "Closed", "1"],
    ],
  );

  const rows = Array.from(
    fixture.body.querySelectorAll(".workspace-overview-row[data-workspace-id]"),
  );
  assert.equal(rows.length, 2);
  assert.equal(rows[0].dataset.workspaceId, "workspace-current");
  assert.equal(rows[0].dataset.attention, "needs_attention");
  assert.equal(rows[0].getAttribute("aria-selected"), "true");
  assert.match(rows[0].textContent, /Release Notes cleanup/);
  assert.match(rows[0].textContent, /SPEC-2356/);
  assert.match(rows[0].textContent, /Resolve blocker/);
  assert.equal(
    rows[0].querySelectorAll(".workspace-attention-chip").length,
    0,
    "row state should not duplicate lifecycle badges with attention chips",
  );
});

test("Workspace list filters explicit attention separately from PR metadata", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, {
    id: "projection",
    title: "projection",
    status_category: "active",
    active_works: [
      {
        id: "work-attention",
        title: "Broken launch",
        status_category: "active",
        lifecycle_state: "active",
        blocked_reason: "API token is missing",
        next_action: "Resolve blocker",
        active_agents: 1,
        blocked_agents: 1,
        agents: [],
      },
      {
        id: "work-pr-only",
        title: "PR metadata only",
        status_category: "active",
        lifecycle_state: "active",
        active_agents: 1,
        blocked_agents: 0,
        pr_number: 2847,
        pr_state: "OPEN",
        agents: [],
      },
      {
        id: "work-paused",
        title: "Paused branch",
        status_category: "idle",
        lifecycle_state: "paused",
        active_agents: 0,
        blocked_agents: 0,
        agents: [],
      },
      {
        id: "work-closed",
        title: "Merged branch",
        status_category: "idle",
        lifecycle_state: "active",
        done_equivalent: true,
        active_agents: 0,
        blocked_agents: 0,
        agents: [],
      },
    ],
    unassigned_agents: [],
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  assert.deepEqual(rowIdsInList(fixture), [
    "work-attention",
    "work-pr-only",
    "work-paused",
    "work-closed",
  ]);

  const attention = fixture.body.querySelector(
    '.workspace-overview-row[data-workspace-id="work-attention"]',
  );
  assert.match(attention.textContent, /API token is missing/);
  assert.match(attention.textContent, /Resolve blocker/);

  const prOnly = fixture.body.querySelector(
    '.workspace-overview-row[data-workspace-id="work-pr-only"]',
  );
  assert.match(prOnly.textContent, /PR #2847/);
  assert.doesNotMatch(prOnly.textContent, /Needs Attention/);

  fixture.body.querySelector('[data-workspace-filter="paused"]').click();
  assert.deepEqual(rowIdsInList(fixture), ["work-paused"]);
  assert.equal(
    fixture.body
      .querySelector('[data-workspace-filter="paused"]')
      .getAttribute("aria-pressed"),
    "true",
  );
});

test("Workspace list does not expose Kanban D&D lifecycle affordances", () => {
  const fixture = createFixture();
  const sent = [];
  const surface = createSurface(
    fixture,
    {
      id: "projection",
      title: "projection",
      status_category: "active",
      active_works: [
        {
          id: "work-running",
          title: "Running work",
          status_category: "active",
          lifecycle_state: "active",
          active_agents: 1,
          blocked_agents: 0,
          agents: [],
        },
        {
          id: "work-paused",
          title: "Paused work",
          status_category: "idle",
          lifecycle_state: "paused",
          active_agents: 0,
          blocked_agents: 0,
          agents: [],
        },
      ],
      unassigned_agents: [],
    },
    { send: (message) => sent.push(message) },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const runningRow = fixture.body.querySelector(
    '.workspace-overview-row[data-workspace-id="work-running"]',
  );
  assert.equal(runningRow.draggable, false);
  assert.equal(
    fixture.body.querySelectorAll(".workspace-attention-lane[data-attention-lane]").length,
    0,
  );

  assert.deepEqual(sent, [], "D&D must not send lifecycle, Board, or runtime mutations");
  assert.deepEqual(rowIdsInList(fixture), ["work-running", "work-paused"]);
});

test("Workspace list row Launch Agent opens the launch wizard without changing selection", () => {
  const fixture = createFixture();
  const sent = [];
  const surface = createSurface(
    fixture,
    {
      id: "projection",
      title: "projection",
      status_category: "active",
      active_works: [
        {
          id: "work-launch",
          title: "Launchable work",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/launchable",
          active_agents: 0,
          blocked_agents: 0,
          agents: [],
        },
      ],
      unassigned_agents: [],
    },
    { send: (message) => sent.push(message) },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const rowLaunch = fixture.body.querySelector(
    '.workspace-overview-row[data-workspace-id="work-launch"] [data-action="launch-workspace-row"]',
  );
  assert.ok(rowLaunch, "row must expose a compact Launch Agent action");
  rowLaunch.click();

  assert.deepEqual(sent, [
    {
      kind: "open_launch_wizard",
      id: fixture.windowData.id,
      branch_name: "work/launchable",
    },
  ]);
  assert.equal(
    fixture.body
      .querySelector('.workspace-overview-row[data-workspace-id="work-launch"]')
      .getAttribute("aria-selected"),
    "true",
  );
});

test("Workspace list row keyboard handler does not steal Launch Agent button keys", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, {
    id: "projection",
    title: "projection",
    status_category: "active",
    active_works: [
      {
        id: "work-selected",
        title: "Selected work",
        status_category: "active",
        lifecycle_state: "active",
        branch: "work/selected",
        active_agents: 1,
        blocked_agents: 0,
        agents: [],
      },
      {
        id: "work-launch",
        title: "Launchable work",
        status_category: "idle",
        lifecycle_state: "paused",
        branch: "work/launchable",
        active_agents: 0,
        blocked_agents: 0,
        agents: [],
      },
    ],
    unassigned_agents: [],
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const selectedRow = fixture.body.querySelector(
    '.workspace-overview-row[data-workspace-id="work-selected"]',
  );
  const launchRow = fixture.body.querySelector(
    '.workspace-overview-row[data-workspace-id="work-launch"]',
  );
  const rowLaunch = launchRow.querySelector('[data-action="launch-workspace-row"]');
  const event = new fixture.window.Event("keydown", { bubbles: true });
  event.key = "Enter";
  rowLaunch.dispatchEvent(event);

  assert.equal(selectedRow.getAttribute("aria-selected"), "true");
  assert.equal(launchRow.getAttribute("aria-selected"), "false");
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
    /2 Workspaces · 1 Needs Attention · 1 Unassigned Agents/,
  );
  const rows = Array.from(
    fixture.body.querySelectorAll(".workspace-overview-row[data-workspace-id]"),
  );
  assert.deepEqual(
    rows.map((row) => row.dataset.workspaceId),
    ["work-ui", "work-parser"],
  );
  assert.match(rows[0].textContent, /UI polish/);
  assert.match(rows[1].textContent, /Parser cleanup/);
  const queue = fixture.body.querySelector(".workspace-agent-queue");
  assert.ok(queue);
  assert.equal(queue.querySelectorAll(".workspace-overview-agent-row").length, 1);
  assert.match(queue.textContent, /No Workspace selected/);
});

test("Workspace Overview does not leak projection progress summary into other Active Works", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, {
    id: "workspace-current",
    title: "Current projection",
    status_category: "active",
    progress_summary: "Projection-only progress should stay on the current projection.",
    active_work_count: 2,
    active_works: [
      {
        id: "work-parser",
        title: "Parser cleanup",
        status_category: "active",
        progress_summary: "Parser-specific progress summary.",
        agents: [],
      },
      {
        id: "work-ui",
        title: "UI polish",
        status_category: "paused",
        agents: [],
      },
    ],
    unassigned_agents: [],
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const detail = fixture.body.querySelector(".workspace-overview-detail-pane");
  assert.ok(detail);
  let detailText = detail.textContent.replace(/\s+/g, " ");
  assert.match(detailText, /Parser-specific progress summary/);
  assert.doesNotMatch(detailText, /Projection-only progress/);

  fixture.body
    .querySelector('.workspace-overview-row[data-workspace-id="work-ui"]')
    .click();

  detailText = detail.textContent.replace(/\s+/g, " ");
  assert.match(detailText, /No progress summary yet/);
  assert.doesNotMatch(detailText, /Projection-only progress/);
  assert.doesNotMatch(detailText, /Parser-specific progress summary/);
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
  // SPEC-3075: the Work purpose is the detail heading (not a body section), so
  // the body leads with the accumulated progress summary, then separates the
  // current state, related sessions, linked work, lifecycle, and execution
  // context instead of one conflated "Summary".
  assert.deepEqual(sectionTitles, [
    "Progress Summary",
    "Current State",
    "Agents & Sessions",
    "Linked Work",
    "Lifecycle",
    "Context",
  ]);

  // The heading carries the Work purpose ("what work was running"), not the
  // branch and not the status text.
  assert.equal(
    detail.querySelector(".workspace-detail-title").textContent.trim(),
    "Release Notes cleanup",
  );

  const text = detail.textContent.replace(/\s+/g, " ").trim();
  assert.match(text, /Reworked the Workspace list into a purpose-first surface/);
  assert.match(text, /Quiet Work UI redesign/);
  assert.match(text, /Mona Sans body copy/);
  assert.match(text, /work\/20260521-0234/);
  assert.match(text, /repo\/work\/20260521-0234/);
  assert.match(text, /board-claim-1/);
});

test("Workspace detail Board refs can focus the matching Board entry", () => {
  const fixture = createFixture();
  const focused = [];
  const surface = createSurface(fixture, sampleProjection(), {
    focusBoardEntry: (entryId) => focused.push(entryId),
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const boardRef = fixture.body.querySelector(
    "[data-action='focus-board-entry'][data-board-entry-id='board-claim-1']",
  );
  assert.ok(boardRef, "Board ref chip should be clickable");
  boardRef.click();
  assert.deepEqual(focused, ["board-claim-1"]);
});

test("Workspace detail surfaces Managed Hooks health without raw JSON dumps", () => {
  const projection = sampleProjection();
  projection.works[0].managed_hook_health = {
    status: "needs_attention",
    last_event: "PreToolUse",
    last_event_at: "2026-06-17T00:00:00Z",
    pending_discussion: {
      proposal_label: "Proposal A",
      proposal_title: "Managed Hooks UX",
      next_question: "Choose the repair path",
    },
    pending_goal: {
      proposal_label: "Proposal A",
      proposal_title: "Managed Hooks UX",
      condition: "Implement hook health first",
    },
    slow_handlers: [
      {
        event: "PreToolUse",
        handler: "workflow-policy",
        status: "ok",
        duration_ms: 1250.25,
        occurred_at: "2026-06-17T00:00:01Z",
      },
    ],
    issues: ["managed hook event missing: Stop in .codex/hooks.json"],
  };
  const fixture = createFixture();
  const surface = createSurface(fixture, projection);

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const detail = fixture.body.querySelector(".workspace-overview-detail-pane");
  const hookSection = detail.querySelector('[data-section="managed-hooks"]');
  assert.ok(hookSection, "managed hook health should render in Work detail");
  assert.equal(detail.querySelector("pre"), null);
  assert.match(hookSection.textContent, /Managed Hooks/);
  assert.match(hookSection.textContent, /Needs attention/);
  assert.match(hookSection.textContent, /PreToolUse/);
  assert.match(hookSection.textContent, /Proposal A/);
  assert.match(hookSection.textContent, /workflow-policy/);
  assert.match(hookSection.textContent, /1250ms/);
  assert.match(hookSection.textContent, /Stop/);
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
  // User decision 2026-06-12: multiple Session rows per agent read as noise —
  // only the latest conversation renders.
  assert.equal(sessions.length, 1, "only the latest Session renders");
  const active = sessions.filter((row) => row.dataset.active === "true");
  assert.equal(active.length, 1, "the rendered Session is the active one");
  assert.match(active[0].textContent, /conv-bbb/);
  const activeBadge = active[0].querySelector(".workspace-detail-session-badge");
  assert.equal(activeBadge.textContent, "Current");
  assert.equal(activeBadge.dataset.sessionState, "current");
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
  // User decision 2026-06-12: only the latest Session renders, so exactly one
  // Resume targeting the latest conversation.
  assert.equal(resumes.length, 1, "one Resume for the latest Session");
  assert.equal(resumes[0].dataset.sessionId, "work-launch-1");
  assert.equal(resumes[0].dataset.agentSessionId, "conv-latest2222");

  resumes[0].click();
  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, "resume_workspace_agent");
  assert.equal(sent[0].session_id, "work-launch-1");
  assert.equal(sent[0].agent_session_id, "conv-latest2222");
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

  // Only the latest Session renders (user decision 2026-06-12) and it
  // carries no Resume control because it is not resumable.
  assert.equal(fixture.body.querySelectorAll(".workspace-detail-session").length, 1);
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

test("Merged Workspace without a cleanup candidate does not claim it is safe to delete", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, {
    id: "cleanup-projection",
    title: "Cleanup projection",
    status_category: "idle",
    active_work_count: 1,
    active_works: [
      {
        id: "work-live-cwd",
        title: "work/20260616-0203",
        status_category: "idle",
        lifecycle_state: "paused",
        merged_into_base: true,
        cleanup_candidate: null,
        branch: "work/20260616-0203",
        worktree_path: "/repo/work/20260616-0203",
        agents: [],
      },
    ],
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const row = fixture.body.querySelector(
    '.workspace-overview-row[data-workspace-id="work-live-cwd"]',
  );
  assert.ok(row, "expected the merged Workspace row to remain visible");
  assert.match(row.textContent, /Merged/, "merged badge remains visible");
  const detailText = fixture.body
    .querySelector(".workspace-overview-detail-pane")
    .textContent.replace(/\s+/g, " ");
  assert.doesNotMatch(detailText, /safe to delete/i);
  assert.equal(
    fixture.body.querySelector("[data-action='cleanup-merged-workspace']"),
    null,
  );
  assert.equal(
    fixture.body.querySelector("[data-action='cleanup-merged-workspaces']"),
    null,
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
    progress_summary:
      "Reworked the Workspace list into a purpose-first surface and split current status from cumulative progress.",
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
        progress_summary:
          "Reworked the Workspace list into a purpose-first surface and split current status from cumulative progress.",
        owner: "SPEC-2356",
        status_category: "active",
        lifecycle_stage: "active",
        next_action: "Resolve blocker",
        blocked_reason: "Resolve blocker",
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
  const { document, window } = parseHTML(`
    <div id="workspace-window">
      <div class="window-body"></div>
    </div>
  `);
  const windowElement = document.getElementById("workspace-window");
  const body = windowElement.querySelector(".window-body");
  const windowData = { id: "workspace-1", preset: "workspace" };
  return {
    document,
    window,
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

function rowIdsInList(fixture) {
  return Array.from(
    fixture.body.querySelectorAll(".workspace-overview-list .workspace-overview-row[data-workspace-id]"),
    (row) => row.dataset.workspaceId,
  );
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
  assert.equal(sent[0].kind, "open_launch_wizard");
  assert.equal(sent[0].id, fixture.windowData.id);
  assert.equal(sent[0].branch_name, "work/foo");
});

// Placement feedback (2026-06-11 user verification): the Launch Agent control
// has one canonical home — the detail header actions — never an arbitrary
// position after a variable-length Work list. A backfilled row whose title IS
// the branch name must not repeat the same string as subtitle meta.
test("Launch control lives in the detail header actions and duplicate branch meta is suppressed", () => {
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
  assert.ok(launch, "Launch Agent control must exist");
  assert.ok(
    launch.parentElement.classList.contains("workspace-detail-actions"),
    "Launch Agent belongs to the detail header actions",
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
test("Workspace rows are titled by work purpose with the branch as the sub-line (SPEC-3075)", () => {
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
          title: "Tray Copy URL polish",
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
    "Tray Copy URL polish",
    "row primary label is the work purpose (what work was running)",
  );
  const metaTexts = Array.from(
    row.querySelectorAll(".workspace-overview-row-meta span"),
  ).map((el) => el.textContent.trim());
  assert.ok(
    metaTexts.includes("develop"),
    `branch moves to the meta sub-line: ${JSON.stringify(metaTexts)}`,
  );
  assert.ok(
    metaTexts.includes("SPEC-2359"),
    `owner stays on the meta line: ${JSON.stringify(metaTexts)}`,
  );

  const heading = fixture.body.querySelector(".workspace-detail-title");
  assert.equal(
    heading.textContent.trim(),
    "Tray Copy URL polish",
    "detail heading is the work purpose (matches the rail)",
  );
  const subtitleText = fixture.body
    .querySelector(".workspace-detail-subtitle")
    .textContent;
  assert.match(subtitleText, /develop/, "detail subtitle carries the branch (the place)");
});

// SPEC-3075 FR-001 / US-1: the rail label is the Work *purpose* (identity).
// A Board-body status snapshot (the demoted `summary`) must never be promoted
// to the row label even when the Work has no recorded title — otherwise the
// list reads as a stream of status reports ("Deployed PR #3007") instead of
// "what is this Work about". The purpose falls back to the owner identity, not
// the status text.
test("Workspace rail never promotes a status snapshot to the row label (SPEC-3075 FR-001)", () => {
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
          id: "work-status-only",
          // No title (purpose) recorded — only a status snapshot in summary.
          summary: "Deployed PR #3007 to production",
          status_text: "Deployed PR #3007 to production",
          owner: "SPEC-3075",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/20260612-1405",
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
  // The status snapshot must not surface as a label anywhere on the row.
  assert.ok(
    !row.textContent.includes("Deployed PR #3007"),
    `status snapshot must not become a row label: ${row.textContent}`,
  );
  // With no recorded purpose, the owner is the purpose anchor and becomes the
  // primary label (never the status snapshot).
  assert.equal(
    row.querySelector(".workspace-overview-row-title").textContent.trim(),
    "SPEC-3075",
    "owner identity is the purpose fallback for the primary label",
  );
});

// SPEC-3075: the backend-derived work_summary (the agent-declared title-summary
// purpose, "what work was running") is the row's primary label; the branch (the
// Workspace place) is demoted to the meta sub-line.
test("Workspace rail leads with the work_summary purpose and demotes the branch (SPEC-3075)", () => {
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
          id: "work-3075",
          title: "work/20260612-1405",
          work_summary: "Work 要約を目的第一に再構成",
          owner: "SPEC-3075",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/20260612-1405",
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
    "Work 要約を目的第一に再構成",
    "primary label is the work_summary purpose",
  );
  const metaTexts = Array.from(
    row.querySelectorAll(".workspace-overview-row-meta span"),
  ).map((el) => el.textContent.trim());
  assert.ok(
    metaTexts.includes("work/20260612-1405"),
    `branch is demoted to the meta sub-line: ${JSON.stringify(metaTexts)}`,
  );
});

// SPEC-3075 FR-001 / FR-002: the detail must not render a Purpose section that
// only echoes the branch heading. A Work with no owner and whose recorded
// title is just its branch (backfilled rows) has no purpose distinct from the
// heading, so the "Purpose" section is omitted rather than repeating the branch.
test("Workspace detail omits a Purpose that only repeats the branch (SPEC-3075)", () => {
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
          id: "work-branch-only",
          title: "work/20260614-0444",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/20260614-0444",
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

  const sectionTitles = Array.from(
    fixture.body.querySelectorAll(".workspace-detail-section-title"),
    (node) => node.textContent,
  );
  assert.ok(
    !sectionTitles.includes("Purpose"),
    `Purpose section must be omitted when it only echoes the branch: ${JSON.stringify(sectionTitles)}`,
  );
});

// SPEC-3075 US-1 / US-2: a recorded title can be a gwt-* skill name (resume
// events fill it with the agent's running skill). That is not a purpose, so the
// detail Purpose and the rail/detail meta show the owner identity instead — the
// skill name never surfaces as what the Work is about.
test("Workspace purpose shows the owner, not a gwt-* skill-name title (SPEC-3075)", () => {
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
          id: "work-skill-title",
          title: "gwt-manage-pr",
          owner: "SPEC-2359",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/20260610-0120",
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

  const detail = fixture.body.querySelector(".workspace-overview-detail-pane");
  // The purpose is the detail heading; a gwt-* skill name is not a purpose, so
  // the owner identity anchors the heading instead.
  const heading = detail.querySelector(".workspace-detail-title");
  assert.equal(heading.textContent.trim(), "SPEC-2359", "purpose heading is the owner identity");
  assert.ok(
    !detail.textContent.includes("gwt-manage-pr"),
    "the gwt-* skill name must not surface anywhere in the detail",
  );
});

// SPEC-3075 US-1: backfill paths can leave the recorded title as a raw
// work-item id (e.g. "work-work-20260601-0908-9ffe416f"). An identifier is not
// a purpose, so it must never surface as the rail label or the detail purpose.
test("Workspace purpose drops a raw work-item id title (SPEC-3075)", () => {
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
          id: "work-work-20260601-0908-9ffe416f",
          title: "work-work-20260601-0908-9ffe416f",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/20260601-0908",
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
  const metaTexts = Array.from(
    row.querySelectorAll(".workspace-overview-row-meta span"),
  ).map((el) => el.textContent.trim());
  assert.ok(
    !metaTexts.some((text) => text.includes("9ffe416f")),
    `a raw work-item id must not become the rail label: ${JSON.stringify(metaTexts)}`,
  );
  const sectionTitles = Array.from(
    fixture.body.querySelectorAll(".workspace-detail-section-title"),
    (node) => node.textContent,
  );
  assert.ok(
    !sectionTitles.includes("Purpose"),
    "Purpose section is omitted when the only title is a raw work-item id",
  );
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
  assert.equal(sent.at(-1)?.kind, "open_launch_wizard");
  assert.equal(sent.at(-1)?.branch_name, "develop");

  // Placement feedback (2026-06-11): one fixed home in the header actions —
  // first action, before Done / Discard — and no floating row after the list.
  const actions = fixture.body.querySelector(".workspace-detail-actions");
  assert.ok(actions, "detail header actions container must exist");
  assert.equal(
    actions.firstElementChild?.dataset?.action,
    "launch-workspace",
    "Launch Agent is the first (primary) header action",
  );
  assert.equal(
    fixture.body.querySelectorAll('[data-action="launch-workspace"]').length,
    1,
    "exactly one Launch control — no duplicate after the Work list",
  );
  assert.equal(
    fixture.body.querySelector(".workspace-detail-launch-row"),
    null,
    "the post-list 'Start a new agent' row is gone",
  );
});

// User verification (2026-06-12) + SPEC-2359 US-78: "Safe to delete" must
// come with an actual delete action, but only when the backend supplied a
// cleanup candidate after applying the live-agent guard.
test("merged Workspace detail offers Clean Up only from a backend cleanup candidate", () => {
  const fixture = createFixture();
  const cleanupCalls = [];
  const surface = createSurface(
    fixture,
    {
      id: "proj-1",
      title: "projection",
      status_category: "idle",
      active_work_count: 1,
      active_works: [
        {
          id: "work-work-merged-12345678",
          title: "work/merged",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/merged",
          merged_into_base: true,
          cleanup_candidate: {
            branch: "work/merged",
            reason: "pr_merged",
            default_delete_remote: false,
            remote_delete_available: true,
          },
          active_agents: 0,
          blocked_agents: 0,
          agents: [],
        },
      ],
      agents: [],
    },
    {
      send() {},
      openWorkspaceCleanup: (candidate) => cleanupCalls.push(candidate),
    },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const cleanup = [...fixture.body.querySelectorAll(".workspace-detail-actions button")]
    .find((button) => button.textContent.trim() === "Clean Up");
  assert.ok(cleanup, "merged Workspace must offer a Clean Up action");
  cleanup.click();
  assert.equal(cleanupCalls.length, 1);
  assert.equal(cleanupCalls[0]?.branch, "work/merged");
});

test("merged Workspace without cleanup candidate does not offer Clean Up", () => {
  const fixture = createFixture();
  const cleanupCalls = [];
  const surface = createSurface(
    fixture,
    {
      id: "proj-1",
      title: "projection",
      status_category: "idle",
      active_work_count: 1,
      active_works: [
        {
          id: "work-work-live-12345678",
          title: "work/live",
          status_category: "active",
          lifecycle_state: "active",
          branch: "work/live",
          merged_into_base: true,
          active_agents: 1,
          blocked_agents: 0,
          agents: [
            {
              session_id: "session-live",
              display_name: "Codex",
              status_category: "active",
            },
          ],
        },
      ],
      agents: [],
    },
    {
      send() {},
      openWorkspaceCleanup: (candidate) => cleanupCalls.push(candidate),
    },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const cleanup = [...fixture.body.querySelectorAll(".workspace-detail-actions button")]
    .find((button) => button.textContent.trim() === "Clean Up");
  assert.equal(cleanup, undefined);
  assert.equal(cleanupCalls.length, 0);
});

// User verification 2026-06-12: completed (merged) local branches need a
// BULK cleanup path — the list header offers "Clean Up Merged (N)" that opens
// the cleanup flow with every merged row preselected.
test("list header offers bulk Clean Up Merged for all merged Workspaces", () => {
  const fixture = createFixture();
  const cleanupCalls = [];
  const surface = createSurface(
    fixture,
    {
      id: "proj-1",
      title: "projection",
      status_category: "idle",
      active_work_count: 3,
      active_works: [
        {
          id: "work-work-a-12345678",
          title: "work/a",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/a",
          merged_into_base: true,
          done_equivalent: true,
          cleanup_candidate: {
            branch: "work/a",
            reason: "pr_merged",
            default_delete_remote: false,
            remote_delete_available: true,
          },
          active_agents: 0,
          blocked_agents: 0,
          agents: [],
        },
        {
          id: "work-work-b-12345678",
          title: "work/b",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/b",
          merged_into_base: true,
          cleanup_candidate: {
            branch: "work/b",
            reason: "pr_merged",
            default_delete_remote: false,
            remote_delete_available: true,
          },
          active_agents: 0,
          blocked_agents: 0,
          agents: [],
        },
        {
          id: "work-work-live-12345678",
          title: "work/live",
          status_category: "active",
          lifecycle_state: "active",
          branch: "work/live",
          merged_into_base: true,
          active_agents: 1,
          blocked_agents: 0,
          agents: [
            {
              session_id: "session-live",
              display_name: "Codex",
              status_category: "active",
            },
          ],
        },
        {
          id: "work-work-open-12345678",
          title: "work/open",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/open",
          active_agents: 0,
          blocked_agents: 0,
          agents: [],
        },
      ],
      agents: [],
    },
    {
      send() {},
      openWorkspaceCleanup: (candidates) => cleanupCalls.push(candidates),
    },
  );

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const bulk = fixture.body.querySelector('[data-action="cleanup-merged-workspaces"]');
  assert.ok(bulk, "bulk Clean Up Merged control must exist");
  assert.match(bulk.textContent, /Clean Up Merged \(2\)/);
  bulk.click();
  assert.equal(cleanupCalls.length, 1);
  const branches = cleanupCalls[0].map((candidate) => candidate.branch).sort();
  assert.deepEqual(branches, ["work/a", "work/b"]);
});

// SPEC-2359 W16-4 (FR-391 / SC-262): a merged-and-stale Workspace presents
// as derived Done — badge "Done" with data-derived marking it apart from an
// explicit close — and never as Active/Paused.
test("done-equivalent Workspace presents as derived Done, not Paused", () => {
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
          id: "work-work-merged-12345678",
          title: "work/merged",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/merged",
          merged_into_base: true,
          done_equivalent: true,
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

  const badge = fixture.body.querySelector(".workspace-overview-lifecycle");
  assert.equal(badge.textContent, "Done", "derived Done presents as Done");
  assert.equal(badge.dataset.lifecycle, "done");
  assert.equal(badge.dataset.derived, "true", "distinct from an explicit close");
});

// SPEC-2359 W16-3 (FR-390): a fetched-remote-only Workspace shows a Remote
// badge; the Launch Agent header action still opens the launch wizard with
// the branch prefilled (worktree materializes on demand) and rendering the
// badge sends nothing.
test("remote-only Workspace shows the Remote badge and keeps the prefilled Launch", () => {
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
          id: "work-work-fetched-12345678",
          title: "work/fetched",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/fetched",
          remote_only: true,
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

  const badge = fixture.body.querySelector(".workspace-overview-remote");
  assert.ok(badge, "Remote badge renders for remote-only rows");
  assert.equal(badge.textContent, "Remote");
  assert.equal(sent.length, 0, "rendering generates no events (FR-381/FR-390)");

  const launch = fixture.body.querySelector('[data-action="launch-workspace"]');
  assert.ok(launch, "Launch Agent stays available for remote-only rows");
  launch.click();
  assert.equal(sent.at(-1)?.kind, "open_launch_wizard");
  assert.equal(sent.at(-1)?.branch_name, "work/fetched");
});

// Design pass (2026-06-11, frontend-design): the branch name renders with a
// dimmed namespace prefix and a strong leaf so 200+ work/* rows scan by leaf;
// the full text content stays the verbatim branch for copy / a11y.
test("row title splits the branch namespace prefix from the leaf", () => {
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
          title: "work/20260610-0120-4",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/20260610-0120-4",
          updated_at: "2026-06-11T05:00:00Z",
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

  const title = fixture.body.querySelector(".workspace-overview-row-title");
  assert.equal(title.textContent, "work/20260610-0120-4", "verbatim branch text is preserved");
  assert.equal(
    title.querySelector(".workspace-branch-prefix")?.textContent,
    "work/",
    "namespace prefix is split for dimming",
  );
  assert.equal(
    title.querySelector(".workspace-branch-leaf")?.textContent,
    "20260610-0120-4",
    "leaf is split for emphasis",
  );
  // The row carries its relative updated time, right-aligned via CSS.
  assert.ok(
    fixture.body.querySelector(".workspace-overview-row-time"),
    "row shows the relative updated time",
  );
});

// Design pass (2026-06-11): each Work group carries the agent color keyword so
// the existing [data-agent-color] → --current-agent CSS identity system colors
// the group rail and agent dot (SPEC-2133 agent colors).
test("work groups carry the agent identity color keyword", () => {
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
          agents: [
            {
              session_id: "sess-claude",
              agent_id: "claude",
              display_name: "Claude Code",
              affiliation_status: "assigned",
              status_category: "idle",
              updated_at: "2026-06-10T12:00:00Z",
              sessions: [],
            },
            {
              session_id: "sess-codex",
              agent_id: "codex",
              display_name: "Codex",
              affiliation_status: "assigned",
              status_category: "idle",
              updated_at: "2026-06-10T11:00:00Z",
              sessions: [],
            },
          ],
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

  const groups = [...fixture.body.querySelectorAll(".workspace-detail-work-group")];
  assert.equal(groups.length, 2);
  assert.equal(groups[0].dataset.agentColor, "yellow", "Claude maps to the claude color");
  assert.equal(groups[1].dataset.agentColor, "cyan", "Codex maps to the codex color");
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

// SPEC-2359 W-15 / US-78: a merged Workspace always shows the Merged badge,
// while the stronger safe-to-delete signal is reserved for backend-vetted
// cleanup candidates.
test("cleanup-candidate Workspace shows the safe-to-delete detail signal", () => {
  const fixture = createFixture();
  const surface = createSurface(
    fixture,
    {
      id: "proj-1",
      title: "projection",
      status_category: "idle",
      active_work_count: 2,
      active_works: [
        {
          id: "work-merged",
          title: "work/merged",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/merged",
          merged_into_base: true,
          cleanup_candidate: {
            branch: "work/merged",
            worktree_path: "/repo/work/merged",
            reason: "pr_merged",
            default_delete_remote: false,
            remote_delete_available: true,
          },
          active_agents: 0,
          blocked_agents: 0,
          agents: [],
        },
        {
          id: "work-open",
          title: "work/open",
          status_category: "idle",
          lifecycle_state: "paused",
          branch: "work/open",
          merged_into_base: false,
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

  const rows = Array.from(
    fixture.body.querySelectorAll(".workspace-overview-row[data-workspace-id]"),
  );
  const mergedRow = rows.find((row) => row.dataset.workspaceId === "work-merged");
  const openRow = rows.find((row) => row.dataset.workspaceId === "work-open");
  assert.ok(
    mergedRow.querySelector(".workspace-overview-merged"),
    "merged row carries the Merged badge",
  );
  assert.equal(
    openRow.querySelector(".workspace-overview-merged"),
    null,
    "unmerged row has no badge",
  );

  mergedRow.click();
  assert.match(
    fixture.body.querySelector(".workspace-detail-subtitle").textContent,
    /Merged — safe to delete/,
    "detail subtitle carries the safe-to-delete signal",
  );
});

// SPEC-2359 W-17 (FR-398): Resume entry points show pending state and guard
// against double-sends via the shared launch-pending controller.
test("Resume click marks the Work pending and a re-click does not re-send", () => {
  const projection = sampleProjection();
  projection.works[0].agents[0].status_category = "idle";
  projection.works[0].agents[0].session_id = "work-launch-1";
  const fixture = createFixture();
  const sent = [];
  const launchPending = createLaunchPendingController({
    setTimeoutFn: () => 1,
    clearTimeoutFn: () => {},
  });
  const surface = createSurface(fixture, projection, {
    send: (message) => sent.push(message),
    getResumeBounds: () => ({ x: 0, y: 0, width: 800, height: 600 }),
    launchPending,
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const resume = fixture.body.querySelector("[data-action='resume-work']");
  resume.click();
  assert.equal(sent.length, 1);
  assert.equal(
    launchPending.isPending("session:work-launch-1"),
    true,
    "click registers the Work as pending",
  );

  resume.click();
  assert.equal(sent.length, 1, "re-click while pending must not re-send");
});

test("a pending Work renders its Resume button disabled with progress label", () => {
  const projection = sampleProjection();
  projection.works[0].agents[0].status_category = "idle";
  projection.works[0].agents[0].session_id = "work-launch-1";
  const fixture = createFixture();
  const launchPending = createLaunchPendingController({
    setTimeoutFn: () => 1,
    clearTimeoutFn: () => {},
  });
  launchPending.begin("session:work-launch-1", "Resume");
  const surface = createSurface(fixture, projection, {
    getResumeBounds: () => ({ x: 0, y: 0, width: 800, height: 600 }),
    launchPending,
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const resume = fixture.body.querySelector("[data-action='resume-work']");
  assert.ok(resume, "Resume control still renders while pending");
  assert.equal(resume.disabled, true, "pending Work disables its Resume");
  assert.match(resume.textContent, /Resuming/);
});
