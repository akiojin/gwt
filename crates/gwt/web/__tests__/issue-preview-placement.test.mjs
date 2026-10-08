import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

import {
  isAgentKanbanEligible,
  isAgentKanbanPlacement,
  isOffCanvasPlacement,
  windowsForAgentKanbanLane,
} from "../agent-kanban-surface.js";

const here = dirname(fileURLToPath(import.meta.url));
const surfaceSource = readFileSync(resolve(here, "../agent-kanban-surface.js"), "utf8");

function issuePreviewWindow(overrides = {}) {
  return {
    id: "agent-1",
    preset: "agent",
    title: "Implement SPEC-3671",
    placement: {
      kind: "issue_preview",
      issue_window_id: "issue-1",
      issue_number: 3671,
    },
    ...overrides,
  };
}

// SPEC-3671 FR-004 / T-011.
test("issue_preview windows are excluded from canvas rendering", () => {
  const windowData = issuePreviewWindow();

  assert.equal(isOffCanvasPlacement(windowData), true);
  assert.equal(
    isAgentKanbanPlacement(windowData),
    false,
    "an Issue preview is not a Kanban card",
  );
});

// SPEC-3671 T-010: adding the new placement must only touch the off-canvas seam. It
// must not leak into Kanban lane membership or Kanban drop eligibility.
test("issue_preview does not leak into Agent Kanban lane membership", () => {
  const windowData = issuePreviewWindow();

  assert.deepEqual(windowsForAgentKanbanLane([windowData], "kanban-1", "active"), []);
  assert.equal(
    isAgentKanbanEligible(windowData),
    false,
    "an off-canvas window is not a free canvas window that can be dropped into a lane",
  );
});

test("canvas and agent_kanban placements keep their previous off-canvas answers", () => {
  assert.equal(isOffCanvasPlacement({ id: "a", preset: "agent" }), false);
  assert.equal(isOffCanvasPlacement({ id: "a", preset: "agent", placement: {} }), false);
  assert.equal(
    isOffCanvasPlacement({ id: "a", preset: "agent", placement: { kind: "canvas" } }),
    false,
  );
  assert.equal(
    isOffCanvasPlacement({
      id: "a",
      preset: "agent",
      placement: { kind: "agent_kanban", board_id: "kanban-1", lane_id: "active", order: 0 },
    }),
    true,
  );
  assert.equal(isOffCanvasPlacement(undefined), false);
  assert.equal(isOffCanvasPlacement(null), false);
});

// SPEC-3671 T-010: the predicate is the single place that enumerates off-canvas kinds.
test("off-canvas kinds are enumerated in exactly one predicate", () => {
  const occurrences = surfaceSource.match(/"issue_preview"/g) || [];
  assert.equal(
    occurrences.length,
    1,
    "issue_preview must be named once, inside isOffCanvasPlacement()",
  );
  const predicate = surfaceSource.slice(
    surfaceSource.indexOf("export function isOffCanvasPlacement"),
  );
  assert.match(predicate.slice(0, 400), /"issue_preview"/);
});
