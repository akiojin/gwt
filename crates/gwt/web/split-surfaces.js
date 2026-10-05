const SURFACES = { issues: "Issues", agents: "Agents", board: "Board", settings: "Settings" };
const PRESETS = { issues: "issue", board: "board", settings: "settings" };

// A client-local layout: move live views, never clone terminals or persist
// split dimensions as canvas geometry. The canvas remains the single-view
// layout until the later #4777 migration slices retire it.
export function createSplitSurfaces({
  document, stage, getWindows, getElement, openSurface, onChange, onLayout,
  restoreVisibility = () => {}, onAgentsHost = () => {},
}) {
  const host = document.getElementById("split-surfaces");
  const button = document.getElementById("split-view-button");
  const area = host.parentElement;
  let opened = false;
  let active = 0;
  let selections = ["issues", "board"];
  const requested = new Set();
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
    const changed = active !== index;
    active = index;
    for (const [i, pane] of panes.entries()) pane.dataset.active = String(i === active);
    if (changed && selections.every(surface => surface === "agents")) sync();
    onChange(selections[active]);
  }

  function sync() {
    if (!opened) return;
    const windows = getWindows();
    const desired = new Map();
    const assigned = new Map();
    // Reserve each existing pane's live view before allocating a second view.
    for (const [index, pane] of panes.entries()) {
      const data = windows.find(data => data.preset === PRESETS[selections[index]] &&
        getElement(data.id)?.parentElement === pane.querySelector(".split-pane__body"));
      if (data) { assigned.set(index, data); desired.set(data.id, { element: getElement(data.id), index }); }
    }
    for (const [index, pane] of panes.entries()) {
      const surface = selections[index];
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
      const data = assigned.get(index) || windows.find(data => data.preset === PRESETS[surface] && !desired.has(data.id));
      const empty = pane.querySelector(".split-pane__empty");
      empty.hidden = Boolean(data);
      empty.textContent = `Opening ${SURFACES[surface]}…`;
      const request = `${index}:${surface}`;
      if (data) requested.delete(request);
      else if (!requested.has(request)) {
        requested.add(request);
        openSurface(surface);
      }
      if (data) {
        const element = getElement(data.id);
        if (element) desired.set(data.id, { element, index });
      }
    }
    const agentPanes = panes.filter((_, index) => selections[index] === "agents");
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

  function choose(surface, index = active) {
    if (!opened || !SURFACES[surface]) return false;
    selections[index] = surface;
    active = index;
    sync();
    return true;
  }

  function open(surface = "issues") {
    if (opened) return;
    selections = [SURFACES[surface] ? surface : "issues", surface === "board" ? "issues" : "board"];
    active = 0;
    opened = true;
    host.hidden = false;
    area.classList.add("is-split");
    button.setAttribute("aria-pressed", "true");
    button.textContent = "Close split";
    button.title = "Return to the canvas without closing any windows";
    sync();
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
    requested.clear();
    opened = false;
    onAgentsHost(null, null);
    host.hidden = true;
    area.classList.remove("is-split");
    button.setAttribute("aria-pressed", "false");
    button.textContent = "Split view";
    button.title = "Show two surfaces side by side";
    onLayout(ids);
    onChange(null);
  }

  return {
    open, close, sync, select: choose,
    isOpen: () => opened,
    activeSurface: () => opened ? selections[active] : null,
    containsWindow: (id) => mounted.has(id),
    focusSurface(surface) {
      const index = selections[active] === surface ? active : selections.indexOf(surface);
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
