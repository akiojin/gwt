// SPEC-3671 P5 — surface naming.
//
// FR-014: `issue` / `issue_monitor` / `spec` all fell back to the single label
// "Issue", so an open window's title could not tell the user which face it was.
// FR-015: ADD WINDOW called the Work surface "Workspace" while its window title
// said "Work"; the surface lists Works (launches), so "Work" is the实体 name.
// FR-016 / 受け入れシナリオ 10: the Work window stays selectable while any Work
// information is still only available there.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const { document } = parseHTML(readFileSync(resolve(here, "../index.html"), "utf8"));
const modal = document.getElementById("preset-modal");

function presetRoleLabels() {
  const start = appSource.indexOf("function presetRoleLabel(");
  assert.notEqual(start, -1, "expected presetRoleLabel in app.js");
  const open = appSource.indexOf("{", appSource.indexOf("const labels =", start));
  const close = appSource.indexOf("};", open);
  const body = appSource.slice(open + 1, close);
  const labels = {};
  for (const match of body.matchAll(/([a-z_]+):\s*"([^"]*)"/g)) {
    labels[match[1]] = match[2];
  }
  return labels;
}

// FR-014 / 受け入れシナリオ 9.
test("the three Issue-family presets carry distinguishable role labels", () => {
  const labels = presetRoleLabels();

  assert.equal(labels.issue, "Issue");
  assert.equal(labels.issue_monitor, "Issue Monitor");
  assert.equal(labels.spec, "SPEC");

  const issueFamily = [labels.issue, labels.issue_monitor, labels.spec];
  assert.equal(
    new Set(issueFamily).size,
    issueFamily.length,
    "each Issue-family face must be tellable apart from its window title",
  );
});

// FR-015.
test("the Work surface is named Work in ADD WINDOW and in its role label", () => {
  const labels = presetRoleLabels();
  assert.equal(labels.work, "Work", "the wire preset is `work`, so that key must be labelled");

  const workCard = modal.querySelector('.preset-button[data-preset="work"]');
  assert.ok(workCard, "the Work surface stays selectable in ADD WINDOW");
  assert.equal(workCard.querySelector("strong")?.textContent.trim(), "Work");
  assert.match(
    workCard.querySelector(".preset-button__text span")?.textContent ?? "",
    /Work overview/,
    "the ADD WINDOW description must describe the Work surface, not a Workspace",
  );
  assert.doesNotMatch(
    workCard.textContent,
    /Workspace/,
    "the card must not keep the old Workspace naming",
  );
});

// FR-016 / 受け入れシナリオ 10: the Work window is only removed from ADD WINDOW once
// every Work information item has moved to the Issue row. The CI check rollup is
// not carried by the active Work projection, so the migration is incomplete and
// this card must stay.
test("the Work window remains selectable while Work information migration is incomplete", () => {
  assert.ok(
    modal.querySelector('.preset-button[data-preset="work"]'),
    "removing the Work card before the migration completes would delete the only home of that information",
  );
});
