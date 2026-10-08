import { surfaceForWindow } from "./surface-rail.js";

// The grid owns presentation only. Sessions and terminal runtimes remain shared
// with the existing window model; column changes never persist canvas geometry.
export function createAgentsSurface({ document, mountTerminal, mountPreview, onLayout,
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
  const tiles = new Map();
  const tabs = new Map();
  const previews = new Map();
  let previewElement = null;
  let selectedId = null;
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
        tile.innerHTML = `<header class="agent-tile__header"><h3></h3><span>Read-only preview</span></header>
          <div class="agent-tile__terminal terminal-root"></div>`;
        for (const event of ["pointerdown", "focusin"]) {
          tile.addEventListener(event, event => {
            // The preview moves to the other pane during activation. Do not
            // let the browser focus that moved node after focusing the xterm.
            if (event.type === "pointerdown") event.preventDefault();
            // Native events can run microtasks between listeners. Focus only
            // after the ancestor pane has activated and moved the live host.
            requestAnimationFrame(() => {
              if (previewElement && tiles.has(id)) onPreviewFocus(id);
            });
          });
        }
        previewGrid.appendChild(tile);
        preview = { tile, cleanup: mountPreview(id, tile.querySelector(".terminal-root")) };
        previews.set(id, preview);
      }
      preview.tile.querySelector("h3").textContent = source.querySelector("h3").textContent;
      preview.tile.hidden = source.hidden;
    }
    previewGrid.classList.toggle("is-single", selectedId !== null);
    previewGrid.style.gridTemplateColumns = grid.style.gridTemplateColumns;
    previewElement.querySelector(".agents-empty").hidden = tiles.size > 0;
  }
  function setPreviewHost(host) {
    if (!host) {
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
    }
    if (previewElement.parentElement !== host) host.appendChild(previewElement);
    syncPreview();
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
    if (!tile) return;
    if (!tabs.has(id)) {
      addTab(id, tile.querySelector("h3").textContent);
    }
    select(id);
    tabs.get(id).focus();
  }
  function addTab(id, label) {
    const entry = document.createElement("div");
    entry.className = "agents-tab";
    const tab = document.createElement("button");
    tab.type = "button";
    tab.id = id === null ? "agents-tab-all" : `agents-tab-${id}`;
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-controls", grid.id);
    tab.textContent = label;
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
    const count = Math.max(1, Math.min(4, Number.parseInt(columns.value, 10) || 1));
    columns.value = String(count);
    updateSelection();
    onLayout();
  }
  columns.addEventListener("input", setColumns);
  setColumns();

  function sync(windows) {
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
        tile.innerHTML = `<header class="agent-tile__header"><h3></h3><button type="button" class="agent-tile__open">Open tab</button><span>Interactive</span></header>
          <div class="agent-tile__terminal terminal-root"></div>`;
        tile.querySelector(".agent-tile__open").addEventListener("click", () => openTab(data.id));
        tiles.set(data.id, tile);
        grid.appendChild(tile);
        addTab(data.id, "");
        changed = true;
      }
      const title = data.dynamic_title || data.title || data.id;
      const open = tile.querySelector(".agent-tile__open");
      tile.querySelector("h3").textContent = title;
      open.setAttribute("aria-label", `Open ${title} tab`);
      const tab = tabs.get(data.id);
      if (tab) {
        tab.textContent = title;
        tab.parentElement.querySelector(".agents-tab-close").setAttribute("aria-label", `Close ${title} tab`);
      }
      const disabled = !data.session_id || ["stopped", "error", "interrupted"].includes(data.status);
      tile.querySelector(".agent-tile__header span").textContent = disabled ? "Input unavailable" : "Interactive";
      mountTerminal(data.id, tile.querySelector(".terminal-root"));
    }
    if (selectedId !== null && !ids.has(selectedId)) selectedId = null;
    updateSelection();
    if (restoreFocus) tabs.get(selectedId).focus();
    element.querySelector(".agents-empty").hidden = agents.length > 0;
    if (changed) onLayout();
  }
  return { element, sync, setPreviewHost, contains: (id) => tiles.has(id),
    isVisible: (id) => tiles.has(id) && !tiles.get(id).hidden,
    reveal: (id) => { if (tiles.has(id) && (!tabs.has(id) || selectedId !== null)) openTab(id); },
  };
}
