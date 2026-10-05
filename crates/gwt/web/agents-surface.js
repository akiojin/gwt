import { surfaceForWindow } from "./surface-rail.js";

// The grid owns presentation only. Sessions and terminal runtimes remain shared
// with the existing window model; column changes never persist canvas geometry.
export function createAgentsSurface({ document, mountTerminal, onLayout, onFocus = () => {} }) {
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
  let selectedId = null;
  function updateSelection() {
    for (const [id, tile] of tiles) tile.hidden = selectedId !== null && id !== selectedId;
    for (const [id, tab] of tabs) {
      tab.setAttribute("aria-selected", String(id === selectedId));
      tab.setAttribute("tabindex", id === selectedId ? "0" : "-1");
    }
    grid.setAttribute("aria-labelledby", tabs.get(selectedId).id);
    grid.classList.toggle("is-single", selectedId !== null);
    columns.closest("label").hidden = selectedId !== null;
    grid.style.gridTemplateColumns = `repeat(${selectedId === null ? columns.value : 1}, minmax(0, 1fr))`;
  }
  function select(id) {
    if (selectedId === id) return;
    selectedId = id;
    updateSelection();
    if (id !== null) onFocus(id);
    onLayout();
  }
  function addTab(id, label) {
    const tab = document.createElement("button");
    tab.type = "button";
    tab.id = id === null ? "agents-tab-all" : `agents-tab-${id}`;
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-controls", grid.id);
    tab.textContent = label;
    tab.addEventListener("click", () => select(id));
    tab.addEventListener("keydown", event => {
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
    tablist.appendChild(tab);
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
        restoreFocus ||= tile.contains(document.activeElement) || tabs.get(id).contains(document.activeElement);
        tile.remove(); tiles.delete(id);
        tabs.get(id).remove(); tabs.delete(id);
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
        tile.innerHTML = `<header class="agent-tile__header"><h3></h3><span>Interactive</span></header>
          <div class="agent-tile__terminal terminal-root"></div>`;
        tiles.set(data.id, tile);
        grid.appendChild(tile);
        addTab(data.id, "");
        changed = true;
      }
      const title = data.dynamic_title || data.title || data.id;
      tile.querySelector("h3").textContent = title;
      tabs.get(data.id).textContent = title;
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
  return { element, sync, contains: (id) => tiles.has(id),
    isVisible: (id) => tiles.has(id) && !tiles.get(id).hidden,
    reveal: (id) => { if (tiles.has(id) && selectedId !== null) select(id); },
  };
}
