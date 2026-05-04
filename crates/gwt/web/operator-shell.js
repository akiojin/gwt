// SPEC-2356 — Operator Design System: chrome shell wiring.
// Imports the small theme/hotkey modules and assembles the project bar
// theme toggle, command palette, hotkey overlay, mission briefing intro,
// status strip live clock, and Living Telemetry counter logic.

import { createThemeManager, createBrowserEnv } from "/theme-manager.js";
import { createHotkeyManager } from "/hotkey.js";

const SIDEBAR_KEY = "gwt:ui:sidebar";
const BRIEFING_KEY = "gwt:ui:briefing";

export function initOperatorShell(deps = {}) {
  const doc = deps.document ?? document;
  const win = deps.window ?? window;

  const themeManager = deps.themeManager ?? createThemeManager(createBrowserEnv(doc, win));
  const hotkey = deps.hotkey ?? createHotkeyManager();

  wireThemeToggle({ doc, themeManager });
  wireSidebarLayers({ doc, win });
  wireStatusStripClock({ doc });
  wireMissionBriefing({ doc, win });
  wireHotkeyOverlay({ doc, hotkey });
  const palette = wireCommandPalette({ doc, hotkey });
  wireGlobalHotkeys({ doc, hotkey, palette });

  return { themeManager, hotkey, palette };
}

// ------------------------------------------------------------
// Theme toggle (Project Bar)
// ------------------------------------------------------------

function wireThemeToggle({ doc, themeManager }) {
  const btn = doc.getElementById("op-theme-toggle");
  const value = doc.getElementById("op-theme-toggle-value");
  if (!btn || !value) return;

  const renderLabel = () => {
    const pref = themeManager.getPreference();
    const eff = themeManager.getEffective();
    btn.dataset.themeState = pref;
    value.textContent = pref === "auto"
      ? `AUTO ${eff === "dark" ? "▮" : "▯"}`
      : pref.toUpperCase();
  };

  renderLabel();
  themeManager.subscribe(renderLabel);

  btn.addEventListener("click", () => {
    const cycle = { auto: "dark", dark: "light", light: "auto" };
    themeManager.setTheme(cycle[themeManager.getPreference()] ?? "auto");
  });
}

// ------------------------------------------------------------
// Sidebar Layers — persist toggle state
// ------------------------------------------------------------

function wireSidebarLayers({ doc, win }) {
  const layers = doc.querySelectorAll(".op-layer[data-layer]");
  if (!layers.length) return;

  let state = readJson(win, SIDEBAR_KEY, { agents: true, git: true, hooks: true });

  layers.forEach((el) => {
    const key = el.dataset.layer;
    const enabled = state[key] !== false;
    el.setAttribute("aria-pressed", enabled ? "true" : "false");
    el.addEventListener("click", () => {
      const next = el.getAttribute("aria-pressed") !== "true";
      el.setAttribute("aria-pressed", next ? "true" : "false");
      state = { ...state, [key]: next };
      writeJson(win, SIDEBAR_KEY, state);
      doc.documentElement.dataset[`opLayer${capitalize(key)}`] = next ? "on" : "off";
    });
    doc.documentElement.dataset[`opLayer${capitalize(key)}`] = enabled ? "on" : "off";
  });

  const cmdLayers = doc.querySelectorAll(".op-layer[data-cmd]");
  cmdLayers.forEach((el) => {
    el.addEventListener("click", () => {
      doc.dispatchEvent(new CustomEvent("op:command", { detail: { id: el.dataset.cmd } }));
    });
  });
}

// ------------------------------------------------------------
// Status Strip — live clock + counters
// ------------------------------------------------------------

function wireStatusStripClock({ doc }) {
  const clock = doc.getElementById("op-strip-clock");
  if (!clock) return;

  // SPEC-2356 — render the clock with separately-styled colon glyphs so the
  // mission-control "blink" effect can target only the separators while the
  // numbers stay rock-steady. Clears + rebuilds via DOM nodes (no innerHTML
  // because the security hook flags textContent-as-template-string).
  const reduced = matchReduced(doc);
  const setClock = (h, m, s) => {
    while (clock.firstChild) clock.removeChild(clock.firstChild);
    const append = (text, isColon) => {
      const span = doc.createElement("span");
      if (isColon) span.className = "op-strip-clock__colon";
      span.textContent = text;
      clock.appendChild(span);
    };
    append(pad2(h), false);
    append(":", true);
    append(pad2(m), false);
    append(":", true);
    append(pad2(s), false);
  };
  const tick = () => {
    const now = new Date();
    setClock(now.getHours(), now.getMinutes(), now.getSeconds());
  };
  tick();

  if (reduced.matches) {
    setInterval(tick, 1000);
  } else {
    let last = 0;
    const loop = (t) => {
      if (t - last >= 1000) {
        last = t;
        tick();
      }
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(loop);
  }
}

export function applyTelemetryCounts(doc, counts = {}) {
  const setText = (id, v) => {
    const el = doc.getElementById(id);
    if (el) el.textContent = String(v);
  };
  // SPEC-2356 — paint a Sidebar Layer's status hint based on whether the
  // resource is "live" (active > 0). This lets the agents/git rows light up
  // even when the user is not looking at the Status Strip.
  const markLive = (layer, isLive) => {
    const row = doc.querySelector(`.op-layer[data-layer="${layer}"]`);
    if (!row) return;
    if (isLive) row.dataset.live = "true";
    else delete row.dataset.live;
  };
  if ("active" in counts) markLive("agents", (counts.active ?? 0) > 0);
  if ("branches" in counts) markLive("git", (counts.branches ?? 0) > 0);
  setText("op-strip-active", counts.active ?? 0);
  setText("op-strip-idle", counts.idle ?? 0);
  setText("op-strip-blocked", counts.blocked ?? 0);
  if ("branches" in counts) setText("op-strip-branches", counts.branches ?? "—");
  if ("agents" in counts) setText("op-layer-count-agents", counts.agents ?? 0);
  if ("git" in counts) setText("op-layer-count-git", counts.git ?? 0);
  if ("hooks" in counts) setText("op-layer-count-hooks", counts.hooks ?? 0);
  if ("layers" in counts) setText("op-sidebar-count", counts.layers ?? 0);
}

// ------------------------------------------------------------
// Mission Briefing intro
// ------------------------------------------------------------

function wireMissionBriefing({ doc, win }) {
  const overlay = doc.getElementById("op-briefing");
  if (!overlay) return;

  let shown = false;
  try { shown = win.sessionStorage.getItem(BRIEFING_KEY) === "1"; } catch { /* no-op */ }
  if (shown) {
    overlay.hidden = true;
    return;
  }

  // SPEC-2356 — stamp the briefing with the current session timestamp so the
  // splash reads like a mission-control boot log, not just a static splash.
  const stamp = doc.getElementById("op-briefing-stamp");
  if (stamp) {
    const now = new Date();
    const datePart = `${now.getFullYear()}.${pad2(now.getMonth() + 1)}.${pad2(now.getDate())}`;
    const timePart = `${pad2(now.getHours())}:${pad2(now.getMinutes())}:${pad2(now.getSeconds())}`;
    stamp.textContent = `T+0 · ${datePart} ${timePart}`;
  }

  const reduced = matchReduced(doc);
  const totalDelay = reduced.matches ? 250 : 1450;

  overlay.removeAttribute("aria-hidden");
  overlay.hidden = false;

  setTimeout(() => {
    overlay.dataset.state = "exiting";
    setTimeout(() => {
      overlay.hidden = true;
      try { win.sessionStorage.setItem(BRIEFING_KEY, "1"); } catch { /* no-op */ }
    }, reduced.matches ? 0 : 360);
  }, totalDelay);
}

// ------------------------------------------------------------
// Hotkey Overlay
// ------------------------------------------------------------

function wireHotkeyOverlay({ doc, hotkey }) {
  const overlay = doc.getElementById("op-hotkey-overlay");
  if (!overlay) return;

  const open = () => {
    overlay.dataset.open = "true";
    overlay.removeAttribute("aria-hidden");
  };
  const close = () => {
    delete overlay.dataset.open;
    overlay.setAttribute("aria-hidden", "true");
  };

  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) close();
  });

  doc.addEventListener("keydown", (e) => {
    if (overlay.dataset.open === "true" && e.key === "Escape") {
      close();
      e.preventDefault();
    }
  });

  hotkey.register("cmd+shift+/", () => { // ⌘ ?
    if (overlay.dataset.open === "true") close();
    else open();
    return true;
  });
}

// ------------------------------------------------------------
// Command Palette
// ------------------------------------------------------------

function wireCommandPalette({ doc, hotkey }) {
  const overlay = doc.getElementById("op-palette-backdrop");
  const input = doc.getElementById("op-palette-input");
  const list = doc.getElementById("op-palette-list");
  const button = doc.getElementById("op-palette-button");
  if (!overlay || !input || !list) return null;

  const actions = createActionRegistry(doc);
  let selectedIndex = 0;
  let visible = [];

  function open() {
    overlay.dataset.open = "true";
    overlay.removeAttribute("aria-hidden");
    input.value = "";
    selectedIndex = 0;
    render();
    setTimeout(() => input.focus(), 0);
  }

  function close() {
    delete overlay.dataset.open;
    overlay.setAttribute("aria-hidden", "true");
    if (doc.activeElement === input) input.blur();
  }

  function render() {
    const query = input.value.trim().toLowerCase();
    visible = actions.filter(query);
    if (selectedIndex >= visible.length) selectedIndex = Math.max(0, visible.length - 1);
    while (list.firstChild) list.removeChild(list.firstChild);
    if (visible.length === 0) {
      const empty = doc.createElement("li");
      empty.className = "op-palette__empty";
      empty.textContent = query
        ? `No commands match "` + query + `"`
        : "No commands registered yet.";
      list.appendChild(empty);
      return;
    }
    const groups = new Map();
    for (const a of visible) {
      const key = a.group ?? "Commands";
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(a);
    }
    let idx = 0;
    for (const [group, items] of groups) {
      const head = doc.createElement("li");
      head.className = "op-palette__group";
      head.textContent = group;
      list.appendChild(head);
      for (const a of items) {
        const li = doc.createElement("li");
        li.className = "op-palette__row";
        li.dataset.index = String(idx);
        li.dataset.selected = idx === selectedIndex ? "true" : "false";
        li.innerHTML = `<span></span><span class="op-palette__hint"></span>`;
        li.firstChild.textContent = a.label;
        li.lastChild.textContent = a.hint ?? "";
        li.addEventListener("mousemove", () => {
          if (selectedIndex !== idx) {
            selectedIndex = idx;
            updateSelection();
          }
        });
        const myIdx = idx;
        li.addEventListener("click", () => {
          execute(visible[myIdx]);
        });
        list.appendChild(li);
        idx += 1;
      }
    }
  }

  function updateSelection() {
    const rows = list.querySelectorAll(".op-palette__row");
    rows.forEach((row, i) => {
      row.dataset.selected = i === selectedIndex ? "true" : "false";
      if (i === selectedIndex) row.scrollIntoView({ block: "nearest" });
    });
  }

  function execute(action) {
    if (!action) return;
    close();
    try { action.handler(); } catch (e) { console.error("palette action threw", e); }
  }

  input.addEventListener("input", () => {
    selectedIndex = 0;
    render();
  });

  input.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { close(); e.preventDefault(); return; }
    if (e.key === "ArrowDown") {
      selectedIndex = Math.min(selectedIndex + 1, visible.length - 1);
      updateSelection();
      e.preventDefault();
    } else if (e.key === "ArrowUp") {
      selectedIndex = Math.max(selectedIndex - 1, 0);
      updateSelection();
      e.preventDefault();
    } else if (e.key === "Enter") {
      execute(visible[selectedIndex]);
      e.preventDefault();
    }
  });

  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) close();
  });

  button?.addEventListener("click", open);

  hotkey.register("cmd+k", () => { open(); return true; });
  hotkey.register("cmd+p", () => { open(); return true; });

  return {
    open, close,
    register: (action) => actions.register(action),
    unregister: (id) => actions.unregister(id),
  };
}

function createActionRegistry(doc) {
  const items = new Map();

  // Default surface commands. Frontend modules dispatch these via DOM events
  // so we don't need a hard import dependency between operator-shell and app.
  const dispatch = (id) => () => doc.dispatchEvent(new CustomEvent("op:command", { detail: { id } }));

  const seed = [
    { id: "open-board", label: "Focus Board surface", hint: "⌘B", group: "Navigate", handler: dispatch("open-board") },
    { id: "open-git", label: "Focus Git surface", hint: "⌘G", group: "Navigate", handler: dispatch("open-git") },
    { id: "open-logs", label: "Focus Logs surface", hint: "⌘L", group: "Navigate", handler: dispatch("open-logs") },
    { id: "open-help", label: "Show hotkey reference", hint: "⌘?", group: "Navigate", handler: dispatch("open-help") },
    { id: "start-work", label: "Start Work", hint: "Project", group: "Workflow", handler: dispatch("start-work") },
    { id: "spawn-shell", label: "Spawn shell window", group: "Spawn", handler: dispatch("spawn-shell") },
    { id: "spawn-agent", label: "Start Work", group: "Spawn", handler: dispatch("start-work") },
    { id: "open-branches", label: "Open Branches surface", group: "Spawn", handler: dispatch("open-branches") },
    { id: "open-files", label: "Open File Tree", group: "Spawn", handler: dispatch("open-files") },
    { id: "theme-cycle", label: "Cycle theme (auto → dark → light)", group: "View", handler: dispatch("theme-cycle") },
  ];
  seed.forEach((a) => items.set(a.id, a));

  return {
    register(action) {
      if (!action || !action.id) throw new Error("action requires id");
      items.set(action.id, action);
    },
    unregister(id) {
      items.delete(id);
    },
    filter(query) {
      const all = Array.from(items.values());
      if (!query) return all;
      const score = (a) => {
        const haystack = `${a.label} ${a.id} ${a.group ?? ""}`.toLowerCase();
        if (haystack.startsWith(query)) return 100;
        if (haystack.includes(query)) return 50;
        const tokens = query.split(/\s+/).filter(Boolean);
        let acc = 0;
        for (const t of tokens) if (haystack.includes(t)) acc += 10;
        return acc;
      };
      return all.map((a) => ({ a, s: score(a) }))
        .filter(({ s }) => s > 0)
        .sort((x, y) => y.s - x.s)
        .map(({ a }) => a);
    },
  };
}

// ------------------------------------------------------------
// Global hotkeys (delegate to operator command bus)
// ------------------------------------------------------------

function wireGlobalHotkeys({ doc, hotkey, palette }) {
  const send = (id) => () => {
    doc.dispatchEvent(new CustomEvent("op:command", { detail: { id } }));
    return true;
  };

  hotkey.register("cmd+b", send("open-board"));
  hotkey.register("cmd+g", send("open-git"));
  hotkey.register("cmd+l", send("open-logs"));

  if (typeof palette?.close === "function") {
    doc.addEventListener("keydown", (e) => {
      if (e.key === "Escape") palette.close();
    });
  }

  hotkey.attach(doc);
}

// ------------------------------------------------------------
// helpers
// ------------------------------------------------------------

function readJson(win, key, fallback) {
  try {
    const raw = win.localStorage.getItem(key);
    if (!raw) return fallback;
    const parsed = JSON.parse(raw);
    return { ...fallback, ...parsed };
  } catch {
    return fallback;
  }
}

function writeJson(win, key, value) {
  try { win.localStorage.setItem(key, JSON.stringify(value)); } catch { /* no-op */ }
}

function matchReduced(doc) {
  return (doc.defaultView ?? window).matchMedia("(prefers-reduced-motion: reduce)");
}

function pad2(n) {
  return String(n).padStart(2, "0");
}

function capitalize(s) {
  return s.charAt(0).toUpperCase() + s.slice(1);
}
