// SPEC-1934 US-8 DOM smoke test for the Clone Project modal renderer.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

import { renderProjectCloneModal, createFolderPickerController, createOpenProjectPathDialog } from "../project-clone-modal.js";

test("folder picker callbacks read immutable authority and selected data in order", () => {
  const changes = [], selected = [], sends = [];
  const picker = createFolderPickerController({
    send: event => sends.push(event),
    onChange: (pending, message) => changes.push({ pending, message }),
    onSelected: event => selected.push({ event, pending: changes.at(-1).pending }),
  });
  assert.equal(changes.length, 0, "mount does not notify existing callbacks");
  picker.begin("clone_parent");
  const pending = changes.at(-1).pending;
  assert.ok(Object.isFrozen(pending), "observers cannot mutate request authority");
  picker.receive({ kind: "picker_started", purpose: "clone_parent", request_id: 7 });
  assert.equal(changes.length, 1, "request acknowledgement does not rebuild the form");
  assert.equal(pending.request_id, null, "previous snapshots stay unchanged");
  const event = { kind: "picker_selected", purpose: "clone_parent", request_id: 7, path: "/projects" };
  picker.receive(event);
  event.path = "/mutated";
  assert.equal(selected[0].event.path, "/projects");
  assert.ok(Object.isFrozen(selected[0].event));
  assert.equal(selected[0].pending, null, "clear busy state before delivering the destination");
  assert.deepEqual(sends, [{ kind: "select_clone_project_parent" }]);
  picker.dispose();
});

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
  const snapshot = Object.freeze(state());
  let draftUrl;

  renderProjectCloneModal({
    modalEl,
    dialogEl,
    state: snapshot,
    createNode,
    onClose: noop,
    onModeChange: noop,
    onUrlChange: value => { draftUrl = value; },
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
  const input = dialogEl.querySelector("#clone-project-url-input");
  input.value = "";
  input.dispatchEvent(new modalEl.ownerDocument.defaultView.Event("input"));
  assert.equal(draftUrl, "");
  assert.equal(dialogEl.querySelector("#clone-project-start").disabled, true);
  assert.equal(snapshot.url, "https://github.com/akiojin/gwt.git");
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


test("clone progress batches frames while drafts and modal actions stay synchronous", async t => {
  const { modalEl, dialogEl, createNode } = mount();
  const document = modalEl.ownerDocument;
  const previousDocument = globalThis.document;
  globalThis.document = document;
  t.after(() => { globalThis.document = previousDocument; });
  const source = readFileSync(resolve(here, '../project-shell-surface.js'), 'utf8')
    .replace(/from "\/(.*?)"/g, (_, name) => `from "${new URL(`../${name}`, import.meta.url).href}"`);
  const { createProjectShellSurface } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
  const frames = new Set();
  const flush = () => { for (const callback of [...frames]) { frames.delete(callback); callback(); } };
  const sent = [];
  let renders = 0;
  const createElement = document.createElement.bind(document);
  document.createElement = (...args) => { if (args[0] === 'h2') renders += 1; return createElement(...args); };
  const controller = createProjectShellSurface({
    send: message => {
      if (message.kind === 'clone_project_start') {
        assert.equal(dialogEl.querySelector('#clone-project-start').disabled, true, 'disable before send');
        assert.equal(dialogEl.getAttribute('aria-busy'), 'true');
      }
      sent.push(message);
    },
    createNode, windowCatalogModel: { subscribeFrame: () => () => {} },
    cloneProjectModal: modalEl, cloneProjectDialog: dialogEl,
    scheduleFrame: callback => { frames.add(callback); return () => frames.delete(callback); },
  });
  t.after(() => controller.dispose());
  // The existing shell installer also registers a global hotkey listener.
  const previousWindow = globalThis.window;
  globalThis.window = document.defaultView;
  t.after(() => { globalThis.window = previousWindow; });
  controller.installProjectShellChrome();
  document.getElementById('picker-clone-project').click();
  const input = dialogEl.querySelector('#clone-project-url-input');
  input.value = 'https://example.test/repo.git';
  input.dispatchEvent(new document.defaultView.Event('input'));
  flush();
  assert.equal(dialogEl.querySelector('#clone-project-url-input'), input, 'drafts never rebuild inputs');
  controller.applyCloneProjectReceiveEvent({ kind: 'clone_project_parent_selected', path: '/destination' });
  dialogEl.querySelector('#clone-project-start').click();
  assert.deepEqual(sent, [{ kind: 'clone_project_start', url: input.value, parent_path: '/destination' }]);
  const initialRenders = renders;
  flush();
  assert.equal(renders, initialRenders, 'do not repeat synchronous action rendering');
  const before = dialogEl.innerHTML;
  const progress = message => ({ kind: 'clone_project_progress', message });
  for (let n = 1; n <= 50; n += 1) controller.applyCloneProjectReceiveEvent(progress(`Receiving ${n}%`));
  assert.equal(dialogEl.innerHTML, before, 'progress waits for the frame');
  assert.equal(frames.size, 1);
  flush();
  assert.equal(renders, initialRenders + 1);
  assert.match(dialogEl.textContent, /Receiving 50%/);
  controller.applyCloneProjectReceiveEvent(progress('Pending'));
  controller.applyCloneProjectReceiveEvent({ kind: 'clone_project_parent_selected', path: '/new-destination' });
  assert.equal(dialogEl.querySelector('#clone-project-parent-input').value, '/new-destination', 'picker results stay synchronous during progress');
  const pickerRenders = renders;
  flush();
  assert.equal(renders, pickerRenders);
  controller.applyCloneProjectReceiveEvent(progress('Pending error'));
  controller.applyCloneProjectReceiveEvent({ kind: 'clone_project_error', message: 'Failure' });
  assert.match(dialogEl.textContent, /Failure/);
  const errorRenders = renders;
  flush();
  assert.equal(renders, errorRenders);
  dialogEl.querySelector('#clone-project-start').click();
  controller.applyCloneProjectReceiveEvent(progress('Pending done'));
  controller.applyCloneProjectReceiveEvent({ kind: 'clone_project_done' });
  assert.equal(modalEl.classList.contains('open'), false);
  flush();
  assert.equal(modalEl.classList.contains('open'), false);
  document.getElementById('picker-clone-project').click();
  dialogEl.querySelector('#clone-project-start').click();
  controller.applyCloneProjectReceiveEvent(progress('Dispose pending'));
  controller.dispose();
  assert.equal(frames.size, 0, 'dispose cancels pending presentation');
});
