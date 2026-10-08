// SPEC-1934 US-8 DOM smoke test for the Clone Project modal renderer.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

import { renderProjectCloneModal, createOpenProjectPathDialog } from "../project-clone-modal.js";

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

test("manual open retains its path on failure and closes only after open success", () => {
  const { modalEl } = mount();
  const document = modalEl.ownerDocument;
  const sent = [];
  const dialog = createOpenProjectPathDialog(document, { onChoose: noop, onOpen: (path, request_id) => sent.push({ path, request_id }) });
  document.body.append(dialog.modal);
  dialog.open();
  const input = dialog.modal.querySelector('[data-open-project-path]');
  input.value = '/missing/project';
  input.dispatchEvent(new document.defaultView.Event('input'));
  dialog.modal.querySelector('[data-open-project-submit]').click();
  assert.equal(sent[0].path, '/missing/project');
  assert.ok(sent[0].request_id, 'each submission must carry a correlation ID');
  assert.equal(dialog.modal.classList.contains('open'), true, 'submission must keep feedback visible');
  dialog.receive({ kind: 'project_opened', request_id: 'another-client', title: 'Other project' });
  dialog.receive({ kind: 'project_open_error', message: 'Another client failed' });
  assert.equal(dialog.modal.classList.contains('open'), true);
  assert.equal(dialog.modal.querySelector('[role="status"]').textContent, 'Opening project…');
  dialog.receive({ kind: 'project_open_error', request_id: sent[0].request_id, message: 'Folder does not exist' });
  assert.equal(input.value, '/missing/project');
  assert.equal(dialog.modal.querySelector('[role="status"]').textContent, 'Folder does not exist');
  input.value = '/valid/project';
  input.dispatchEvent(new document.defaultView.Event('input'));
  dialog.modal.querySelector('[data-open-project-submit]').click();
  assert.notEqual(sent[1].request_id, sent[0].request_id);
  dialog.receive({ kind: 'project_opened', request_id: sent[0].request_id, title: 'Superseded' });
  assert.equal(dialog.modal.classList.contains('open'), true);
  dialog.receive({ kind: 'project_opened', request_id: sent[1].request_id, project_key: '0123456789abcdef', title: 'Project' });
  assert.equal(dialog.modal.classList.contains('open'), false);
  dialog.receive({ kind: 'project_open_error', message: 'Another client failed' });
  assert.equal(dialog.modal.classList.contains('open'), false, 'unrelated results must not reopen the dialog');
  dialog.waitForOpen('/native/project', 'picker:42');
  dialog.hide();
  dialog.receive({ kind: 'project_open_error', request_id: 'picker:42', message: 'Late result after dismissal' });
  assert.equal(dialog.modal.classList.contains('open'), false, 'dismissal clears the pending request');
});

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
