import assert from "node:assert/strict";
import test from "node:test";

import { parseHTML } from "linkedom";

import {
  applyWindowLaneData,
  renderWindowLaneBadge,
  shouldShowWindowLaneBadge,
  windowLaneBadgeView,
  windowLaneKind,
} from "../window-lane-identity.js";

test("lane kind normalizes intake execution and unknown without falling back to execution", () => {
  assert.equal(windowLaneKind({ lane_kind: "intake" }), "intake");
  assert.equal(windowLaneKind({ lane_kind: "execution" }), "execution");
  assert.equal(windowLaneKind({ lane_kind: "garbage" }), "unknown");
  assert.equal(windowLaneKind({}), "unknown");
});

test("badge view keeps lane identity separate from provider identity", () => {
  const intake = windowLaneBadgeView({
    preset: "agent",
    agent_id: "codex",
    agent_color: "cyan",
    lane_kind: "intake",
  });
  const execution = windowLaneBadgeView({
    preset: "agent",
    agent_id: "codex",
    agent_color: "cyan",
    lane_kind: "execution",
  });

  assert.equal(intake.kind, "intake");
  assert.equal(execution.kind, "execution");
  assert.equal(intake.providerColor, "cyan");
  assert.equal(execution.providerColor, "cyan");
  assert.notEqual(intake.shortLabel, execution.shortLabel);
  assert.match(intake.ariaLabel, /Intake lane/);
  assert.match(execution.ariaLabel, /Execution lane/);
});

test("unknown lane is visible only for agent windows, not ordinary panels", () => {
  assert.equal(
    shouldShowWindowLaneBadge({ preset: "agent", lane_kind: "unknown" }),
    true,
  );
  assert.equal(
    shouldShowWindowLaneBadge({ preset: "file_tree", lane_kind: "unknown" }),
    false,
  );
});

test("DOM helpers attach data-lane-kind and accessible badge labels", () => {
  const { document } = parseHTML("<div><span></span></div>");
  const root = document.querySelector("div");
  const badge = document.querySelector("span");
  const windowData = {
    preset: "agent",
    agent_id: "codex",
    lane_kind: "intake",
  };

  applyWindowLaneData(root, windowData);
  renderWindowLaneBadge(badge, windowData);

  assert.equal(root.dataset.laneKind, "intake");
  assert.equal(root.dataset.laneLabel, "Intake");
  assert.equal(badge.hidden, false);
  assert.equal(badge.dataset.laneKind, "intake");
  assert.equal(badge.textContent, "Intake");
  assert.equal(badge.getAttribute("aria-label"), "Intake lane");
});
