import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));

async function loadBoardLogsSurface() {
  const source = readFileSync(resolve(here, "../board-logs-surface.js"), "utf8").replace(
    'from "/board-surface.js"',
    `from ${JSON.stringify(pathToFileURL(resolve(here, "../board-surface.js")).href)}`,
  );
  return import(`data:text/javascript,${encodeURIComponent(source)}`);
}

function createNodeFactory(document) {
  return (tagName, className = "", textContent) => {
    const node = document.createElement(tagName);
    node.className = className;
    if (textContent !== undefined) node.textContent = textContent;
    return node;
  };
}

test("Intake BackendEvent renders a dedicated filter/lane/card and Recovery Center deep link", async () => {
  const { document, window } = parseHTML(`
    <div id="board-window"><div class="window-body"></div></div>
  `);
  const previousDocument = globalThis.document;
  const previousWindow = globalThis.window;
  globalThis.document = document;
  globalThis.window = window;
  try {
    const { createBoardLogsSurface } = await loadBoardLogsSurface();
    const windowElement = document.getElementById("board-window");
    const body = windowElement.querySelector(".window-body");
    const sent = [];
    let recoveryCenterOpens = 0;
    const createNode = createNodeFactory(document);
    const surface = createBoardLogsSurface({
      send: (message) => sent.push(message),
      createNode,
      createKnowledgeMarkdownBody: (entry, className) => {
        const node = createNode("div", className);
        if (entry.body_html) node.innerHTML = entry.body_html;
        else node.textContent = entry.body || "";
        return node;
      },
      windowMap: new Map([["board-1", windowElement]]),
      focusWindowLocally: () => {},
      pushAlertToast: () => {},
      sendWindowFocus: () => {},
      focusOrSpawnPreset: () => {},
      openRecoveryCenter: () => { recoveryCenterOpens += 1; },
      activeWorkspace: () => ({ windows: [{ id: "board-1", preset: "board" }] }),
      activeProjectTab: () => ({ project_root: "C:/repo" }),
      visibleBounds: () => ({ x: 0, y: 0, width: 1200, height: 800 }),
      getActiveWorkProjection: () => ({ active_works: [] }),
    });

    surface.mountBoardWindow({ id: "board-1", preset: "board" }, body);
    surface.applyBoardLogsReceiveEvent({
      kind: "board_entries",
      id: "board-1",
      total_entries: 3,
      has_more_before: true,
      entries: [
        {
          id: "work-entry",
          author_kind: "agent",
          author: "Codex",
          body: "Work only",
          created_at: "2026-07-16T09:00:00Z",
          updated_at: "2026-07-16T09:00:00Z",
          audience: ["other-work"],
        },
        {
          id: "intake-current",
          author_kind: "agent",
          author: "gwt-discussion",
          title: "Intake checkpoint: Unified recovery",
          body: "Current Intake checkpoint body",
          body_html: "<p>Current <strong>Intake</strong> checkpoint body</p>",
          created_at: "2026-07-16T10:00:00Z",
          updated_at: "2026-07-16T10:00:00Z",
          audience: ["other-work"],
          origin_session_id: "session-intake",
          origin_session_kind: "intake",
          origin_recovery_id: "recovery-intake",
          origin_branch: null,
          origin_agent_id: "codex",
        },
        {
          id: "legacy-current-summary",
          author_kind: "agent",
          author: "gwt-discussion",
          title: "Current recovery summary",
          body: "Legacy current Intake summary",
          created_at: "2026-07-16T10:01:00Z",
          updated_at: "2026-07-16T10:01:00Z",
          related_topics: ["intake"],
          origin_session_kind: null,
          origin_branch: null,
        },
      ],
    });

    body.querySelector("[data-action='toggle-board-intake']").click();

    const intakeLane = body.querySelector('[data-lane-key="__intake__"]');
    assert.ok(intakeLane, "nullable-branch Intake entries must not be buried in General");
    assert.equal(intakeLane.querySelector(".board-lane-label").textContent, "Intake");
    assert.deepEqual(
      Array.from(intakeLane.querySelectorAll("[data-board-entry-id]")).map(
        (node) => node.dataset.boardEntryId,
      ),
      ["intake-current", "legacy-current-summary"],
    );
    const card = intakeLane.querySelector('[data-board-entry-id="intake-current"]');
    assert.equal(
      card.querySelector(".board-message-title").textContent,
      "Intake checkpoint: Unified recovery",
    );
    assert.match(card.querySelector(".board-message-body").textContent, /Current Intake checkpoint/);
    assert.match(card.querySelector(".board-origin-badge").textContent, /session-/);
    assert.equal(body.querySelector(".board-status").textContent, "Visible 2 / Total 3");

    body.querySelector("[data-action='open-recovery-center']").click();
    assert.equal(recoveryCenterOpens, 1);
    assert.equal(
      sent.some((message) => message.kind === "load_board" && message.all === true),
      true,
      "aggregate Intake filtering must request the full Board timeline",
    );
  } finally {
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
  }
});
