// SPEC-1939 Phase 12 / T-IDX-106 — Settings.Index tab renderer.
//
// Pure rendering: receives the aggregated payload + a `send` transport
// (so unit tests can stub WebSocket dispatch) and rebuilds the panel DOM
// in place. Each cell carries `last_repair_at`, `document_count`, `reason`
// and a per-cell Rebuild button that emits `rebuild_index_cell` with
// `(project_root, scope, worktree_hash?)`.

const REPO_SHARED_SCOPES = ["issues", "specs", "lessons"];
const PER_WORKTREE_SCOPES = ["files", "files-docs"];
const ALL_SCOPES = [...REPO_SHARED_SCOPES, ...PER_WORKTREE_SCOPES];

function clear(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
}

function formatTimestamp(value) {
  if (!value) return "—";
  return value;
}

function emptyMessage(doc) {
  const note = doc.createElement("p");
  note.className = "settings-help";
  note.dataset.role = "index-settings-empty";
  note.textContent =
    "Project index status is not available yet. Open a project and wait for the bootstrap to complete.";
  return note;
}

function buildHealthCell(doc, scope, worktreeHash, view, send, projectRoot) {
  const td = doc.createElement("td");
  td.className = "settings-index-cell";
  if (worktreeHash) td.dataset.worktreeHash = worktreeHash;
  td.dataset.scope = scope;
  if (!view) {
    td.classList.add("settings-index-cell-empty");
    td.textContent = "—";
    return td;
  }
  td.classList.add(view.healthy ? "ready" : "unhealthy");

  const status = doc.createElement("div");
  status.className = "settings-index-cell-status";
  status.textContent = view.healthy ? "ready" : view.reason || "unhealthy";
  td.appendChild(status);

  const meta = doc.createElement("div");
  meta.className = "settings-index-cell-meta";
  const docs = doc.createElement("span");
  docs.textContent = `${view.document_count ?? 0} docs`;
  meta.appendChild(docs);
  if (view.last_repair_at) {
    const repaired = doc.createElement("span");
    repaired.dataset.role = "last-repair-at";
    repaired.textContent = `last ${formatTimestamp(view.last_repair_at)}`;
    meta.appendChild(repaired);
  }
  td.appendChild(meta);

  const rebuild = doc.createElement("button");
  rebuild.type = "button";
  rebuild.className = "settings-index-rebuild";
  rebuild.dataset.action = "rebuild-cell";
  rebuild.dataset.scope = scope;
  if (worktreeHash) rebuild.dataset.worktreeHash = worktreeHash;
  rebuild.textContent = "Rebuild";
  rebuild.addEventListener("click", () => {
    const message = {
      kind: "rebuild_index_cell",
      project_root: projectRoot,
      scope,
    };
    if (worktreeHash) message.worktree_hash = worktreeHash;
    send(message);
  });
  td.appendChild(rebuild);
  return td;
}

function appendRebuildAllScopeButton(doc, headerCell, scope, send, projectRoot) {
  const button = doc.createElement("button");
  button.type = "button";
  button.className = "settings-index-rebuild-all";
  button.dataset.action = "rebuild-scope";
  button.dataset.scope = scope;
  button.textContent = "Rebuild all";
  button.addEventListener("click", () => {
    send({ kind: "rebuild_index_cell", project_root: projectRoot, scope });
  });
  headerCell.appendChild(button);
}

export function renderIndexSettingsPanel(options) {
  const { panel, status, projectRoot, send, document: doc } = options;
  const ownerDoc = doc || (panel && panel.ownerDocument) || globalThis.document;
  if (!panel) return;
  clear(panel);

  if (!projectRoot) {
    panel.appendChild(emptyMessage(ownerDoc));
    return;
  }

  const scopes = (status && status.scopes) || {};
  const worktrees = (status && status.worktrees) || {};
  const worktreeHashes = Object.keys(worktrees).sort();

  if (
    worktreeHashes.length === 0
    && !scopes.issues
    && !scopes.specs
    && (!scopes.files || Object.keys(scopes.files).length === 0)
    && (!scopes["files-docs"] || Object.keys(scopes["files-docs"]).length === 0)
  ) {
    panel.appendChild(emptyMessage(ownerDoc));
    return;
  }

  const heading = ownerDoc.createElement("h3");
  heading.className = "settings-section-heading";
  heading.textContent = "Project index health";
  panel.appendChild(heading);

  const table = ownerDoc.createElement("table");
  table.className = "settings-index-table";
  table.dataset.role = "index-settings-table";

  const thead = ownerDoc.createElement("thead");
  const headRow = ownerDoc.createElement("tr");
  const scopeHeader = ownerDoc.createElement("th");
  scopeHeader.setAttribute("scope", "col");
  scopeHeader.textContent = "Scope";
  headRow.appendChild(scopeHeader);
  if (worktreeHashes.length === 0) {
    const repoHeader = ownerDoc.createElement("th");
    repoHeader.setAttribute("scope", "col");
    repoHeader.textContent = "Repo";
    headRow.appendChild(repoHeader);
  } else {
    for (const wtHash of worktreeHashes) {
      const meta = worktrees[wtHash] || {};
      const th = ownerDoc.createElement("th");
      th.setAttribute("scope", "col");
      th.dataset.worktreeHash = wtHash;
      th.textContent = meta.branch || wtHash;
      headRow.appendChild(th);
    }
  }
  thead.appendChild(headRow);
  table.appendChild(thead);

  const tbody = ownerDoc.createElement("tbody");
  for (const scope of ALL_SCOPES) {
    const tr = ownerDoc.createElement("tr");
    tr.dataset.scope = scope;
    const scopeCell = ownerDoc.createElement("th");
    scopeCell.setAttribute("scope", "row");
    scopeCell.textContent = scope;
    if (PER_WORKTREE_SCOPES.includes(scope) && worktreeHashes.length > 0) {
      appendRebuildAllScopeButton(ownerDoc, scopeCell, scope, send, projectRoot);
    } else if (REPO_SHARED_SCOPES.includes(scope)) {
      appendRebuildAllScopeButton(ownerDoc, scopeCell, scope, send, projectRoot);
    }
    tr.appendChild(scopeCell);

    if (REPO_SHARED_SCOPES.includes(scope)) {
      const view = scopes[scope] || null;
      const cell = buildHealthCell(ownerDoc, scope, null, view, send, projectRoot);
      cell.colSpan = Math.max(worktreeHashes.length || 1, 1);
      tr.appendChild(cell);
    } else {
      const perScope = scopes[scope] || {};
      if (worktreeHashes.length === 0) {
        const cell = buildHealthCell(ownerDoc, scope, null, null, send, projectRoot);
        tr.appendChild(cell);
      } else {
        for (const wtHash of worktreeHashes) {
          const view = perScope[wtHash] || null;
          tr.appendChild(buildHealthCell(ownerDoc, scope, wtHash, view, send, projectRoot));
        }
      }
    }
    tbody.appendChild(tr);
  }
  table.appendChild(tbody);
  panel.appendChild(table);
}
