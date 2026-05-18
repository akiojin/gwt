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
