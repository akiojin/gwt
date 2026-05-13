// SPEC-1934 US-8 DOM smoke test for the Clone Project modal renderer.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

import { renderProjectCloneModal } from "../project-clone-modal.js";

const here = dirname(fileURLToPath(import.meta.url));
const indexHtml = readFileSync(resolve(here, "..", "index.html"), "utf8");

function mount() {
  const { document } = parseHTML(indexHtml);
  const modalEl = document.getElementById("clone-project-modal");
  assert.ok(modalEl, "expected #clone-project-modal in index.html");
  const dialogEl = modalEl.querySelector(".modal-shell");
  assert.ok(dialogEl, "expected #clone-project-modal to contain .modal-shell");
  const createNode = (tagName, className, textContent) => {
    const node = document.createElement(tagName);
    if (className) node.className = className;
    if (textContent !== undefined) node.textContent = textContent;
    return node;
  };
  return { modalEl, dialogEl, createNode };
}

function state(overrides = {}) {
  return {
    open: true,
    mode: "url",
    url: "https://github.com/akiojin/gwt.git",
    parentPath: "/Users/akiojin/Projects",
    query: "",
    repositories: [],
    selectedRepositoryUrl: "",
    searching: false,
    cloning: false,
    progress: "",
    error: "",
    ...overrides,
  };
}

const noop = () => {};

test("url mode renders URL and destination controls", () => {
  const { modalEl, dialogEl, createNode } = mount();

  renderProjectCloneModal({
    modalEl,
    dialogEl,
    state: state(),
    createNode,
    onClose: noop,
    onModeChange: noop,
    onUrlChange: noop,
    onParentSelect: noop,
    onSearchQueryChange: noop,
    onSearch: noop,
    onRepositorySelect: noop,
    onClone: noop,
  });

  assert.equal(modalEl.classList.contains("open"), true);
  assert.match(dialogEl.textContent, /Clone Project/);
  assert.ok(dialogEl.querySelector("#clone-project-url-input"));
  assert.ok(dialogEl.querySelector("#clone-project-parent-button"));
  assert.ok(dialogEl.querySelector("#clone-project-start"));
});

test("search mode renders candidates and selects repository URL", () => {
  const { modalEl, dialogEl, createNode } = mount();
  let selected = "";

  renderProjectCloneModal({
    modalEl,
    dialogEl,
    state: state({
      mode: "search",
      query: "gwt",
      repositories: [
        {
          full_name: "akiojin/gwt",
          description: "Git Worktree Manager",
          url: "https://github.com/akiojin/gwt",
          default_branch: "develop",
          visibility: "public",
          updated_at: "2026-05-13T00:00:00Z",
        },
      ],
    }),
    createNode,
    onClose: noop,
    onModeChange: noop,
    onUrlChange: noop,
    onParentSelect: noop,
    onSearchQueryChange: noop,
    onSearch: noop,
    onRepositorySelect: (url) => {
      selected = url;
    },
    onClone: noop,
  });

  const candidate = dialogEl.querySelector("[data-clone-repository-url]");
  assert.ok(candidate, "expected repository candidate");
  assert.match(candidate.textContent, /akiojin\/gwt/);
  candidate.dispatchEvent(new modalEl.ownerDocument.defaultView.Event("click"));
  assert.equal(selected, "https://github.com/akiojin/gwt");
});

test("progress and error states render inside the modal", () => {
  const { modalEl, dialogEl, createNode } = mount();

  renderProjectCloneModal({
    modalEl,
    dialogEl,
    state: state({
      cloning: true,
      progress: "Cloning repository...",
      error: "target already exists",
    }),
    createNode,
    onClose: noop,
    onModeChange: noop,
    onUrlChange: noop,
    onParentSelect: noop,
    onSearchQueryChange: noop,
    onSearch: noop,
    onRepositorySelect: noop,
    onClone: noop,
  });

  assert.match(dialogEl.textContent, /Cloning repository/);
  assert.match(dialogEl.textContent, /target already exists/);
  assert.equal(dialogEl.querySelector("#clone-project-start").disabled, true);
});
