import { surfaceForWindow } from "./surface-rail.js";
import { bindControlExplanations, setControlExplanation } from "./control-explanations.js";

// The grid owns presentation only. Sessions and terminal runtimes remain shared
// with the existing window model; column changes never persist canvas geometry.
export function createAgentsSurface({ document, windowCatalogModel, getWindows, windowDisplayTitle,
  isMounted = () => true, mountTerminal, mountPreview, onLayout,
  onFocus = () => {}, onPreviewFocus = onFocus }) {
  const element = document.createElement("section");
  element.className = "agents-surface";
  element.setAttribute("aria-label", "Agents");
  element.innerHTML = `<header class="agents-toolbar"><h2>Agents</h2>
    <label>Columns <input type="number" min="1" max="4" step="1" value="2" aria-label="Agent columns"></label></header>
    <div class="agents-tabs" role="tablist" aria-label="Agent views"></div>
    <div class="agents-grid" id="agents-panel" role="tabpanel"></div><p class="agents-empty">No agents. Launch an agent from Issues.</p>`;
  const grid = element.querySelector(".agents-grid");
  const tablist = element.querySelector(".agents-tabs");
  const columns = element.querySelector("input");
  const explanations = bindControlExplanations({ document, root: element, id: "agents-control-explanation" });
  const columnsKey = "gwt:ui:agent-columns";
  try {
    const saved = document.defaultView.localStorage?.getItem(columnsKey);
    if (saved !== null && saved !== undefined) columns.value = saved;
  } catch { /* Keep the default when browser storage is unavailable. */ }
  const tiles = new Map();
  const tabs = new Map();
  const previews = new Map();
  let previewElement = null;
  let previewExplanations = null;
  let selectedId = null;
  function setTitle(node, title, name) {
    if (!node.classList.contains("agent-title")) {
      node.classList.add("agent-title");
      node.innerHTML = '<span class="agent-title__name"></span><span class="agent-title__purpose"></span>';
    }
    node.querySelector(".agent-title__name").textContent = name;
    node.querySelector(".agent-title__purpose").textContent = title.slice(name.length);
    node.setAttribute("aria-label", title);
    setControlExplanation(node, title);
  }
  function syncPreview() {
    if (!previewElement) return;
    const previewGrid = previewElement.querySelector(".agents-grid");
    for (const [id, preview] of previews) {
      if (!tiles.has(id)) { preview.cleanup?.(); preview.tile.remove(); previews.delete(id); }
    }
    for (const [id, source] of tiles) {
      let preview = previews.get(id);
      if (!preview) {
        const tile = document.createElement("article");
        tile.className = "agent-tile";
        tile.dataset.agentId = id;
        tile.tabIndex = 0;
        tile.innerHTML = `<header class="agent-tile__header"><h3 tabindex="0"></h3><span>Read-only preview</span></header>
          <div class="agent-tile__terminal terminal-root"></div>`;
        for (const event of ["pointerdown", "focusin"]) {
          tile.addEventListener(event, event => {
            // The preview moves to the other pane during activation. Do not
            // let the browser focus that moved node after focusing the xterm.
            if (event.type === "pointerdown") event.preventDefault();
            // Native events can run microtasks between listeners. Focus only
            // after the ancestor pane has activated and moved the live host.
            requestAnimationFrame(() => {
              if (!previewElement || !tiles.has(id)) return;
              if (event.type === "focusin" && event.target.closest(".agent-title")) {
                tiles.get(id).querySelector("h3").focus({ preventScroll: true });
              } else {
                onPreviewFocus(id);
              }
            });
          });
        }
        previewGrid.appendChild(tile);
        preview = { tile, cleanup: mountPreview(id, tile.querySelector(".terminal-root")) };
        previews.set(id, preview);
      }
      const title = source.querySelector("h3");
      setTitle(preview.tile.querySelector("h3"), title.textContent, title.querySelector(".agent-title__name").textContent);
      preview.tile.hidden = source.hidden;
    }
    previewGrid.classList.toggle("is-single", selectedId !== null);
    previewGrid.style.gridTemplateColumns = grid.style.gridTemplateColumns;
    previewElement.querySelector(".agents-empty").hidden = tiles.size > 0;
    previewElement.dataset.catalogVersion = element.dataset.catalogVersion;
    previewExplanations.refresh();
  }
  function setPreviewHost(host) {
    if (!host) {
      previewExplanations?.dispose();
      previewExplanations = null;
      for (const preview of previews.values()) preview.cleanup?.();
      previews.clear();
      previewElement?.remove();
      previewElement = null;
      return;
    }
    if (!previewElement) {
      previewElement = document.createElement("section");
      previewElement.className = "agents-surface agents-surface--preview";
      previewElement.setAttribute("aria-label", "Agents read-only preview");
      previewElement.innerHTML = `<header class="agents-toolbar"><h2>Agents</h2><span>Read-only preview · Select this pane to interact</span></header>
        <div class="agents-grid"></div><p class="agents-empty">No agents. Launch an agent from Issues.</p>`;
      previewExplanations = bindControlExplanations({ document, root: previewElement, id: "agents-preview-control-explanation" });
    }
    if (previewElement.parentElement !== host) host.appendChild(previewElement);
    syncPreview();
    explanations.refresh();
  }
  function updateSelection() {
    for (const [id, tile] of tiles) {
      tile.hidden = selectedId !== null && id !== selectedId;
      tile.querySelector(".agent-tile__open").hidden = tabs.has(id);
    }
    for (const [id, tab] of tabs) {
      tab.setAttribute("aria-selected", String(id === selectedId));
      tab.setAttribute("tabindex", id === selectedId ? "0" : "-1");
    }
    grid.setAttribute("aria-labelledby", tabs.get(selectedId).id);
    grid.classList.toggle("is-single", selectedId !== null);
    columns.closest("label").hidden = selectedId !== null;
    grid.style.gridTemplateColumns = `repeat(${selectedId === null ? columns.value : 1}, minmax(0, 1fr))`;
    syncPreview();
    explanations.refresh();
  }
  function select(id) {
    if (selectedId === id) return;
    selectedId = id;
    updateSelection();
    if (id !== null) onFocus(id);
    onLayout();
  }
  function closeTab(id) {
    if (id === null || !tabs.has(id)) return;
    const entry = tabs.get(id).parentElement;
    const restoreFocus = id === selectedId || entry.contains(document.activeElement);
    entry.remove();
    tabs.delete(id);
    if (selectedId === id) selectedId = null;
    updateSelection();
    if (restoreFocus) tabs.get(selectedId).focus();
    onLayout();
  }
  function openTab(id) {
    const tile = tiles.get(id);
    if (!tile) return false;
    if (!tabs.has(id)) {
      const title = tile.querySelector("h3");
      addTab(id, title.textContent, title.querySelector(".agent-title__name").textContent);
    }
    select(id);
    tabs.get(id).focus();
    return true;
  }
  function addTab(id, label, name = "") {
    const entry = document.createElement("div");
    entry.className = "agents-tab";
    const tab = document.createElement("button");
    tab.type = "button";
    tab.id = id === null ? "agents-tab-all" : `agents-tab-${id}`;
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-controls", grid.id);
    if (id === null) tab.textContent = label;
    else setTitle(tab, label, name);
    tab.addEventListener("click", () => select(id));
    tab.addEventListener("keydown", event => {
      if (event.key === "Delete" && id !== null) {
        event.preventDefault();
        closeTab(id);
        return;
      }
      const ids = [...tabs.keys()];
      const index = ids.indexOf(id);
      const next = { ArrowLeft: (index + ids.length - 1) % ids.length,
        ArrowRight: (index + 1) % ids.length, Home: 0, End: ids.length - 1 }[event.key];
      if (next === undefined) return;
      event.preventDefault();
      select(ids[next]);
      tabs.get(ids[next]).focus();
    });
    tabs.set(id, tab);
    entry.appendChild(tab);
    if (id !== null) {
      const close = document.createElement("button");
      close.type = "button";
      close.className = "icon-button agents-tab-close";
      close.textContent = "×";
      close.setAttribute("aria-label", `Close ${label} tab`);
      close.title = "Close tab (agent keeps running)";
      close.addEventListener("click", () => closeTab(id));
      entry.appendChild(close);
    }
    tablist.appendChild(entry);
    return tab;
  }
  addTab(null, "All agents");
  function setColumns() {
    const value = Number(columns.value);
    const count = Math.max(1, Math.min(4, Number.isFinite(value) ? Math.trunc(value) : 1));
    columns.value = String(count);
    updateSelection();
    onLayout();
  }
  columns.addEventListener("input", () => {
    setColumns();
    try { document.defaultView.localStorage?.setItem(columnsKey, columns.value); }
    catch { /* The in-memory preference remains usable without storage. */ }
  });
  setColumns();

  function sync() {
    if (!isMounted()) return;
    const catalog = windowCatalogModel.read().catalog;
    const windows = getWindows(catalog);
    const agents = windows.filter((data) => surfaceForWindow(data) === "agents");
    const ids = new Set(agents.map((data) => data.id));
    let changed = false;
    let restoreFocus = false;
    for (const [id, tile] of tiles) {
      if (!ids.has(id)) {
        restoreFocus ||= tile.contains(document.activeElement) || !!tabs.get(id)?.parentElement.contains(document.activeElement);
        tile.remove(); tiles.delete(id);
        tabs.get(id)?.parentElement.remove(); tabs.delete(id);
        changed = true;
      }
    }
    for (const data of agents) {
      let tile = tiles.get(data.id);
      if (!tile) {
        tile = document.createElement("article");
        tile.className = "agent-tile";
        tile.dataset.agentId = data.id;
        tile.addEventListener("pointerdown", () => onFocus(data.id));
        tile.addEventListener("focusin", () => onFocus(data.id));
        tile.innerHTML = `<header class="agent-tile__header"><h3 tabindex="0"></h3><button type="button" class="agent-tile__open">Open tab</button><span class="agent-tile__mode">Interactive</span></header>
          <div class="agent-tile__terminal terminal-root"></div>`;
        tile.querySelector(".agent-tile__open").addEventListener("click", () => openTab(data.id));
        tiles.set(data.id, tile);
        grid.appendChild(tile);
        addTab(data.id, "");
        changed = true;
      }
      const title = windowDisplayTitle(data);
      const name = ["agent", "claude", "codex"].includes(data.preset)
        ? String(data.agent_name || "").trim() || "Naming…" : "";
      const open = tile.querySelector(".agent-tile__open");
      setTitle(tile.querySelector("h3"), title, name);
      open.setAttribute("aria-label", `Open ${title} tab`);
      const tab = tabs.get(data.id);
      if (tab) {
        setTitle(tab, title, name);
        tab.parentElement.querySelector(".agents-tab-close").setAttribute("aria-label", `Close ${title} tab`);
      }
      const shell = data.preset === "shell";
      const disabled = (!data.session_id && !shell) || ["stopped", "error", "interrupted"].includes(data.status);
      tile.querySelector(".agent-tile__mode").textContent = disabled ? "Input unavailable" : "Interactive";
      mountTerminal(data.id, tile.querySelector(".terminal-root"));
    }
    if (selectedId !== null && !ids.has(selectedId)) selectedId = null;
    element.dataset.catalogVersion = String(catalog.version);
    updateSelection();
    if (restoreFocus) tabs.get(selectedId).focus();
    element.querySelector(".agents-empty").hidden = agents.length > 0;
    if (changed) onLayout();
  }
  const unsubscribe = windowCatalogModel.subscribe(state => state.catalog, () => sync());
  return { element, sync, setPreviewHost, openTab,
    dispose() { unsubscribe(); explanations.dispose(); setPreviewHost(null); },
    contains: (id) => tiles.has(id),
    isVisible: (id) => tiles.has(id) && !tiles.get(id).hidden,
    reveal: (id) => { if (tiles.has(id) && (!tabs.has(id) || selectedId !== null)) openTab(id); },
  };
}
