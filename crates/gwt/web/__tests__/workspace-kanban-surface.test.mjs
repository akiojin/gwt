import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { createWorkspaceKanbanSurface } from "../workspace-kanban-surface.js";

test("Workspace journal cards derive titles from each entry instead of the current Workspace title", () => {
  const fixture = createFixture();
  const projection = {
    id: "current-workspace",
    title: "Workspace Kanban Surface Extraction",
    status_category: "active",
    status_text: "Current work is active",
    summary: "Current work summary",
    owner: "SPEC-2359",
    journal_entries: [
      {
        id: "journal-one",
        status_category: "done",
        summary: "Scroll fix details remain in the summary body",
        agent_title_summary: "Kanban scroll PR",
        updated_at: "2026-05-08T15:14:43Z",
      },
      {
        id: "journal-two",
        status_category: "active",
        summary: "Update CTA dismiss PR",
        updated_at: "2026-05-08T14:07:07Z",
      },
      {
        id: "journal-focus-only",
        status_category: "idle",
        status_text: "Workspace update",
        next_action: "Continue",
        agent_current_focus: "Review thread title fallback",
        updated_at: "2026-05-08T13:00:00Z",
      },
    ],
    agents: [],
  };

  const surface = createSurface(fixture, projection);
  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const titles = Array.from(
    fixture.document.querySelectorAll(".workspace-kanban-card .kanban-card-title"),
    (node) => node.textContent,
  );

  assert.equal(
    titles.filter((title) => title === "Workspace Kanban Surface Extraction").length,
    1,
    "only the current card should use the current Workspace title",
  );
  assert.ok(
    titles.includes("Kanban scroll PR"),
    "journal cards should prefer agent_title_summary as their visible title",
  );
  assert.ok(
    titles.includes("Update CTA dismiss PR"),
    "journal cards without title_summary should derive a visible title from their own summary",
  );
  assert.ok(
    titles.includes("Review thread title fallback"),
    "focus-only journal cards should prefer agent_current_focus before generic status text",
  );
});

test("Workspace Kanban keeps journal history out of the Active column", () => {
  const fixture = createFixture();
  const projection = {
    id: "current-work",
    title: "Current work",
    status_category: "active",
    status_text: "Current work is running",
    journal_entries: [
      {
        id: "statusless",
        status_category: null,
        agent_title_summary: "Statusless journal",
        summary: "A historical entry without an explicit status.",
      },
      {
        id: "old-active",
        status_category: "active",
        agent_title_summary: "Old active journal",
        summary: "This entry was active when written, but it is not current work.",
      },
      {
        id: "old-blocked",
        status_category: "blocked",
        agent_title_summary: "Old blocked journal",
        summary: "This blocked state is historical.",
      },
      {
        id: "done",
        status_category: "done",
        agent_title_summary: "Completed journal",
        summary: "This entry is complete.",
      },
    ],
  };

  const surface = createSurface(fixture, projection);
  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const activeTexts = cardTexts(column(fixture.body, "active"));
  const inactiveTexts = cardTexts(column(fixture.body, "inactive"));
  const completedTexts = cardTexts(column(fixture.body, "completed"));

  assert.equal(activeTexts.length, 1);
  assert.match(activeTexts[0], /Current work/);
  assert.equal(inactiveTexts.length, 3);
  assert.match(inactiveTexts.join("\n"), /Statusless journal/);
  assert.match(inactiveTexts.join("\n"), /Old active journal/);
  assert.match(inactiveTexts.join("\n"), /Old blocked journal/);
  assert.equal(completedTexts.length, 1);
  assert.match(completedTexts[0], /Completed journal/);
});

test("Workspace Kanban labels the non-current history column as Inactive", () => {
  const fixture = createFixture();
  const surface = createSurface(fixture, {
    id: "current-work",
    title: "Current work",
    status_category: "idle",
    journal_entries: [],
  });

  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const inactiveColumn = column(fixture.body, "inactive");
  assert.match(
    inactiveColumn.getAttribute("aria-label"),
    /Inactive Workspace column/,
  );
  assert.equal(
    inactiveColumn.querySelector(".workspace-column-name").textContent,
    "Inactive",
  );
});

test("Workspace Kanban renders Unassigned agents outside Workspace status columns", () => {
  const fixture = createFixture();
  const projection = {
    id: "project-workspace",
    title: "Project workspace",
    status_category: "idle",
    status_text: "No active Workspace selected",
    journal_entries: [],
    workspaces: [],
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

  const surface = createSurface(fixture, projection);
  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const unassigned = fixture.body.querySelector("[data-role='unassigned-agents']");
  assert.ok(unassigned, "Unassigned section should be rendered");
  assert.match(unassigned.textContent, /Unassigned/);
  assert.match(unassigned.textContent, /No Workspace selected/);
  assert.match(unassigned.textContent, /Codex/);
  assert.equal(cardTexts(column(fixture.body, "active")).length, 0);
  assert.equal(cardTexts(column(fixture.body, "inactive")).length, 1);
  assert.doesNotMatch(fixture.body.textContent, /WorkItem/);
});

test("Workspace Kanban renders Workspace history cards with lifecycle timeline detail", () => {
  const fixture = createFixture();
  const projection = {
    id: "current-workspace",
    title: "Legacy current Workspace",
    status_category: "idle",
    status_text: "Idle",
    workspaces: [
      {
        id: "workspace-history",
        title: "Workspace history",
        intent: "Group duplicate work under one Workspace",
        summary: "One Workspace owns the history.",
        status_category: "active",
        owner: "SPEC-2359",
        agents: [
          {
            session_id: "session-other",
            display_name: "Codex",
            status_category: "active",
          },
        ],
        execution_containers: [
          {
            branch: "work/20260510-2353",
            worktree_path: "/repo/work/20260510-2353",
            pr_number: 2638,
            pr_url: "https://github.com/akiojin/gwt/pull/2638",
            pr_state: "open",
          },
        ],
        board_refs: ["board-claim-1"],
        events: [
          {
            id: "evt-start",
            kind: "start",
            title: "Start Workspace",
            summary: "Started lifecycle implementation.",
            updated_at: "2026-05-11T01:00:00Z",
            agent_session_id: "session-other",
            board_entry_id: "board-claim-1",
          },
          {
            id: "evt-blocked",
            kind: "blocked",
            summary: "Waiting for Board coordination.",
            updated_at: "2026-05-11T01:10:00Z",
            agent_session_id: "session-other",
            board_entry_id: "board-blocked-1",
          },
        ],
      },
    ],
    journal_entries: [
      {
        id: "legacy-journal",
        status_category: "active",
        agent_title_summary: "Legacy duplicate card",
      },
    ],
  };

  const surface = createSurface(fixture, projection);
  surface.mount(fixture.body, fixture.windowData, {
    focusWindowLocally() {},
    sendFocus() {},
  });

  const cardText = cardTexts(column(fixture.body, "active")).join("\n");
  assert.match(cardText, /Workspace/);
  assert.match(cardText, /Workspace history/);
  assert.doesNotMatch(cardText, /WorkItem/);
  assert.doesNotMatch(cardText, /Legacy duplicate card/);

  const detailText = fixture.body
    .querySelector(".workspace-kanban-detail-pane")
    .textContent.replace(/\s+/g, " ")
    .trim();
  assert.match(detailText, /Lifecycle/);
  assert.match(detailText, /start/);
  assert.match(detailText, /Started lifecycle implementation/);
  assert.match(detailText, /board-claim-1/);
  assert.match(detailText, /blocked/);
  assert.match(detailText, /Waiting for Board coordination/);
  assert.match(detailText, /board-blocked-1/);
});

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

function createSurface(fixture, projection) {
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
    createWorkspacePrMeta: () => null,
    createNode: (tag, className, text) =>
      createNode(fixture.document, tag, className, text),
    getActiveWorkProjection: () => projection,
    openWorkspaceCleanup() {},
    send() {},
    windowMap: fixture.windowMap,
    workspaceWindowById: (windowId) =>
      workspace.windows.find((window) => window.id === windowId) || null,
  });
}

function createNode(document, tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function column(body, key) {
  const element = body.querySelector(`[data-workspace-column="${key}"]`);
  assert.ok(element, `missing Workspace Kanban column: ${key}`);
  return element;
}

function cardTexts(columnElement) {
  return Array.from(columnElement.querySelectorAll(".workspace-kanban-card"))
    .map((card) => card.textContent.replace(/\s+/g, " ").trim());
}
