// SPEC-2008 FR-035 follow-up DOM smoke test for the Branch Cleanup modal
// renderer. Mounts the modal markup in a linkedom Document and asserts the
// renderer fills the .modal-shell with the correct body for each stage. This
// guards against regressions where only the modal chrome paints (the v9.11.0
// bug that motivated this branch).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

import { renderBranchCleanupModal } from "../branch-cleanup-modal.js";

const here = dirname(fileURLToPath(import.meta.url));
const indexHtmlPath = resolve(here, "..", "index.html");
const indexHtml = readFileSync(indexHtmlPath, "utf8");

function mount() {
  const { document } = parseHTML(indexHtml);
  const modalEl = document.getElementById("branch-cleanup-modal");
  assert.ok(modalEl, "expected #branch-cleanup-modal in index.html");
  const dialogEl = modalEl.querySelector(".modal-shell");
  assert.ok(
    dialogEl,
    "expected #branch-cleanup-modal to contain a .modal-shell child (SPEC-2008 FR-035)",
  );
  const createNode = (tagName, className, textContent) => {
    const node = document.createElement(tagName);
    if (className) {
      node.className = className;
    }
    if (textContent !== undefined) {
      node.textContent = textContent;
    }
    return node;
  };
  return { document, modalEl, dialogEl, createNode };
}

const resultSummary = (results) => {
  const counts = { success: 0, partial: 0, failed: 0 };
  for (const result of results || []) {
    counts[result.status] = (counts[result.status] || 0) + 1;
  }
  return `success ${counts.success} · partial ${counts.partial} · failed ${counts.failed}`;
};

const mergeTargetText = (target) => {
  if (!target) return "";
  if (target.kind === "gone") return "upstream is gone";
  return target.reference ? `merged to ${target.reference}` : "";
};

const riskLabels = (risks) =>
  (risks || []).map((risk) => {
    if (risk === "remote_tracking") return "remote-tracking";
    if (risk === "unmerged") return "unmerged";
    return "warning";
  });

function makeState(overrides = {}) {
  return {
    cleanupModal: {
      open: true,
      stage: "confirm",
      deleteRemote: false,
      results: [],
      timeoutId: null,
      ...overrides.cleanupModal,
    },
  };
}

function makeEntry(name, overrides = {}) {
  return {
    name,
    cleanup_ready: true,
    cleanup: {
      availability: "safe",
      upstream: null,
      merge_target: null,
      execution_branch: null,
      risks: [],
      ...overrides.cleanup,
    },
    ...overrides,
  };
}

test("closed modal: clears the dialog and removes .open", () => {
  const { modalEl, dialogEl, createNode } = mount();
  modalEl.classList.add("open");
  dialogEl.appendChild(createNode("div", "stale", "leftover"));

  renderBranchCleanupModal({
    modalEl,
    dialogEl,
    windowId: null,
    state: null,
    selectedEntries: [],
    createNode,
    resultSummary,
    mergeTargetText,
    riskLabels,
    onCancel: () => {},
    onSubmit: () => {},
    onDeleteRemoteToggle: () => {},
  });

  assert.equal(modalEl.classList.contains("open"), false);
  assert.equal(dialogEl.childNodes.length, 0);
});

test("confirm stage: renders title, list, Cancel and Run cleanup buttons", () => {
  const { modalEl, dialogEl, createNode } = mount();
  const state = makeState();
  const entries = [makeEntry("feature/a"), makeEntry("feature/b")];
  let cancelCalls = 0;
  let submitCalls = 0;

  renderBranchCleanupModal({
    modalEl,
    dialogEl,
    windowId: "win-1",
    state,
    selectedEntries: entries,
    createNode,
    resultSummary,
    mergeTargetText,
    riskLabels,
    onCancel: () => {
      cancelCalls += 1;
    },
    onSubmit: () => {
      submitCalls += 1;
    },
    onDeleteRemoteToggle: () => {},
  });

  assert.equal(modalEl.classList.contains("open"), true);
  const heading = dialogEl.querySelector("h2");
  assert.ok(heading, "expected an <h2> heading inside the dialog");
  assert.equal(heading.textContent, "Clean up branches");

  const items = dialogEl.querySelectorAll(".branch-cleanup-item");
  assert.equal(items.length, entries.length);

  const buttons = Array.from(dialogEl.querySelectorAll("button")).map(
    (b) => b.textContent,
  );
  assert.deepEqual(buttons.sort(), ["Cancel", "Run cleanup"]);

  const cancelBtn = Array.from(dialogEl.querySelectorAll("button")).find(
    (b) => b.textContent === "Cancel",
  );
  const submitBtn = Array.from(dialogEl.querySelectorAll("button")).find(
    (b) => b.textContent === "Run cleanup",
  );
  cancelBtn.click();
  submitBtn.click();
  assert.equal(cancelCalls, 1);
  assert.equal(submitCalls, 1);
});

test("confirm stage: deleteRemote toggle appears when any entry has upstream", () => {
  const { modalEl, dialogEl, createNode } = mount();
  const state = makeState();
  const entries = [
    makeEntry("feature/x", {
      cleanup: {
        availability: "risky",
        upstream: "origin/feature/x",
        risks: ["remote_tracking"],
      },
    }),
  ];
  let toggledTo = null;

  renderBranchCleanupModal({
    modalEl,
    dialogEl,
    windowId: "win-1",
    state,
    selectedEntries: entries,
    createNode,
    resultSummary,
    mergeTargetText,
    riskLabels,
    onCancel: () => {},
    onSubmit: () => {},
    onDeleteRemoteToggle: (checked) => {
      toggledTo = checked;
    },
  });

  const checkbox = dialogEl.querySelector(
    ".branch-cleanup-toggle-row input[type='checkbox']",
  );
  assert.ok(
    checkbox,
    "expected the 'Also delete matching remote branches' toggle to render",
  );
  assert.equal(checkbox.checked, false);
  checkbox.checked = true;
  checkbox.dispatchEvent(new dialogEl.ownerDocument.defaultView.Event("change"));
  assert.equal(toggledTo, true);
});

test("running stage: renders running heading and copy", () => {
  const { modalEl, dialogEl, createNode } = mount();
  const state = makeState({ cleanupModal: { open: true, stage: "running" } });

  renderBranchCleanupModal({
    modalEl,
    dialogEl,
    windowId: "win-1",
    state,
    selectedEntries: [makeEntry("feature/a"), makeEntry("feature/b")],
    createNode,
    resultSummary,
    mergeTargetText,
    riskLabels,
    onCancel: () => {},
    onSubmit: () => {},
    onDeleteRemoteToggle: () => {},
  });

  const heading = dialogEl.querySelector("h2");
  assert.equal(heading.textContent, "Cleaning up branches");
  const running = dialogEl.querySelector(".branch-cleanup-running");
  assert.ok(running);
  assert.match(running.textContent, /Running cleanup for 2 branches\./);
});

test("result stage: renders summary, per-result rows and Close button", () => {
  const { modalEl, dialogEl, createNode } = mount();
  const state = makeState({
    cleanupModal: {
      open: true,
      stage: "result",
      results: [
        {
          branch: "feature/a",
          status: "success",
          message: "Deleted",
          execution_branch: null,
        },
        {
          branch: "feature/b",
          status: "failed",
          message: "Conflict",
          execution_branch: "feature/b@worktree",
        },
      ],
    },
  });
  let closed = 0;

  renderBranchCleanupModal({
    modalEl,
    dialogEl,
    windowId: "win-1",
    state,
    selectedEntries: [],
    createNode,
    resultSummary,
    mergeTargetText,
    riskLabels,
    onCancel: () => {
      closed += 1;
    },
    onSubmit: () => {},
    onDeleteRemoteToggle: () => {},
  });

  const heading = dialogEl.querySelector("h2");
  assert.equal(heading.textContent, "Cleanup result");
  const summary = dialogEl.querySelector(".branch-cleanup-results-summary");
  assert.match(summary.textContent, /success 1.*partial 0.*failed 1/);

  const items = dialogEl.querySelectorAll(".branch-cleanup-item");
  assert.equal(items.length, 2);
  const executionLine = Array.from(
    dialogEl.querySelectorAll(".branch-cleanup-item-copy"),
  ).find((node) => node.textContent === "Executed as feature/b@worktree");
  assert.ok(executionLine, "expected execution_branch line for feature/b");

  const buttons = Array.from(dialogEl.querySelectorAll("button"));
  assert.equal(buttons.length, 1);
  assert.equal(buttons[0].textContent, "Close");
  buttons[0].click();
  assert.equal(closed, 1);
});
