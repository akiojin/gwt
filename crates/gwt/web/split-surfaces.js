import { surfaceForWindow } from "./surface-rail.js";

const SURFACES = { issues: "Issues", agents: "Agents", board: "Board", settings: "Settings" };
const PRESETS = { issues: "settings", board: "board", settings: "settings" };

// A client-local layout: move live views, never clone terminals or persist
// dimensions as canvas geometry. One or two panes share this same host;
// close returns live views to the legacy canvas during the migration.
export function createSplitSurfaces({
  document, stage, windowCatalogModel, getWindows, getElement, openSurface, onChange, onLayout,
  restoreVisibility = () => {}, onAgentsHost = () => {},
}) {
  const host = document.getElementById("split-surfaces");
  const button = document.getElementById("split-view-button");
  const area = host.parentElement;
  let opened = false;
  let split = false;
  let active = 0;
  let selections = ["issues", "board"];
  let fixedWindowId = null;
  const requested = new Map();
  const mounted = new Map();
  const panes = ["Left", "Right"].map((label, index) => {
    const pane = document.createElement("section");
    pane.className = "split-pane";
    pane.setAttribute("aria-label", `${label} pane`);
    pane.innerHTML = `
      <div class="split-pane__header">
        <label>${label}<select aria-label="${label} pane surface"></select></label>
        <span class="split-pane__focus" aria-hidden="true">Active pane</span>
      </div>
      <div class="split-pane__body"></div>
      <p class="split-pane__empty" hidden></p>`;
    const select = pane.querySelector("select");
    for (const [value, text] of Object.entries(SURFACES)) {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = text;
      select.appendChild(option);
    }
    select.addEventListener("change", () => choose(select.value, index));
    pane.addEventListener("pointerdown", () => focusPane(index));
    pane.addEventListener("focusin", () => focusPane(index));
    host.appendChild(pane);
    return pane;
  });

  function focusPane(index) {
    if (!split && index !== active) return;
    const changed = active !== index;
    active = index;
    for (const [i, pane] of panes.entries()) pane.dataset.active = String(i === active);
    if (changed && selections.every(surface => surface === "agents")) sync();
    onChange(selections[active]);
  }

  function sync() {
    const catalog = windowCatalogModel.read().catalog;
    const windows = getWindows(catalog);
    host.dataset.catalogVersion = String(catalog.version);
    for (const [surface, existingIds] of requested) {
      if (windows.some(data => matchesSurface(data, surface) && !existingIds.has(data.id))) {
        requested.delete(surface);
      }
    }
    if (!opened) return;
    const pm = selections[active] === "pm";
    if (pm && !windows.some(data => data.id === fixedWindowId && surfaceForWindow(data) === "pm")) {
      fixedWindowId = windows.find(data => data.is_pm)?.id || null;
    }
    const visiblePanes = panes.map((pane, index) => ({ pane, index }))
      .filter(({ index }) => split || index === active);
    host.hidden = false;
    host.dataset.split = String(split);
    area.classList.add("has-surface-layout");
    area.classList.toggle("is-split", split);
    button.setAttribute("aria-pressed", String(split));
    button.disabled = pm;
    button.textContent = split ? "Close split" : "Split view";
    button.title = pm ? "PM is shown in a single pane" : split ? "Keep the active surface in a single pane" : "Show two surfaces side by side";
    for (const [index, pane] of panes.entries()) {
      pane.hidden = !split && index !== active;
      pane.setAttribute("aria-label", split ? `${index === 0 ? "Left" : "Right"} pane` : "Main pane");
    }
    const desired = new Map();
    const assigned = new Map();
    // Reserve each existing pane's live view before allocating a second view.
    for (const { index, pane } of visiblePanes) {
      const data = windows.find(data => matchesSurface(data, selections[index]) &&
        getElement(data.id)?.parentElement === pane.querySelector(".split-pane__body"));
      if (data) { assigned.set(index, data); desired.set(data.id, { element: getElement(data.id), index }); }
    }
    for (const { index, pane } of visiblePanes) {
      const surface = selections[index];
      const changedSurface = pane.dataset.surface !== surface;
      pane.dataset.surface = surface;
      for (const option of pane.querySelectorAll("option")) {
        option.selected = option.value === surface;
        option.disabled = false;
        option.textContent = SURFACES[option.value];
      }
      if (surface === "agents") {
        pane.querySelector(".split-pane__empty").hidden = true;
        continue;
      }
      const data = assigned.get(index) || windows.find(data => matchesSurface(data, surface) && !desired.has(data.id));
      const empty = pane.querySelector(".split-pane__empty");
      empty.hidden = Boolean(data);
      empty.textContent = surface === "pm" ? "PM is not running. Open PM from the rail." : `Opening ${SURFACES[surface]}…`;
      if (!data && surface !== "pm" && !requested.has(surface)) {
        requested.set(surface, new Set(windows.filter(data => matchesSurface(data, surface)).map(data => data.id)));
        openSurface(surface);
      }
      if (data) {
        const element = getElement(data.id);
        if (element) {
          if (surface === "issues" && (changedSurface || element.parentElement !== pane.querySelector(".split-pane__body"))) {
            element.querySelector('[data-settings-tab="issues"]')?.click();
          }
          desired.set(data.id, { element, index });
        }
      }
    }
    const agentPanes = visiblePanes.filter(({ index }) => selections[index] === "agents").map(({ pane }) => pane);
    const livePane = selections[active] === "agents" ? panes[active] : agentPanes[0];
    const previewPane = agentPanes.find(pane => pane !== livePane);
    onAgentsHost(livePane?.querySelector(".split-pane__body") || null,
      previewPane?.querySelector(".split-pane__body") || null);
    const changed = [];
    for (const [id, element] of mounted) {
      if (desired.has(id)) continue;
      // A backend removal may already have detached this view.
      if (element.parentElement) {
        stage.appendChild(element);
        restoreVisibility(id, element);
      }
      mounted.delete(id);
      changed.push(id);
    }
    for (const [id, { element, index }] of desired) {
      const body = panes[index].querySelector(".split-pane__body");
      if (element.parentElement !== body) {
        body.appendChild(element);
        changed.push(id);
      }
      // Split selection is independent of the canvas tab group's one-active
      // rule. Restore that rule when returning this live view to the canvas.
      element.hidden = false;
      mounted.set(id, element);
    }
    focusPane(active);
    if (changed.length) onLayout(changed);
  }

  function matchesSurface(data, surface) {
    if (surface === "pm") return data.id === fixedWindowId && surfaceForWindow(data) === "pm";
    return data.preset === PRESETS[surface] && surfaceForWindow(data) !== "pm";
  }

  function choose(surface, index = active) {
    if (!opened || !SURFACES[surface] || (!split && index !== active)) return false;
    fixedWindowId = null;
    selections[index] = surface;
    active = index;
    sync();
    if (surface === "issues") {
      panes[index].querySelector('[data-settings-tab="issues"]')?.click();
    }
    return true;
  }

  function open(surface = "issues") {
    if (opened && selections[active] === "pm") return;
    if (opened && split) return;
    if (!opened) {
      selections = [SURFACES[surface] ? surface : "issues", surface === "board" ? "issues" : "board"];
      active = 0;
    }
    opened = true;
    split = true;
    sync();
  }

  function show(surface) {
    if (!SURFACES[surface]) return false;
    if (opened) return choose(surface);
    selections = [surface, surface === "board" ? "issues" : "board"];
    active = 0;
    opened = true;
    split = false;
    sync();
    return true;
  }

  function showWindow(id) {
    const windows = getWindows(windowCatalogModel.read().catalog);
    if (!windows.some(data => data.id === id && surfaceForWindow(data) === "pm")) return false;
    fixedWindowId = id;
    selections[active] = "pm";
    opened = true;
    split = false;
    sync();
    return true;
  }

  function collapse() {
    if (!opened || !split) return;
    split = false;
    sync();
    onLayout([...mounted.keys()]);
  }

  function close() {
    if (!opened) return;
    const ids = [...mounted.keys()];
    for (const [id, element] of mounted) {
      if (element.parentElement) {
        stage.appendChild(element);
        restoreVisibility(id, element);
      }
    }
    mounted.clear();
    opened = false;
    split = false;
    fixedWindowId = null;
    onAgentsHost(null, null);
    host.hidden = true;
    area.classList.remove("is-split", "has-surface-layout");
    button.setAttribute("aria-pressed", "false");
    button.disabled = false;
    button.textContent = "Split view";
    button.title = "Show two surfaces side by side";
    onLayout(ids);
    onChange(null);
  }

  const unsubscribe = windowCatalogModel.subscribe(state => state.catalog, () => sync());
  return {
    dispose: unsubscribe,
    open, show, showWindow, close, collapse, sync, select: choose,
    isOpen: () => opened && split,
    get hasLayout() { return opened; },
    activeSurface: () => opened ? selections[active] : null,
    containsWindow: (id) => mounted.has(id),
    focusSurface(surface) {
      const index = selections[active] === surface ? active : split ? selections.indexOf(surface) : -1;
      if (opened && index >= 0) focusPane(index);
    },
    focusWindow(id) {
      const pane = mounted.get(id)?.closest(".split-pane");
      if (!pane) return false;
      focusPane(panes.indexOf(pane));
      return true;
    },
  };
}
