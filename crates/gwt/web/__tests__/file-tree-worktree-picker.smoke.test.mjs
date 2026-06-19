// SPEC-2009 amendment Phase 1 — File Tree v2 contract smoke test.
//
// Verifies that the picker modal landing pad is present in index.html and
// exposes the structural hooks that app.js relies on (the modal shell, the
// classed backdrop, and the data attributes that the modal flow toggles).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseHTML } from "linkedom";
import { createFileTreeSurface } from "../file-tree-surface.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const INDEX_HTML = path.resolve(here, "..", "index.html");

async function loadIndex() {
  const html = await readFile(INDEX_HTML, "utf8");
  return parseHTML(html);
}

test("index.html ships the SPEC-2009 worktree picker modal landing pad", async () => {
  const { document } = await loadIndex();
  const modal = document.getElementById("file-tree-worktree-picker-modal");
  assert.ok(modal, "worktree picker modal element must exist for app.js to mount");
  assert.ok(
    modal.classList.contains("modal-backdrop"),
    "picker modal must adopt the shared modal-backdrop class so existing CSS hits",
  );
  assert.equal(
    modal.getAttribute("aria-hidden"),
    "true",
    "picker modal starts hidden until app.js opens it on File Tree window creation",
  );
  const shell = modal.querySelector(".modal-shell");
  assert.ok(shell, "picker modal must wrap a .modal-shell so app.js can mount content");
  assert.equal(
    shell.getAttribute("role"),
    "dialog",
    "picker modal shell must announce role=dialog for assistive tech",
  );
  assert.equal(
    shell.getAttribute("aria-modal"),
    "true",
    "picker modal shell must trap focus via aria-modal",
  );
});

test("index.html keeps the legacy branch cleanup modal landing pad intact", async () => {
  // SPEC-2009 amendment must not regress existing modal contracts.
  const { document } = await loadIndex();
  assert.ok(
    document.getElementById("branch-cleanup-modal"),
    "branch cleanup modal must remain available; SPEC-2009 amendment only adds the worktree picker",
  );
});

test("File Tree split layout CSS contract ships expected selectors", async () => {
  // SPEC-2009 amendment FR-024/025/027: confirm the canonical split layout
  // classes the runtime depends on are present in the styles bundle. The
  // CSS file ships with the gwt binary, so a missing rule would silently
  // collapse the viewer.
  const cssPath = path.resolve(here, "..", "styles", "components.css");
  const css = await readFile(cssPath, "utf8");
  for (const cls of [
    ".file-tree-split",
    ".file-tree-splitter",
    ".file-tree-viewer",
    ".file-tree-viewer-header",
    ".file-tree-viewer-body",
    ".file-tree-viewer-text",
    ".file-tree-viewer-hex",
    ".worktree-picker-row",
  ]) {
    assert.ok(
      css.includes(cls),
      `components.css must declare ${cls} so the split layout renders correctly`,
    );
  }
});

test("index.html exposes Phase 2 Discard / Conflict modal landing pads", async () => {
  // SPEC-2009 amendment Phase 2 FR-034/035: app.js mounts editor flow modals
  // into these landing pads. Their absence would silently drop the Save /
  // Discard / Overwrite / Reload affordances.
  const { document } = await loadIndex();
  for (const id of ["file-tree-discard-modal", "file-tree-conflict-modal"]) {
    const modal = document.getElementById(id);
    assert.ok(modal, `${id} landing pad must exist for the editor flow`);
    assert.ok(
      modal.classList.contains("modal-backdrop"),
      `${id} must reuse the shared modal-backdrop primitive`,
    );
    const shell = modal.querySelector(".modal-shell");
    assert.ok(shell, `${id} must include a .modal-shell so app.js can mount content`);
    assert.equal(
      shell.getAttribute("role"),
      "dialog",
      `${id} shell must announce role=dialog for assistive tech`,
    );
  }
});

test("editor CSS contract ships dirty / Save / hex cell / modal styles", async () => {
  // SPEC-2009 amendment Phase 2 FR-032/033/036/037: editor-specific
  // affordances depend on these selectors. The contract guard runs from CI
  // so regressions surface before they hit production.
  const cssPath = path.resolve(here, "..", "styles", "components.css");
  const css = await readFile(cssPath, "utf8");
  for (const cls of [
    ".file-tree-viewer-dirty",
    ".file-tree-viewer-readonly",
    ".file-tree-viewer-saved",
    ".file-tree-viewer-save[disabled]",
    "textarea.file-tree-viewer-editor",
    ".file-tree-hex-cell",
    ".discard-modal-shell",
    ".conflict-modal-shell",
  ]) {
    assert.ok(
      css.includes(cls),
      `components.css must declare ${cls} so the editor flow renders correctly`,
    );
  }
});

function makeEl(tag, options = {}, children = []) {
  const element = document.createElement(tag);
  if (options.className) element.className = options.className;
  if (Object.hasOwn(options, "text")) element.textContent = options.text;
  for (const [key, value] of Object.entries(options.attrs || {})) {
    element.setAttribute(key, value);
  }
  for (const [key, value] of Object.entries(options.dataset || {})) {
    element.dataset[key] = value;
  }
  for (const child of children) {
    if (child) element.appendChild(child);
  }
  return element;
}

function clearChildren(element) {
  while (element.firstChild) element.removeChild(element.firstChild);
}

test("File Tree waits for worktree selection confirmation before loading the root", () => {
  const { document: fixtureDocument, window } = parseHTML(`<!doctype html><body>
    <div id="file-tree-worktree-picker-modal" class="modal-backdrop" aria-hidden="true">
      <div class="modal-shell" role="dialog" aria-modal="true"></div>
    </div>
    <main id="file-tree-window"></main>
  </body>`);
  const previousDocument = globalThis.document;
  const previousEvent = globalThis.Event;
  const previousCss = globalThis.CSS;
  globalThis.document = fixtureDocument;
  globalThis.Event = window.Event;
  globalThis.CSS = { escape: (value) => String(value) };
  try {
    const sent = [];
    const windowMap = new Map();
    const body = fixtureDocument.getElementById("file-tree-window");
    windowMap.set("ft-1", body);
    const surface = createFileTreeSurface({
      send: (message) => sent.push(message),
      makeEl,
      clearChildren,
      focusWindowLocally: () => {},
      sendWindowFocus: () => {},
      windowMap,
    });

    surface.mountFileTreeWindow({ id: "ft-1" }, body);
    surface.applyFileTreeReceiveEvent({
      kind: "file_tree_worktrees",
      id: "ft-1",
      entries: [
        {
          id: "wt-develop",
          kind: "workspace",
          path: "/repo/develop",
          label: "develop",
          branch: "develop",
          is_active: false,
        },
      ],
    });

    fixtureDocument.querySelector("[data-worktree-id='wt-develop']").click();

    assert.deepEqual(
      sent.map((message) => message.kind),
      ["list_file_tree_worktrees", "select_file_tree_worktree"],
      "root load must wait until file_tree_worktree_selected confirms the backend root",
    );

    surface.applyFileTreeReceiveEvent({
      kind: "file_tree_worktree_selected",
      id: "ft-1",
      worktree_id: "wt-develop",
    });

    assert.equal(
      fixtureDocument.querySelector(".file-tree-worktree-trigger").textContent,
      "develop",
      "confirmed selection should update the toolbar worktree label",
    );

    assert.deepEqual(
      sent.map((message) => message.kind),
      ["list_file_tree_worktrees", "select_file_tree_worktree", "load_file_tree"],
      "confirmed selection should trigger exactly one root load",
    );
  } finally {
    globalThis.document = previousDocument;
    globalThis.Event = previousEvent;
    globalThis.CSS = previousCss;
  }
});
