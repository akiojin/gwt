// SPEC-2356 — Operator Design System: chrome shell wiring.
// Imports the small theme/hotkey modules and assembles the project bar
// theme toggle, command palette, hotkey overlay, mission briefing intro,
// status strip live clock, and Living Telemetry counter logic.

import { createThemeManager, createBrowserEnv } from "/theme-manager.js";
import { createHotkeyManager } from "/hotkey.js";
import { wireThemeToggle as wireSegmentedThemeToggle } from "/theme-toggle.js";

const BRIEFING_KEY = "gwt:ui:briefing";

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
  safeWire("legacy chrome keys", () => removeLegacyChromeKeys(win), markDegraded);
  safeWire("rail commands", () => wireRailCommands({ doc }), markDegraded);
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
// Legacy chrome key migration (SPEC-2356 Phase 9, kept in the rail era)
// ------------------------------------------------------------
// The rail is grid-docked and always visible; the hover-reveal machinery and
// the rail update badge are gone (the Update CTA floats fixed bottom-right —
// user verification 2026-06-12). Only the one-shot localStorage migration
// survives so legacy keys keep getting cleaned up.

function removeLegacyChromeKeys(win) {
  const storage = safeLocalStorage(win);
  if (!storage) return;
  try { storage.removeItem("gwt:ui:sidebar-collapsed"); } catch { /* no-op */ }
  try { storage.removeItem("gwt:ui:window-controls"); } catch { /* no-op */ }
}

function safeLocalStorage(win) {
  try { return win.localStorage ?? null; } catch { return null; }
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
// Command Rail command items
// ------------------------------------------------------------
// SPEC-3038: rail items carrying a `data-cmd` attribute (Start Work / Board /
// Logs) dispatch onto the operator command bus; id-wired items (#tile-button
// etc.) keep their app.js handlers untouched.

function wireRailCommands({ doc }) {
  const items = doc.querySelectorAll(".op-rail .op-rail__item[data-cmd]");
  items.forEach((el) => {
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
  // anything actually needs attention, and stays still otherwise. FR-039 (anshin)
  // applies the same alert toggle to the WAITING cell so "agents waiting for
  // input" is just as loud as blocked.
  const toggleAlert = (modifier, count) => {
    const cell = doc.querySelector(`.op-status-strip__cell--${modifier}`);
    if (!cell) return;
    if ((count ?? 0) > 0) cell.classList.add("op-status-strip__cell--alert");
    else cell.classList.remove("op-status-strip__cell--alert");
  };
  if ("blocked" in counts) toggleAlert("blocked", counts.blocked);
  if ("needs_input" in counts) toggleAlert("waiting", counts.needs_input);
  // FR-047 (anshin): MISSION convergence indicator. Shows done/total agents so
  // the operator can see the fleet trending toward completion at a glance, and
  // flips a positive "complete" state (not an alert pulse) when every agent has
  // converged (done === agents > 0).
  const agents = Number(counts.agents ?? 0);
  const done = Number(counts.done ?? 0);
  setText("op-strip-mission", agents > 0 ? `${done}/${agents}` : "—");
  const missionCell = doc.querySelector(".op-status-strip__cell--mission");
  if (missionCell) {
    if (agents > 0 && done === agents) {
      missionCell.classList.add("op-status-strip__cell--complete");
    } else {
      missionCell.classList.remove("op-status-strip__cell--complete");
    }
  }
  // SPEC-2356 operator chrome cleanup: the dead Sidebar Layers rows and their
  // per-layer counters are removed; telemetry now lives solely in the Status
  // Strip cells below.
  setText("op-strip-active", counts.active ?? 0);
  setText("op-strip-idle", counts.idle ?? 0);
  // FR-039 (anshin): WAITING cell counts agents waiting on the operator.
  setText("op-strip-waiting", counts.needs_input ?? 0);
  setText("op-strip-blocked", counts.blocked ?? 0);
  if ("branches" in counts) setText("op-strip-branches", counts.branches ?? "—");
  // SPEC-3038 AS-1.4: the rail Windows item badges the open-window count.
  if ("windows" in counts) {
    const badge = doc.getElementById("op-rail-window-count");
    if (badge) {
      const value = Number(counts.windows) || 0;
      badge.textContent = String(value);
      badge.hidden = value <= 0;
    }
  }
}

export function applyIssueMonitorStatus(doc, status = {}) {
  const cell = doc.getElementById("op-strip-issue-monitor");
  const value = doc.getElementById("op-strip-issue-monitor-value");
  if (!cell || !value) return;

  wireIssueMonitorStatusCell(doc, cell);
  const view = issueMonitorStatusStripView(status);
  value.textContent = view.value;
  cell.dataset.state = view.state;
  cell.setAttribute("aria-label", `Issue Monitor: ${view.value}`);
  cell.setAttribute("title", view.title);
  if (view.alert) {
    cell.classList.add("op-status-strip__cell--alert");
  } else {
    cell.classList.remove("op-status-strip__cell--alert");
  }
}

function wireIssueMonitorStatusCell(doc, cell) {
  if (cell.dataset.issueMonitorBound === "true") return;
  cell.dataset.issueMonitorBound = "true";
  cell.addEventListener("click", () => {
    doc.dispatchEvent(new CustomEvent("op:command", { detail: { id: "open-issue-monitor" } }));
  });
}

function issueMonitorStatusStripView(status = {}) {
  const enabled = Boolean(status.enabled);
  const rawState = String(status.state || (enabled ? "idle" : "disabled"));
  const state = enabled ? rawState : "disabled";
  const queue = Math.max(0, Number(status.queue_len || 0));
  const active = Math.max(0, Number(status.active_count || 0));
  const maxActive = Math.max(1, Number(status.max_active_agents || 1));

  if (state === "disabled") {
    return {
      state,
      value: "Off",
      title: "Issue Monitor: Off",
      alert: false,
    };
  }

  let label = "Idle";
  if (state === "error") label = "Error";
  else if (state === "auth_required") label = "Auth";
  else if (state === "launching" || state === "active") label = "Run";

  const value = `${label} Q${queue} A${active}/${maxActive}`;
  const lastError = typeof status.last_error === "string" ? status.last_error.trim() : "";
  return {
    state,
    value,
    title: lastError ? `Issue Monitor: ${value} | ${lastError}` : `Issue Monitor: ${value}`,
    alert: state === "error" || state === "auth_required",
  };
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
// Runtime health cell (SPEC-3107)
// ------------------------------------------------------------

const RUNTIME_HEALTH_SCROLL_PROCESS_THRESHOLD = 10;
const RUNTIME_HEALTH_DEFAULT_SORT = "load";
const RUNTIME_HEALTH_SORT_MODES = [
  { mode: "load", label: "Load" },
  { mode: "cpu", label: "CPU" },
  { mode: "mem", label: "Mem" },
];
const RUNTIME_HEALTH_LOAD_CPU_UNIT = 20;
const RUNTIME_HEALTH_LOAD_MEMORY_UNIT = 512 * 1024 * 1024;
let runtimeHealthHideTimer = null;

export function applyRuntimeHealth(doc, snapshot = {}, options = {}) {
  const cell = doc.getElementById("op-strip-runtime-health");
  const value = doc.getElementById("op-strip-runtime-health-value");
  if (!cell || !value) return;

  const queue = snapshot.queue || {};
  const state = runtimeHealthState(snapshot.state);
  cell.dataset.state = state;
  cell.hidden = false;
  value.textContent = `${runtimeHealthStateLabel(state)} ${formatRuntimeCpu(
    snapshot.cpu_percent,
  )} ${formatRuntimeMemory(
    snapshot.memory_bytes,
  )}`;
  cell.setAttribute("title", runtimeHealthTitle(snapshot, queue));
  cell.dataset.runtimeHealthSort = runtimeHealthSortMode(cell.dataset.runtimeHealthSort);
  wireRuntimeHealthDetail(doc, cell);
  renderRuntimeHealthDetail(doc, snapshot, queue, options);
}

function runtimeHealthState(state) {
  return ["warming", "ok", "warn", "hot"].includes(state) ? state : "ok";
}

function runtimeHealthStateLabel(state) {
  switch (runtimeHealthState(state)) {
    case "hot":
      return "HOT";
    case "warn":
      return "WARN";
    case "warming":
      return "WARM";
    default:
      return "OK";
  }
}

function formatRuntimeCpu(cpu) {
  if (typeof cpu !== "number" || !Number.isFinite(cpu)) return "--";
  return `${Math.round(cpu)}%`;
}

function formatRuntimeMemory(bytes) {
  const value = Number(bytes) || 0;
  if (value <= 0) return "--";
  const mib = value / (1024 * 1024);
  if (mib < 1024) return `${Math.round(mib)}M`;
  return `${(mib / 1024).toFixed(1)}G`;
}

function signedDelta(value) {
  const delta = Number(value) || 0;
  return delta > 0 ? `+${delta}` : String(delta);
}

function runtimeHealthTitle(snapshot, queue) {
  return [
    `state: ${runtimeHealthState(snapshot.state)}`,
    `processes: ${Number(snapshot.process_count) || 0}`,
    `runners: ${Number(snapshot.runner_count) || 0}`,
    `queued: ${Number(queue.queued_entries) || 0}`,
    `dropped: ${signedDelta(queue.dropped_lossy_delta)}`,
  ].join(" | ");
}

function wireRuntimeHealthDetail(doc, cell) {
  if (cell.dataset.runtimeHealthBound === "true") return;
  cell.dataset.runtimeHealthBound = "true";
  const win = doc.defaultView || globalThis;
  const show = () => {
    clearRuntimeHealthHideTimer(win);
    showRuntimeHealthDetail(doc, cell);
  };
  const hide = () => {
    scheduleRuntimeHealthHide(doc, win);
  };
  cell.addEventListener("mouseenter", show);
  cell.addEventListener("focus", show);
  cell.addEventListener("mouseleave", hide);
  cell.addEventListener("blur", hide);
  const detail = runtimeHealthDetail(doc);
  detail.addEventListener("mouseenter", () => clearRuntimeHealthHideTimer(win));
  detail.addEventListener("focusin", () => clearRuntimeHealthHideTimer(win));
  detail.addEventListener("mouseleave", hide);
  detail.addEventListener("focusout", hide);
}

function clearRuntimeHealthHideTimer(win = globalThis) {
  if (!runtimeHealthHideTimer) return;
  (win.clearTimeout || clearTimeout)(runtimeHealthHideTimer);
  runtimeHealthHideTimer = null;
}

function scheduleRuntimeHealthHide(doc, win = globalThis) {
  clearRuntimeHealthHideTimer(win);
  runtimeHealthHideTimer = (win.setTimeout || setTimeout)(() => {
    runtimeHealthHideTimer = null;
    hideRuntimeHealthDetail(doc);
  }, 120);
}

function runtimeHealthDetail(doc) {
  let detail = doc.getElementById("op-runtime-health-detail");
  if (detail) return detail;
  detail = doc.createElement("div");
  detail.className = "op-runtime-health-detail";
  detail.id = "op-runtime-health-detail";
  detail.hidden = true;
  detail.setAttribute("role", "tooltip");
  doc.body.appendChild(detail);
  return detail;
}

function showRuntimeHealthDetail(doc, cell) {
  const detail = runtimeHealthDetail(doc);
  detail.hidden = false;
  const rect = cell.getBoundingClientRect?.();
  if (!rect) return;
  const viewport = doc.defaultView || {};
  const width = Number(viewport.innerWidth) || 1024;
  const height = Number(viewport.innerHeight) || 768;
  const margin = 8;
  const statusGap = 8;
  const detailWidth = Math.min(420, Math.max(0, width - margin * 2));
  const maxHeight = Math.max(220, Math.floor(rect.top - margin * 2));
  detail.style.left = `${Math.max(
    margin,
    Math.min(rect.left, width - detailWidth - margin),
  )}px`;
  detail.style.bottom = `${Math.max(margin, height - rect.top + statusGap)}px`;
  detail.style.maxHeight = `${maxHeight}px`;
}

function hideRuntimeHealthDetail(doc) {
  const detail = doc.getElementById("op-runtime-health-detail");
  if (detail) detail.hidden = true;
}

function renderRuntimeHealthDetail(doc, snapshot, queue, options) {
  const detail = runtimeHealthDetail(doc);
  while (detail.firstChild) detail.removeChild(detail.firstChild);

  const summary = doc.createElement("div");
  summary.className = "op-runtime-health-detail__summary";
  appendRuntimeHealthChip(doc, summary, "STATE", runtimeHealthStateLabel(snapshot.state));
  appendRuntimeHealthChip(doc, summary, "CPU", formatRuntimeCpu(snapshot.cpu_percent));
  appendRuntimeHealthChip(doc, summary, "MEM", formatRuntimeMemory(snapshot.memory_bytes));
  appendRuntimeHealthChip(
    doc,
    summary,
    "PROC",
    `${Number(snapshot.process_count) || 0}/${Number(snapshot.runner_count) || 0}`,
  );
  detail.appendChild(summary);

  const queueBlock = doc.createElement("div");
  queueBlock.className = "op-runtime-health-detail__queue";
  appendRuntimeHealthQueueItem(doc, queueBlock, "clients", Number(queue.client_count) || 0);
  appendRuntimeHealthQueueItem(doc, queueBlock, "queued", Number(queue.queued_entries) || 0);
  appendRuntimeHealthQueueItem(doc, queueBlock, "dirty", Number(queue.dirty_panes) || 0);
  appendRuntimeHealthQueueItem(
    doc,
    queueBlock,
    "dropped",
    signedDelta(queue.dropped_lossy_delta),
  );
  detail.appendChild(queueBlock);

  const sortMode = runtimeHealthCurrentSortMode(doc);
  detail.appendChild(
    renderRuntimeHealthSortControls(doc, sortMode, (nextMode) => {
      const win = doc.defaultView || globalThis;
      const cell = doc.getElementById("op-strip-runtime-health");
      if (cell) cell.dataset.runtimeHealthSort = nextMode;
      clearRuntimeHealthHideTimer(win);
      renderRuntimeHealthDetail(doc, snapshot, queue, options);
      clearRuntimeHealthHideTimer(win);
      if (cell) showRuntimeHealthDetail(doc, cell);
    }),
  );

  const processList = doc.createElement("div");
  processList.className = "op-runtime-health-detail__process-list";
  processList.setAttribute("aria-label", "Runtime processes");
  const rawProcesses = Array.isArray(snapshot.processes) ? snapshot.processes : [];
  const processes = runtimeHealthDisplayProcesses(rawProcesses, sortMode);
  if (processes.length > RUNTIME_HEALTH_SCROLL_PROCESS_THRESHOLD) {
    processList.dataset.scroll = "true";
  }
  if (processes.length === 0) {
    const empty = doc.createElement("div");
    empty.className = "op-runtime-health-detail__empty";
    empty.textContent = "No process detail";
    processList.appendChild(empty);
  } else {
    processList.appendChild(renderRuntimeHealthProcessHeader(doc));
    for (const process of processes) {
      processList.appendChild(renderRuntimeHealthProcess(doc, process, options));
    }
    const more = doc.createElement("div");
    more.className = "op-runtime-health-detail__process-more";
    more.textContent = runtimeHealthProcessSummary(processes, rawProcesses, sortMode);
    processList.appendChild(more);
  }
  detail.appendChild(processList);
}

function runtimeHealthFocusWindowId(process) {
  return typeof process?.focus_window_id === "string" && process.focus_window_id.trim()
    ? process.focus_window_id
    : null;
}

function appendRuntimeHealthChip(doc, parent, label, value) {
  const chip = doc.createElement("span");
  chip.className = "op-runtime-health-detail__chip";
  const labelEl = doc.createElement("span");
  labelEl.className = "op-runtime-health-detail__chip-label";
  labelEl.textContent = label;
  const valueEl = doc.createElement("span");
  valueEl.className = "op-runtime-health-detail__chip-value";
  valueEl.textContent = String(value);
  chip.appendChild(labelEl);
  chip.appendChild(valueEl);
  parent.appendChild(chip);
}

function appendRuntimeHealthQueueItem(doc, parent, label, value) {
  const item = doc.createElement("span");
  item.className = "op-runtime-health-detail__queue-item";
  const labelEl = doc.createElement("span");
  labelEl.className = "op-runtime-health-detail__queue-label";
  labelEl.textContent = label;
  const valueEl = doc.createElement("span");
  valueEl.className = "op-runtime-health-detail__queue-value";
  valueEl.textContent = String(value);
  item.appendChild(labelEl);
  item.appendChild(valueEl);
  parent.appendChild(item);
}

function renderRuntimeHealthSortControls(doc, activeMode, onChange) {
  const group = doc.createElement("div");
  group.className = "op-runtime-health-detail__sort";
  group.setAttribute("aria-label", "Runtime process sort");
  for (const { mode, label } of RUNTIME_HEALTH_SORT_MODES) {
    const button = doc.createElement("button");
    button.className = "op-runtime-health-detail__sort-button";
    button.type = "button";
    button.textContent = label;
    button.dataset.sort = mode;
    button.setAttribute("aria-pressed", mode === activeMode ? "true" : "false");
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (mode !== activeMode) onChange(mode);
    });
    group.appendChild(button);
  }
  return group;
}

function renderRuntimeHealthProcessHeader(doc) {
  const header = doc.createElement("div");
  header.className = "op-runtime-health-detail__process-header";
  for (const label of ["Role", "Process", "PID", "CPU", "Mem"]) {
    const cell = doc.createElement("span");
    cell.textContent = label;
    header.appendChild(cell);
  }
  return header;
}

function renderRuntimeHealthProcess(doc, process, options = {}) {
  const focusWindowId = runtimeHealthFocusWindowId(process);
  const canFocus = focusWindowId && typeof options.focusWindow === "function";
  const row = doc.createElement(canFocus ? "button" : "div");
  row.className = "op-runtime-health-detail__process";
  row.dataset.heat = runtimeHealthProcessHeat(process.cpu_percent);
  if (runtimeHealthProcessLooksLikeAgent(process)) row.dataset.agent = "true";
  const cpuPercent = Number(process.cpu_percent);
  if (Number.isFinite(cpuPercent)) {
    row.style.setProperty("--runtime-health-cpu", `${Math.max(0, Math.min(cpuPercent, 100))}%`);
  }
  if (canFocus) {
    row.classList.add("op-runtime-health-detail__process--focusable");
    row.type = "button";
    const processLabel = runtimeHealthProcessNameLabel(process);
    row.setAttribute(
      "aria-label",
      `Focus ${processLabel || "process"} process ${runtimeHealthProcessPidLabel(process)}`,
    );
    row.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      options.focusWindow(focusWindowId);
    });
  }

  const role = doc.createElement("span");
  role.className = "op-runtime-health-detail__process-role";
  role.textContent = process.role || "process";
  row.appendChild(role);

  const name = doc.createElement("span");
  name.className = "op-runtime-health-detail__process-name";
  name.textContent = runtimeHealthProcessNameLabel(process);
  row.appendChild(name);

  const pid = doc.createElement("span");
  pid.className = "op-runtime-health-detail__process-pid";
  pid.textContent = runtimeHealthProcessPidLabel(process);
  row.appendChild(pid);

  const cpu = doc.createElement("span");
  cpu.className = "op-runtime-health-detail__process-metric";
  cpu.textContent = formatRuntimeCpu(process.cpu_percent);
  row.appendChild(cpu);

  const memory = doc.createElement("span");
  memory.className = "op-runtime-health-detail__process-metric";
  memory.textContent = formatRuntimeMemory(process.memory_bytes);
  row.appendChild(memory);

  return row;
}

function runtimeHealthProcessHeat(cpu) {
  const value = Number(cpu);
  if (!Number.isFinite(value)) return "unknown";
  if (value >= 20) return "hot";
  if (value >= 5) return "warm";
  return "idle";
}

function runtimeHealthProcessLooksLikeAgent(process) {
  return ["agent", "codex", "claude", "docker"].includes(String(process?.role || ""));
}

function runtimeHealthDisplayProcesses(processes, sortMode = RUNTIME_HEALTH_DEFAULT_SORT) {
  if (!Array.isArray(processes) || processes.length === 0) return [];

  const byPid = new Map();
  for (const process of processes) {
    const pid = Number(process?.pid);
    if (Number.isFinite(pid)) byPid.set(pid, process);
  }

  const groups = new Map();
  const standalone = [];
  for (const process of processes) {
    if (!runtimeHealthProcessShouldGroup(process)) {
      standalone.push(process);
      continue;
    }
    const rootPid = runtimeHealthProcessGroupRootPid(process, byPid);
    if (!groups.has(rootPid)) groups.set(rootPid, []);
    groups.get(rootPid).push(process);
  }

  const display = [...standalone];
  for (const group of groups.values()) {
    display.push(runtimeHealthProcessGroupView(group));
  }
  display.sort((left, right) =>
    compareRuntimeHealthProcesses(left, right, runtimeHealthSortMode(sortMode)),
  );
  return display;
}

function runtimeHealthProcessShouldGroup(process) {
  return ["codex", "claude", "docker"].includes(String(process?.role || ""));
}

function runtimeHealthProcessGroupRootPid(process, byPid) {
  const role = String(process?.role || "");
  let current = process;
  const seen = new Set();
  while (current && !seen.has(Number(current.pid))) {
    seen.add(Number(current.pid));
    const parent = byPid.get(Number(current.parent_pid));
    if (!parent || String(parent.role || "") !== role) break;
    current = parent;
  }
  return Number(current?.pid ?? process?.pid);
}

function runtimeHealthProcessGroupView(group) {
  if (group.length <= 1) return group[0];
  const primary = [...group].sort(compareRuntimeHealthGroupPrimary)[0] || group[0];
  const cpuValues = group
    .map((process) => Number(process.cpu_percent))
    .filter((value) => Number.isFinite(value));
  const memoryValues = group
    .map((process) => Number(process.memory_bytes))
    .filter((value) => Number.isFinite(value));
  const memoryBytes =
    memoryValues.length > 0 ? memoryValues.reduce((sum, value) => sum + value, 0) : null;
  const focusWindowId =
    runtimeHealthFocusWindowId(primary) ||
    group.map(runtimeHealthFocusWindowId).find((windowId) => windowId) ||
    null;

  return {
    ...primary,
    name: runtimeHealthProcessRoleLabel(primary),
    cpu_percent:
      cpuValues.length > 0 ? cpuValues.reduce((sum, value) => sum + value, 0) : null,
    memory_bytes: memoryBytes,
    focus_window_id: focusWindowId,
    process_count: group.length,
  };
}

function compareRuntimeHealthGroupPrimary(left, right) {
  const cpuCompare =
    runtimeHealthNumeric(right.cpu_percent) - runtimeHealthNumeric(left.cpu_percent);
  if (cpuCompare !== 0) return cpuCompare;
  const memoryCompare =
    runtimeHealthNumeric(right.memory_bytes) - runtimeHealthNumeric(left.memory_bytes);
  if (memoryCompare !== 0) return memoryCompare;
  const wrapperCompare = runtimeHealthWrapperRank(left) - runtimeHealthWrapperRank(right);
  if (wrapperCompare !== 0) return wrapperCompare;
  return runtimeHealthNumeric(left.pid) - runtimeHealthNumeric(right.pid);
}

function compareRuntimeHealthProcesses(left, right, sortMode = RUNTIME_HEALTH_DEFAULT_SORT) {
  if (sortMode === "load") {
    const loadCompare = runtimeHealthProcessLoadScore(right) - runtimeHealthProcessLoadScore(left);
    if (loadCompare !== 0) return loadCompare;
  }
  if (sortMode === "mem") {
    const memoryCompare =
      runtimeHealthNumeric(right.memory_bytes) - runtimeHealthNumeric(left.memory_bytes);
    if (memoryCompare !== 0) return memoryCompare;
  }
  const cpuCompare =
    runtimeHealthNumeric(right.cpu_percent) - runtimeHealthNumeric(left.cpu_percent);
  if (cpuCompare !== 0) return cpuCompare;
  const memoryCompare =
    runtimeHealthNumeric(right.memory_bytes) - runtimeHealthNumeric(left.memory_bytes);
  if (memoryCompare !== 0) return memoryCompare;
  return runtimeHealthNumeric(left.pid) - runtimeHealthNumeric(right.pid);
}

function runtimeHealthWrapperRank(process) {
  const name = String(process?.name || "").toLowerCase();
  return name === "node" || name.startsWith("npm") || name.startsWith("npx") ? 1 : 0;
}

function runtimeHealthNumeric(value) {
  const number = Number(value);
  return Number.isFinite(number) ? number : 0;
}

function runtimeHealthProcessLoadScore(process) {
  return Math.max(
    runtimeHealthNumeric(process?.cpu_percent) / RUNTIME_HEALTH_LOAD_CPU_UNIT,
    runtimeHealthNumeric(process?.memory_bytes) / RUNTIME_HEALTH_LOAD_MEMORY_UNIT,
  );
}

function runtimeHealthCurrentSortMode(doc) {
  const cell = doc.getElementById("op-strip-runtime-health");
  return runtimeHealthSortMode(cell?.dataset.runtimeHealthSort);
}

function runtimeHealthSortMode(mode) {
  return RUNTIME_HEALTH_SORT_MODES.some((candidate) => candidate.mode === mode)
    ? mode
    : RUNTIME_HEALTH_DEFAULT_SORT;
}

function runtimeHealthSortLabel(mode) {
  const normalized = runtimeHealthSortMode(mode);
  return (
    RUNTIME_HEALTH_SORT_MODES.find((candidate) => candidate.mode === normalized)?.label || "Load"
  );
}

function runtimeHealthProcessRoleLabel(process) {
  const role = String(process?.role || "").trim();
  return role || "process";
}

function runtimeHealthProcessNameLabel(process) {
  const name = process?.name || "unknown";
  const count = Number(process?.process_count);
  return Number.isFinite(count) && count > 1 ? `${name} (${count} proc)` : name;
}

function runtimeHealthProcessPidLabel(process) {
  const pid = process?.pid ?? "--";
  const count = Number(process?.process_count);
  return Number.isFinite(count) && count > 1 ? `${pid}+${count - 1}` : pid;
}

function runtimeHealthProcessSummary(
  processes,
  rawProcesses,
  sortMode = RUNTIME_HEALTH_DEFAULT_SORT,
) {
  const sortLabel = runtimeHealthSortLabel(sortMode);
  if (rawProcesses.length !== processes.length) {
    return (
      `Showing ${processes.length} groups / ${rawProcesses.length} ` +
      `processes sorted by ${sortLabel}`
    );
  }
  return `Showing ${processes.length} processes sorted by ${sortLabel}`;
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
