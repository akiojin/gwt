// SPEC-1939 Phase 15 — Index window health panel renderer.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { renderIndexSettingsPanel } from "../index-settings-panel.js";

function fixture() {
  const { document } = parseHTML(
    `<!doctype html><body><section id="panel"></section></body>`,
  );
  const sent = [];
  return {
    document,
    panel: document.getElementById("panel"),
    send: (message) => {
      sent.push(message);
    },
    sent,
  };
}

test("renderIndexSettingsPanel shows empty hint when no project root is selected", () => {
  const ctx = fixture();
  renderIndexSettingsPanel({
    panel: ctx.panel,
    status: null,
    projectRoot: "",
    send: ctx.send,
    document: ctx.document,
  });

  const empty = ctx.panel.querySelector("[data-role='index-settings-empty']");
  assert.ok(empty, "expected the empty state placeholder");
  assert.equal(ctx.panel.querySelector("table"), null);
});

test("renderIndexSettingsPanel renders one row per scope and one column per worktree", () => {
  const ctx = fixture();
  renderIndexSettingsPanel({
    panel: ctx.panel,
    status: {
      scopes: {
        issues: { healthy: true, document_count: 1500, repair_required: false, reason: "ready" },
        specs: {
          healthy: false,
          repair_required: true,
          document_count: 5,
          reason: "count_mismatch",
        },
        discussions: {
          healthy: true,
          document_count: 8,
          repair_required: false,
          reason: "ready",
        },
        board: { healthy: true, document_count: 21, repair_required: false, reason: "ready" },
        files: {
          wtAhash: { healthy: true, document_count: 310, repair_required: false, reason: "ready" },
          wtBhash: {
            healthy: false,
            repair_required: true,
            document_count: 0,
            reason: "manifest_missing",
          },
        },
        "files-docs": {
          wtAhash: { healthy: true, document_count: 16, repair_required: false, reason: "ready" },
        },
      },
      worktrees: {
        wtAhash: { branch: "develop", path: "/abs/wtA" },
        wtBhash: { branch: "feature/x", path: "/abs/wtB" },
      },
    },
    projectRoot: "/abs/repo",
    send: ctx.send,
    document: ctx.document,
  });

  const table = ctx.panel.querySelector("[data-role='index-settings-table']");
  assert.ok(table, "expected a health table");

  const headerCols = table.querySelectorAll("thead th[scope='col']");
  // 1 scope column + 2 worktree columns
  assert.equal(headerCols.length, 3);
  const headerLabels = Array.from(headerCols).map((th) => th.textContent.trim());
  assert.deepEqual(headerLabels.slice(1).sort(), ["develop", "feature/x"]);

  const scopeRows = Array.from(table.querySelectorAll("tbody tr"));
  const scopes = scopeRows.map((tr) => tr.dataset.scope);
  assert.deepEqual(scopes, [
    "issues",
    "specs",
    "memory",
    "discussions",
    "board",
    "files",
    "files-docs",
  ]);

  // Repo-shared issues row spans all worktree columns and reports ready.
  const issuesCell = scopeRows[0].querySelector(".settings-index-cell");
  assert.equal(issuesCell.colSpan, 2);
  assert.match(issuesCell.textContent, /ready/);

  // The unhealthy specs cell carries the failure reason.
  const specsCell = scopeRows[1].querySelector(".settings-index-cell");
  assert.match(specsCell.textContent, /count_mismatch/);
  assert.ok(specsCell.classList.contains("unhealthy"));

  // memory row is repo-shared and renders a single column-spanning cell.
  const memoryCell = scopeRows[2].querySelector(".settings-index-cell");
  assert.equal(memoryCell.colSpan, 2);

  // discussions row is repo-shared and reports the Discussion semantic index.
  const discussionsCell = scopeRows[3].querySelector(".settings-index-cell");
  assert.equal(discussionsCell.colSpan, 2);
  assert.match(discussionsCell.textContent, /8 docs/);

  // board row is repo-shared and reports the Board semantic index.
  const boardCell = scopeRows[4].querySelector(".settings-index-cell");
  assert.equal(boardCell.colSpan, 2);
  assert.match(boardCell.textContent, /21 docs/);

  // files row has one cell per worktree, in worktree-hash sort order.
  const filesCells = scopeRows[5].querySelectorAll(".settings-index-cell");
  assert.equal(filesCells.length, 2);
  assert.equal(filesCells[0].dataset.worktreeHash, "wtAhash");
  assert.equal(filesCells[1].dataset.worktreeHash, "wtBhash");
  assert.ok(filesCells[1].classList.contains("unhealthy"));

  // files-docs only seeded wtAhash — wtB renders an empty placeholder.
  const docsCells = scopeRows[6].querySelectorAll(".settings-index-cell");
  assert.equal(docsCells.length, 2);
  assert.ok(docsCells[1].classList.contains("settings-index-cell-empty"));
});

test("Rebuild button dispatches rebuild_index_cell with scope + worktree hash", () => {
  const ctx = fixture();
  renderIndexSettingsPanel({
    panel: ctx.panel,
    status: {
      scopes: {
        files: {
          wtAhash: {
            healthy: false,
            repair_required: true,
            document_count: 0,
            reason: "manifest_missing",
          },
        },
      },
      worktrees: {
        wtAhash: { branch: "develop", path: "/abs/wtA" },
      },
    },
    projectRoot: "/abs/repo",
    send: ctx.send,
    document: ctx.document,
  });

  const cellRebuild = ctx.panel.querySelector(
    ".settings-index-cell[data-scope='files'] .settings-index-rebuild",
  );
  assert.ok(cellRebuild, "expected per-cell Rebuild button");
  cellRebuild.click();
  assert.deepEqual(ctx.sent, [
    {
      kind: "rebuild_index_cell",
      project_root: "/abs/repo",
      scope: "files",
      worktree_hash: "wtAhash",
    },
  ]);
});

test("Rebuild all button dispatches rebuild_index_cell without worktree hash", () => {
  const ctx = fixture();
  renderIndexSettingsPanel({
    panel: ctx.panel,
    status: {
      scopes: {
        issues: { healthy: false, repair_required: true, document_count: 0, reason: "missing" },
      },
      worktrees: {},
    },
    projectRoot: "/abs/repo",
    send: ctx.send,
    document: ctx.document,
  });

  const scopeRebuild = ctx.panel.querySelector(".settings-index-rebuild-all[data-scope='issues']");
  assert.ok(scopeRebuild, "expected scope-level Rebuild all button");
  scopeRebuild.click();
  assert.deepEqual(ctx.sent, [
    {
      kind: "rebuild_index_cell",
      project_root: "/abs/repo",
      scope: "issues",
    },
  ]);
});
