import assert from "node:assert/strict";
import test from "node:test";

import { parseHTML } from "linkedom";

import {
  applyWindowWorktreeData,
  renderWindowWorktreeBadge,
  shouldShowWindowWorktreeBadge,
  windowWorktreeBadgeView,
  windowWorktreeForm,
} from "../window-worktree-form.js";

test("legacy wire lane values adapt to semantic worktree forms", () => {
  assert.equal(windowWorktreeForm({ lane_kind: "intake" }), "ephemeral");
  assert.equal(windowWorktreeForm({ laneKind: "execution" }), "branch-backed");
  assert.equal(windowWorktreeForm({ lane_kind: "unknown" }), "unknown");
  assert.equal(windowWorktreeForm({ lane_kind: "garbage" }), "unknown");
  assert.equal(windowWorktreeForm({}), "unknown");
});

test("badge view keeps worktree form separate from provider identity", () => {
  const ephemeral = windowWorktreeBadgeView({
    preset: "agent",
    agent_id: "codex",
    agent_color: "cyan",
    lane_kind: "intake",
  });
  const branchBacked = windowWorktreeBadgeView({
    preset: "agent",
    agent_id: "codex",
    agent_color: "cyan",
    lane_kind: "execution",
  });

  assert.deepEqual(ephemeral, {
    form: "ephemeral",
    label: "Ephemeral",
    shortLabel: "Ephemeral",
    symbol: "Ø",
    ariaLabel: "Ephemeral branchless worktree",
    title: "Ephemeral branchless worktree",
    providerColor: "cyan",
  });
  assert.deepEqual(branchBacked, {
    form: "branch-backed",
    label: "Branch-backed",
    shortLabel: "Branch-backed",
    symbol: "B",
    ariaLabel: "Branch-backed worktree",
    title: "Branch-backed worktree",
    providerColor: "cyan",
  });
});

test("unknown worktree form is visible for restored agents but not ordinary panels", () => {
  assert.equal(
    shouldShowWindowWorktreeBadge({ preset: "agent" }),
    true,
  );
  assert.equal(
    shouldShowWindowWorktreeBadge({ preset: "file_tree", lane_kind: "unknown" }),
    false,
  );
  assert.equal(
    shouldShowWindowWorktreeBadge({ preset: "file_tree", lane_kind: "intake" }),
    false,
  );
  assert.deepEqual(windowWorktreeBadgeView({ preset: "agent" }), {
    form: "unknown",
    label: "Unknown worktree form",
    shortLabel: "?",
    symbol: "?",
    ariaLabel: "Unknown worktree form",
    title: "Unknown worktree form",
    providerColor: "",
  });
});

test("DOM helpers attach semantic worktree data and accessible badge labels", () => {
  const { document } = parseHTML("<div><span></span></div>");
  const root = document.querySelector("div");
  const badge = document.querySelector("span");
  const windowData = {
    preset: "agent",
    agent_id: "codex",
    lane_kind: "intake",
  };

  applyWindowWorktreeData(root, windowData);
  renderWindowWorktreeBadge(badge, windowData);

  assert.equal(root.dataset.worktreeForm, "ephemeral");
  assert.equal(root.dataset.worktreeLabel, "Ephemeral");
  assert.equal(root.dataset.worktreeSymbol, "Ø");
  assert.equal(badge.hidden, false);
  assert.equal(badge.dataset.worktreeForm, "ephemeral");
  assert.equal(badge.dataset.worktreeLabel, "Ephemeral");
  assert.equal(badge.dataset.worktreeSymbol, "Ø");
  assert.equal(badge.textContent, "Ephemeral");
  assert.equal(badge.getAttribute("aria-label"), "Ephemeral branchless worktree");
  assert.equal(badge.title, "Ephemeral branchless worktree");
});

test("hidden ordinary-panel badges clear semantic attributes", () => {
  const { document } = parseHTML("<span></span>");
  const badge = document.querySelector("span");

  renderWindowWorktreeBadge(badge, {
    preset: "agent",
    lane_kind: "execution",
  });
  renderWindowWorktreeBadge(badge, {
    preset: "file_tree",
    lane_kind: "unknown",
  });

  assert.equal(badge.hidden, true);
  assert.equal(badge.textContent, "");
  assert.equal(badge.dataset.worktreeForm, undefined);
  assert.equal(badge.dataset.worktreeLabel, undefined);
  assert.equal(badge.dataset.worktreeSymbol, undefined);
  assert.equal(badge.getAttribute("aria-label"), null);
  assert.equal(badge.getAttribute("title"), null);
});
