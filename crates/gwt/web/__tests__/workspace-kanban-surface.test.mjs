import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { createWorkspaceKanbanSurface } from "../workspace-kanban-surface.js";

function createNodeFactory(document) {
  return (tag, className = "", text = null) => {
    const element = document.createElement(tag);
    if (className) {
      element.className = className;
    }
    if (text !== null && text !== undefined) {
      element.textContent = String(text);
    }
    return element;
  };
}

function appendMeta(parent, value) {
  const text = String(value || "").trim();
  if (!text) return;
  const meta = parent.ownerDocument.createElement("span");
  meta.className = "kanban-card-chip";
  meta.textContent = text;
  parent.appendChild(meta);
}

function agentStatusLabel(state) {
  switch (String(state || "").toLowerCase()) {
    case "active":
      return "Running";
    case "blocked":
      return "Blocked";
    case "idle":
      return "Idle";
    case "done":
      return "Done";
    default:
      return "Unknown";
  }
}

function mountWorkspaceKanban(projection) {
  const { document } = parseHTML("<!doctype html><html><body></body></html>");
  const body = document.createElement("main");
  const windowMap = new Map([["workspace-1", body]]);
  const surface = createWorkspaceKanbanSurface({
    activeWorkspace: () => ({ title: "Repo" }),
    agentStatusLabel,
    appendMeta,
    createWorkspacePrMeta: () => null,
    createNode: createNodeFactory(document),
    getActiveWorkProjection: () => projection,
    openWorkspaceCleanup: () => {},
    send: () => {},
    windowMap,
    workspaceWindowById: (id) => (
      id === "workspace-1" ? { id, preset: "workspace" } : null
    ),
  });

  surface.mount(body, { id: "workspace-1" }, {
    focusWindowLocally: () => {},
    sendFocus: () => {},
  });

  return { body };
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

test("Workspace Kanban keeps journal history out of the Active column", () => {
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

  const { body } = mountWorkspaceKanban(projection);
  const activeTexts = cardTexts(column(body, "active"));
  const inactiveTexts = cardTexts(column(body, "inactive"));
  const completedTexts = cardTexts(column(body, "completed"));

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
  const { body } = mountWorkspaceKanban({
    id: "current-work",
    title: "Current work",
    status_category: "idle",
    journal_entries: [],
  });

  const inactiveColumn = column(body, "inactive");
  assert.match(
    inactiveColumn.getAttribute("aria-label"),
    /Inactive Workspace column/,
  );
  assert.equal(
    inactiveColumn.querySelector(".workspace-column-name").textContent,
    "Inactive",
  );
});
