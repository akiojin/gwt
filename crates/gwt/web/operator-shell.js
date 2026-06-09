// SPEC-2356 — Operator Design System: chrome shell wiring.
// Imports the small theme/hotkey modules and assembles the project bar
// theme toggle, command palette, hotkey overlay, mission briefing intro,
// status strip live clock, and Living Telemetry counter logic.

import { createThemeManager, createBrowserEnv } from "/theme-manager.js";
import { createHotkeyManager } from "/hotkey.js";
import { wireThemeToggle as wireSegmentedThemeToggle } from "/theme-toggle.js";

const BRIEFING_KEY = "gwt:ui:briefing";

// SPEC-2356 Phase 9 (FR-031): hover-reveal state machine close delay window.
// open delay = 0 (instant reveal), close delay defaults to 250ms (200〜400ms range).
const HOVER_REVEAL_CLOSE_DELAY_MS = 250;

export function initOperatorShell(deps = {}) {
  const doc = deps.document ?? document;
  const win = deps.window ?? window;

  let shellDegraded = false;
  const markDegraded = (label, error) => {
    shellDegraded = true;
    hideMissionBriefingImmediately(doc);
    try { console.warn(`operator shell ${label} failed`, error); } catch { /* no-op */ }
  };

  const themeManager = deps.themeManager ?? createThemeManagerSafe(doc, win, markDegraded);
  const hotkey = deps.hotkey ?? createHotkeyManager();

  safeWire("theme toggle", () => wireThemeToggle({ doc, themeManager }), markDegraded);
  const chromeVisibility = safeWire(
    "chrome visibility",
    () => wireChromeVisibility({ doc, win }),
    markDegraded,
    null,
  );
  safeWire("sidebar commands", () => wireSidebarCommands({ doc }), markDegraded);
  safeWire("status strip clock", () => wireStatusStripClock({ doc }), markDegraded);
  if (shellDegraded) hideMissionBriefingImmediately(doc);
  else safeWire("mission briefing", () => wireMissionBriefing({ doc, win }), markDegraded);
  safeWire("hotkey overlay", () => wireHotkeyOverlay({ doc, hotkey }), markDegraded);
  const palette = safeWire(
    "command palette",
    () => wireCommandPalette({ doc, hotkey }),
    markDegraded,
    null,
  );
  safeWire(
    "global hotkeys",
    () => wireGlobalHotkeys({ doc, hotkey, palette }),
    markDegraded,
  );

  void chromeVisibility;

  return { themeManager, hotkey, palette };
}

function safeWire(label, fn, onError, fallback = undefined) {
  try {
    return fn();
  } catch (error) {
    onError(label, error);
    return fallback;
  }
}

function createThemeManagerSafe(doc, win, onError) {
  try {
    return createThemeManager(createBrowserEnv(doc, win));
  } catch (error) {
    onError("theme manager", error);
    doc.documentElement?.setAttribute?.("data-theme", "dark");
    return createFallbackThemeManager();
  }
}

function createFallbackThemeManager() {
  return {
    getPreference() { return "auto"; },
    getEffective() { return "dark"; },
    setTheme() {},
    subscribe() { return () => {}; },
  };
}

function hideMissionBriefingImmediately(doc) {
  const overlay = doc.getElementById("op-briefing");
  if (!overlay) return;
  overlay.dataset.state = "exiting";
  overlay.hidden = true;
  overlay.setAttribute("aria-hidden", "true");
}

// ------------------------------------------------------------
// Chrome visibility — sidebar hover-reveal state machine (SPEC-2356 Phase 9)
// ------------------------------------------------------------
// FR-021: Sidebar を auto-hide にし、画面左端の peek 帯 (`.op-sidebar-peek`)
// への pointer hover / keyboard focus / pointer tap で overlay 展開、panel +
// peek 領域から離れて 200〜400ms 経過後に収納する hover-reveal state machine。
// FR-031: prefers-reduced-motion: reduce で close delay 0ms に縮退。
// FR-032: 起動時に旧 localStorage キー (`gwt:ui:sidebar-collapsed` /
// `gwt:ui:window-controls`) を `removeItem` で 1 回だけ migration する。
//
// SPEC-2356 operator chrome cleanup: window controls (Tile / Stack / Align /
// Windows / Add) と Command Palette / Update CTA を sidebar に集約したため、
// 旧来の独立した window-controls peek 帯 / hover-reveal controller は撤去し、
// sidebar の hover-reveal のみを管理する。

function wireChromeVisibility({ doc, win }) {
  removeLegacyChromeKeys(win);

  const root = doc.documentElement;
  const closeDelayFor = () =>
    matchPrefersReducedMotion(win) ? 0 : HOVER_REVEAL_CLOSE_DELAY_MS;

  const sidebar = createHoverRevealController({
    doc,
    win,
    peek: doc.querySelector(".op-sidebar-peek"),
    panel: doc.getElementById("op-sidebar"),
    revealedAttr: "opSidebar",
    eventName: "op:chrome-visibility-changed",
    eventDetail: () => ({ sidebarVisible: sidebarRevealed() }),
    closeDelayFor,
  });

  function sidebarRevealed() {
    return root.dataset.opSidebar === "revealed";
  }

  // SPEC-2356 operator chrome cleanup: the Update CTA lives in the sidebar
  // Update section, which is auto-hidden. When an update becomes available the
  // CTA dispatches `op:update-available` so we briefly peek the sidebar (and
  // mark it with a badge attribute) — the user notices without hovering.
  doc.addEventListener("op:update-available", () => {
    root.dataset.opSidebarUpdate = "available";
    sidebar.open();
    sidebar.requestClose(closeDelayFor());
  });
  doc.addEventListener("op:update-dismissed", () => {
    delete root.dataset.opSidebarUpdate;
  });

  return {
    isSidebarRevealed: sidebarRevealed,
    closeSidebar: () => sidebar.requestClose(0),
  };
}

function removeLegacyChromeKeys(win) {
  const storage = safeLocalStorage(win);
  if (!storage) return;
  try { storage.removeItem("gwt:ui:sidebar-collapsed"); } catch { /* no-op */ }
  try { storage.removeItem("gwt:ui:window-controls"); } catch { /* no-op */ }
}

function safeLocalStorage(win) {
  try { return win.localStorage ?? null; } catch { return null; }
}

function matchPrefersReducedMotion(win) {
  try {
    return Boolean(win.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches);
  } catch {
    return false;
  }
}

function createHoverRevealController({
  doc,
  win,
  peek,
  panel,
  extraPanels = [],
  revealedAttr,
  eventName,
  eventDetail,
  closeDelayFor,
}) {
  const root = doc.documentElement;
  const datasetKey = revealedAttr;
  const setTimeoutFn = win.setTimeout?.bind(win) ?? globalThis.setTimeout;
  const clearTimeoutFn = win.clearTimeout?.bind(win) ?? globalThis.clearTimeout;
  let closeTimer = null;

  const cancelClose = () => {
    if (closeTimer !== null) {
      clearTimeoutFn(closeTimer);
      closeTimer = null;
    }
  };

  const setRevealed = (next) => {
    const before = root.dataset[datasetKey];
    if (next) {
      root.dataset[datasetKey] = "revealed";
    } else {
      delete root.dataset[datasetKey];
    }
    if (before !== root.dataset[datasetKey]) {
      try {
        doc.dispatchEvent(new CustomEvent(eventName, { detail: eventDetail() }));
      } catch { /* no-op */ }
    }
  };

  const open = () => {
    cancelClose();
    setRevealed(true);
  };

  const requestClose = (delay) => {
    cancelClose();
    if (!delay || delay <= 0) {
      setRevealed(false);
      return;
    }
    closeTimer = setTimeoutFn(() => {
      closeTimer = null;
      setRevealed(false);
    }, delay);
  };

  if (!peek || !panel) {
    return { open, requestClose };
  }

  const allTargets = [peek, panel, ...extraPanels].filter(Boolean);

  for (const target of allTargets) {
    target.addEventListener("pointerenter", open);
    target.addEventListener("focusin", open);
    target.addEventListener("pointerdown", (event) => {
      if (event && event.pointerType === "touch") open();
    });
    target.addEventListener("pointerleave", () => requestClose(closeDelayFor()));
    target.addEventListener("focusout", () => requestClose(closeDelayFor()));
  }

  // Tap-away: pointerdown outside the peek / panel collapses an open overlay.
  doc.addEventListener("pointerdown", (event) => {
    if (root.dataset[datasetKey] !== "revealed") return;
    const target = event?.target;
    if (!target) return;
    const inside = allTargets.some((node) => node === target || node?.contains?.(target));
    if (!inside) requestClose(closeDelayFor());
  });

  return { open, requestClose };
}

// ------------------------------------------------------------
// Theme toggle (Project Bar) — segmented radiogroup
// ------------------------------------------------------------
// SPEC-2356 FR-024: AUTO / DARK / LIGHT are exposed as a parallel radiogroup
// so AUTO is reachable from any state in a single click. Implementation lives
// in `/theme-toggle.js` so it is unit-testable under Node.

function wireThemeToggle(opts) {
  wireSegmentedThemeToggle(opts);
}

// ------------------------------------------------------------
// Sidebar command rows
// ------------------------------------------------------------
// SPEC-2356 operator chrome cleanup: the dead Layers section (Agents / Git /
// Hooks toggles + counters) is removed — toggling only set dataset.opLayer*
// which no CSS ever read, so it was a no-op. The sidebar now carries only
// actionable rows; this wiring dispatches the `data-cmd` rows (Start Work /
// Board / Logs etc.) onto the operator command bus.

function wireSidebarCommands({ doc }) {
  const cmdRows = doc.querySelectorAll(".op-sidebar .op-layer[data-cmd]");
  cmdRows.forEach((el) => {
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
  // SPEC-2356 — toggle the blocked alert state so the BLOCKED cell pulses when
  // anything actually needs attention, and stays still otherwise.
  if ("blocked" in counts) {
    const blockedCell = doc.querySelector(".op-status-strip__cell--blocked");
    if (blockedCell) {
      if ((counts.blocked ?? 0) > 0) blockedCell.classList.add("op-status-strip__cell--alert");
      else blockedCell.classList.remove("op-status-strip__cell--alert");
    }
  }
  // SPEC-2356 operator chrome cleanup: the dead Sidebar Layers rows and their
  // per-layer counters are removed; telemetry now lives solely in the Status
  // Strip cells below.
  setText("op-strip-active", counts.active ?? 0);
  setText("op-strip-idle", counts.idle ?? 0);
  setText("op-strip-blocked", counts.blocked ?? 0);
  if ("branches" in counts) setText("op-strip-branches", counts.branches ?? "—");
}

// ------------------------------------------------------------
// Provider usage pill (SPEC-2970)
// ------------------------------------------------------------

const USAGE_PROVIDER_ICON = { codex: "⬡", claude_code: "◇" };

function usageWindowByKind(account, kind) {
  return (account.windows || []).find((w) => w.kind === kind) || null;
}

// Stable, glanceable per-provider summary value for the status strip (weekly %
// when available, else a short degraded token). No rotation.
function usageSummaryValue(account) {
  const kind = (account.state || {}).kind || "ok";
  if (kind === "disabled") return "off";
  if (kind === "no_data") return "—";
  if (kind === "unavailable") return "n/a";
  const week = usageWindowByKind(account, "weekly");
  const five = usageWindowByKind(account, "five_hour");
  if (week) return `${Math.round(week.used_percent)}%`;
  if (five) return `${Math.round(five.used_percent)}%`;
  return "—";
}

// SPEC-2970 — status-strip USAGE cell: a stable, consolidated summary
// (`USAGE ⬡ 23% ◇ 9%`). Hover shows the full consolidated popover (all windows
// + consumption) via the app-provided hooks; click opens the detail modal. No
// ticker rotation — everything is visible at a glance / on hover.
export function applyProviderUsage(doc, snapshot = {}) {
  const cell = doc.getElementById("op-strip-usage");
  if (!cell) return;
  const accounts = snapshot.accounts || [];
  if (!accounts.length) {
    cell.hidden = true;
    return;
  }
  while (cell.firstChild) cell.removeChild(cell.firstChild);
  const label = doc.createElement("span");
  label.className = "op-status-strip__label";
  label.textContent = "USAGE";
  cell.appendChild(label);
  for (const account of accounts) {
    const chip = doc.createElement("span");
    chip.className = "op-usage-sum";
    chip.dataset.provider = account.provider;
    if (account.limit_reached) chip.dataset.limit = "true";
    chip.textContent = `${USAGE_PROVIDER_ICON[account.provider] || ""} ${usageSummaryValue(
      account,
    )}`;
    cell.appendChild(chip);
  }
  if (accounts.some((a) => a.limit_reached)) cell.dataset.limit = "true";
  else delete cell.dataset.limit;
  cell.hidden = false;

  const win = doc.defaultView || window;
  // No modal: hover (or click, for touch/non-hover) shows the full popover.
  cell.onclick = () => {
    try {
      win.__gwtShowUsageHover?.(cell);
    } catch {
      /* no-op */
    }
  };
  cell.onmouseenter = () => {
    try {
      win.__gwtShowUsageHover?.(cell);
    } catch {
      /* no-op */
    }
  };
  cell.onmouseleave = () => {
    try {
      win.__gwtHideUsageHover?.();
    } catch {
      /* no-op */
    }
  };
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

  // SPEC-2356 — stamp the briefing with the current session timestamp + a
  // 6-char session-id-style hash so the splash reads like a mission-control
  // boot log, not just a static splash. The hash is mathematically derived
  // from the boot timestamp so two simultaneous sessions render distinct
  // strings without spinning up any randomness source.
  const stamp = doc.getElementById("op-briefing-stamp");
  if (stamp) {
    const now = new Date();
    const datePart = `${now.getFullYear()}.${pad2(now.getMonth() + 1)}.${pad2(now.getDate())}`;
    const timePart = `${pad2(now.getHours())}:${pad2(now.getMinutes())}:${pad2(now.getSeconds())}`;
    let hashSrc = now.getTime();
    let hash = "";
    while (hash.length < 6) {
      hashSrc = (hashSrc * 9301 + 49297) % 0xfffff;
      hash += hashSrc.toString(16).padStart(5, "0").slice(-2);
    }
    const sessionId = hash.slice(0, 6).toUpperCase();
    stamp.textContent = `T+0 · ${datePart} ${timePart} · SESSION ${sessionId}`;
  }

  const reduced = matchReduced(doc);
  const totalDelay = reduced.matches ? 250 : 1450;

  overlay.removeAttribute("aria-hidden");
  overlay.hidden = false;

  // SPEC-2356 — let the user dismiss the splash early by pressing any
  // key or clicking through it. The splash is purely decorative and
  // shouldn't block users who want to get into the canvas immediately.
  let exited = false;
  const exitNow = () => {
    if (exited) return;
    exited = true;
    overlay.dataset.state = "exiting";
    setTimeout(() => {
      overlay.hidden = true;
      try { win.sessionStorage.setItem(BRIEFING_KEY, "1"); } catch { /* no-op */ }
    }, reduced.matches ? 0 : 360);
  };

  const earlyDismiss = (event) => {
    if (overlay.hidden) return;
    // Only fast-forward once, after the staged lines have started rendering
    // so the user actually sees that something happened.
    if (event && event.type === "keydown" && event.key === "Tab") return;
    exitNow();
  };
  doc.addEventListener("keydown", earlyDismiss, { once: true });
  overlay.addEventListener("click", earlyDismiss, { once: true });

  setTimeout(exitNow, totalDelay);
}

// ------------------------------------------------------------
// Hotkey Overlay
// ------------------------------------------------------------

function wireHotkeyOverlay({ doc, hotkey }) {
  const overlay = doc.getElementById("op-hotkey-overlay");
  if (!overlay) return;
  const card = overlay.querySelector(".op-hotkey-card");

  // SPEC-2356 — modal-dialog focus management: remember the trigger so we can
  // restore focus on close, and move focus into the dialog on open so screen
  // readers announce "Hotkey reference dialog" instead of staying on whatever
  // surface invoked ⌘?.
  let returnFocusTo = null;

  const open = () => {
    returnFocusTo = doc.activeElement instanceof Element ? doc.activeElement : null;
    overlay.dataset.open = "true";
    overlay.removeAttribute("aria-hidden");
    if (card) {
      try { card.focus({ preventScroll: true }); } catch { card.focus(); }
    }
  };
  const close = () => {
    delete overlay.dataset.open;
    overlay.setAttribute("aria-hidden", "true");
    if (returnFocusTo && typeof returnFocusTo.focus === "function") {
      try { returnFocusTo.focus({ preventScroll: true }); } catch { returnFocusTo.focus(); }
    }
    returnFocusTo = null;
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
    // SPEC-2356 — combobox accessibility: announce that the popup is now open
    // so screen readers attach the listbox to the input.
    input.setAttribute("aria-expanded", "true");
    input.value = "";
    selectedIndex = 0;
    render();
    setTimeout(() => input.focus(), 0);
  }

  function close() {
    delete overlay.dataset.open;
    overlay.setAttribute("aria-hidden", "true");
    input.setAttribute("aria-expanded", "false");
    input.removeAttribute("aria-activedescendant");
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
        // SPEC-2356 — combobox/listbox a11y: each row is an option with a
        // stable id so the input can target it via aria-activedescendant.
        li.id = `op-palette-row-${idx}`;
        li.setAttribute("role", "option");
        li.setAttribute("aria-selected", idx === selectedIndex ? "true" : "false");
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
      const isSelected = i === selectedIndex;
      row.dataset.selected = isSelected ? "true" : "false";
      row.setAttribute("aria-selected", isSelected ? "true" : "false");
      if (isSelected) {
        row.scrollIntoView({ block: "nearest" });
        // SPEC-2356 — point the combobox input at the active option so screen
        // readers announce the highlighted command without moving DOM focus.
        input.setAttribute("aria-activedescendant", row.id);
      }
    });
    if (rows.length === 0) input.removeAttribute("aria-activedescendant");
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
    { id: "open-git", label: "Focus Work", hint: "⌘G", group: "Navigate", handler: dispatch("open-git") },
    { id: "open-logs", label: "Focus Logs surface", hint: "⌘L", group: "Navigate", handler: dispatch("open-logs") },
    { id: "open-help", label: "Show hotkey reference", hint: "⌘?", group: "Navigate", handler: dispatch("open-help") },
    { id: "start-work", label: "Start Work", hint: "Project", group: "Workflow", handler: dispatch("start-work") },
    { id: "spawn-shell", label: "Spawn shell window", group: "Spawn", handler: dispatch("spawn-shell") },
    { id: "spawn-agent", label: "Start Work", group: "Spawn", handler: dispatch("start-work") },
    { id: "open-branches", label: "Open Work", group: "Spawn", handler: dispatch("open-branches") },
    { id: "open-files", label: "Open File Tree", group: "Spawn", handler: dispatch("open-files") },
    { id: "open-index", label: "Open Index search", group: "Spawn", handler: dispatch("open-index") },
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
  // SPEC-2356 Phase 9: Cmd+\\ sidebar toggle hotkey is removed in favor of the
  // hover-reveal peek 帯. Chrome visibility is now driven entirely by pointer
  // hover / keyboard focus / pointer tap.

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

function matchReduced(doc) {
  return (doc.defaultView ?? window).matchMedia("(prefers-reduced-motion: reduce)");
}

function pad2(n) {
  return String(n).padStart(2, "0");
}
