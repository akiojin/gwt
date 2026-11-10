// SPEC-3671 P5 — surface naming.
//
// FR-014: `issue` / `issue_monitor` / `spec` all fell back to the single label
// "Issue", so an open window's title could not tell the user which face it was.
// FR-015: ADD WINDOW called the Work surface "Workspace" while its window title
// said "Work"; the surface lists Works (launches), so "Work" is the实体 name.
// FR-016 / 受け入れシナリオ 10: the Work window stayed selectable while any Work
// information was still only available there. T-023 / T-024 moved every item
// (lifecycle, needs-attention reason, PR number / state, Continue / Resume /
// cleanup) to the Issue row, and the PM ruled the CI summary out of scope
// (2026-09-02), so the Work card now leaves ADD WINDOW. Windows that are already
// open keep working: the `work` preset still resolves to the Work surface.

import assert from "node:assert/strict";
import { readFileSync, realpathSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

test("the status strip uses the Issues label and navigation name", () => {
  const html = readFileSync(resolve(here, "../index.html"), "utf8");
  assert.match(html, /aria-label="Open Issues: Off"/);
  assert.match(html, /op-status-strip__label">ISSUES<\/span>/);
});

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
  assert.equal(labels.issue_monitor, "Issues");
  assert.equal(labels.spec, "SPEC");

  const issueFamily = [labels.issue, labels.issue_monitor, labels.spec];
  assert.equal(
    new Set(issueFamily).size,
    issueFamily.length,
    "each Issue-family face must be tellable apart from its window title",
  );

  const html = readFileSync(resolve(here, "../index.html"), "utf8");
  const workLabel = html.match(/data-surface="works"[^>]*>[\s\S]*?op-rail__surface-label">([^<]+)/)?.[1];
  assert.equal(workLabel, "Works", "the current Work surface retains its own visible label");
  assert.equal(new Set([labels.issue, labels.issue_monitor, workLabel]).size, 3,
    "Issue, Issues and the current Work surface remain distinguishable");
});

test("user and agent copy uses the Issues name across shipped surfaces", () => {
  const root = resolve(realpathSync(here), "../../../..");
  const oldName = new RegExp(["issue", "monitor"].join("\\s+"), "i");
  const paths = execFileSync("git", ["ls-files", "-z", "--",
    "*.rs", "*.md", "*.js", "*.mjs", "*.ts", "*.html", "*.css", "*.py", "*.ps1", "*.yml",
    ":(exclude).gwt/**"], { cwd: root, encoding: "utf8" })
    .split("\0")
    .filter((path) => /\.(?:rs|md|js|mjs|ts|html|css|py|ps1|yml)$/.test(path));
  const remaining = paths.filter((path) => oldName.test(readFileSync(resolve(root, path), "utf8")));
  assert.deepEqual(remaining, [], "shipped source, skill and documentation copy must use Issues");
});

// Work was retired and now restores Issue; it must still be distinct from Issues.
test("legacy Work windows retain their Issue label after migration", () => {
  const labels = presetRoleLabels();
  assert.equal(labels.work, "Issue", "the legacy wire preset is `work`, so that key must be labelled");
  assert.equal(labels.workspace, labels.work);
  assert.notEqual(labels.work, labels.issue_monitor);
  assert.doesNotMatch(
    appSource.slice(
      appSource.indexOf("function presetRoleLabel("),
      appSource.indexOf("}", appSource.indexOf("const labels =")),
    ),
    /Workspace/,
    "the role label must not keep the old Workspace naming",
  );
});

// FR-016: windows that are already open keep working — the `work` preset (and
// its legacy `workspace` / `branches` spellings) must still resolve to the Work
// surface so persisted windows render and reopen as before.
test("already-open Work windows migrate to the Issue surface", () => {
  assert.match(
    appSource,
    /function presetSurface\(preset\)[\s\S]+?preset\s*===\s*"work"\s*\|\|\s*preset\s*===\s*"workspace"[\s\S]+?return\s+"knowledge"/,
    "presetSurface must keep mapping work/workspace to the Work surface",
  );
  assert.match(
    appSource,
    /function normalizeSurfacePreset\(preset\)[\s\S]+?preset\s*===\s*"branches"\s*\|\|\s*preset\s*===\s*"workspace"[\s\S]+?return\s+"issue"/,
    "normalizeSurfacePreset must keep folding legacy spellings onto work",
  );
});
