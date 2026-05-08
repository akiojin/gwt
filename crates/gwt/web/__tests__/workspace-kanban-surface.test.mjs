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
