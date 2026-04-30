// SPEC-1934 US-6 DOM smoke test for the Migration confirmation modal renderer.
// Mounts the modal markup from index.html in a linkedom Document and asserts
// the renderer paints the correct body for each stage (confirm / running /
// error). Mirrors branch-cleanup.smoke.test.mjs.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

import { renderMigrationModal } from "../migration-modal.js";

const here = dirname(fileURLToPath(import.meta.url));
const indexHtmlPath = resolve(here, "..", "index.html");
const indexHtml = readFileSync(indexHtmlPath, "utf8");

function mount() {
  const { document } = parseHTML(indexHtml);
  const modalEl = document.getElementById("migration-modal");
  assert.ok(modalEl, "expected #migration-modal in index.html");
  const dialogEl = modalEl.querySelector(".modal-shell");
  assert.ok(
    dialogEl,
    "expected #migration-modal to contain a .modal-shell child",
  );
  const createNode = (tagName, className, textContent) => {
    const node = document.createElement(tagName);
    if (className) node.className = className;
    if (textContent !== undefined) node.textContent = textContent;
    return node;
  };
  return { document, modalEl, dialogEl, createNode };
}

function makeState(overrides = {}) {
  return {
    migrationModal: {
      open: true,
      stage: "confirm",
      tabId: "tab-1",
      projectRoot: "/Users/akiojin/Workbench/llmlb",
      branch: "develop",
      hasDirty: false,
      hasLocked: false,
      hasSubmodules: false,
      phase: "confirm",
      percent: 0,
      message: "",
      recovery: "",
      ...overrides,
    },
  };
}

const noop = () => {};

test("closed migration modal: clears dialog and removes .open", () => {
  const { modalEl, dialogEl, createNode } = mount();
  modalEl.classList.add("open");
  dialogEl.appendChild(createNode("div", "stale", "leftover"));

  renderMigrationModal({
    modalEl,
    dialogEl,
    state: { migrationModal: { open: false } },
    createNode,
    onMigrate: noop,
    onSkip: noop,
    onQuit: noop,
  });

  assert.equal(modalEl.classList.contains("open"), false);
  assert.equal(dialogEl.firstChild, null);
});

test("confirm stage paints title, project root, and three action buttons", () => {
  const { modalEl, dialogEl, createNode } = mount();
  renderMigrationModal({
    modalEl,
    dialogEl,
    state: makeState(),
    createNode,
    onMigrate: noop,
    onSkip: noop,
    onQuit: noop,
  });

  assert.equal(modalEl.classList.contains("open"), true);
  assert.match(
    dialogEl.textContent,
    /Migrate to Bare \+ Worktree layout/,
    "header should be present",
  );
  assert.match(dialogEl.textContent, /llmlb/);
  const buttons = dialogEl.querySelectorAll("button");
  assert.equal(buttons.length, 3, "Quit / Skip / Migrate buttons");
  const labels = Array.from(buttons).map((b) => b.textContent);
  assert.deepEqual(labels, ["Quit", "Skip", "Migrate"]);
});

test("confirm stage disables Migrate when locked worktrees are present", () => {
  const { modalEl, dialogEl, createNode } = mount();
  renderMigrationModal({
    modalEl,
    dialogEl,
    state: makeState({ hasLocked: true }),
    createNode,
    onMigrate: noop,
    onSkip: noop,
    onQuit: noop,
  });

  const buttons = Array.from(dialogEl.querySelectorAll("button"));
  const migrate = buttons.find((b) => b.textContent === "Migrate");
  assert.ok(migrate, "Migrate button must exist");
  assert.equal(migrate.disabled, true);
});

test("running stage shows phase label and progress bar", () => {
  const { modalEl, dialogEl, createNode } = mount();
  renderMigrationModal({
    modalEl,
    dialogEl,
    state: makeState({ stage: "running", phase: "bareify", percent: 42 }),
    createNode,
    onMigrate: noop,
    onSkip: noop,
    onQuit: noop,
  });

  assert.match(dialogEl.textContent, /Migrating repository/);
  assert.match(dialogEl.textContent, /Building bare repository/);
  const progress = dialogEl.querySelector("progress");
  assert.ok(progress, "progress bar present");
  assert.equal(Number(progress.getAttribute("value")), 42);
});

test("error stage exposes the failing phase, message, and recovery hint", () => {
  const { modalEl, dialogEl, createNode } = mount();
  renderMigrationModal({
    modalEl,
    dialogEl,
    state: makeState({
      stage: "error",
      phase: "worktrees",
      message: "git worktree add failed",
      recovery: "rolled_back",
    }),
    createNode,
    onMigrate: noop,
    onSkip: noop,
    onQuit: noop,
  });

  assert.match(dialogEl.textContent, /Migration failed/);
  assert.match(dialogEl.textContent, /Setting up worktrees/);
  assert.match(dialogEl.textContent, /git worktree add failed/);
  assert.match(dialogEl.textContent, /rolled back/);
  const buttons = dialogEl.querySelectorAll("button");
  assert.equal(buttons.length, 1, "only Close button");
  assert.equal(buttons[0].textContent, "Close");
});
