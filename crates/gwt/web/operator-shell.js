// SPEC-2356 — Operator Design System: chrome shell wiring.
// Imports the small theme/hotkey modules and assembles the project bar
// theme toggle, hotkey overlay, mission briefing intro,
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
  safeWire("status strip clock", () => wireStatusStripClock({ doc, win }), markDegraded);
  if (shellDegraded) hideMissionBriefingImmediately(doc);
  else safeWire("mission briefing", () => wireMissionBriefing({ doc, win }), markDegraded);
  safeWire("hotkey overlay", () => wireHotkeyOverlay({ doc, hotkey }), markDegraded);
  safeWire("global hotkeys", () => wireGlobalHotkeys({ doc, hotkey }), markDegraded);

  return { themeManager, hotkey };
}

function safeWire(label, fn, onError) {
  try {
    fn();
  } catch (error) {
    onError(label, error);
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

export function wireStatusStripClock({ doc, win = doc.defaultView }) {
  const clock = doc.getElementById("op-strip-clock");
  if (!clock) return;

  // SPEC-2356 — render the clock with separately-styled colon glyphs so the
  // mission-control "blink" effect can target only the separators while the
  // numbers stay rock-steady. Clears + rebuilds via DOM nodes (no innerHTML
  // because the security hook flags textContent-as-template-string).
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
  let timer = null;
  const tick = () => {
    if (timer !== null) win.clearTimeout(timer);
    timer = null;
    if (doc.hidden) return;
    const now = new win.Date();
    setClock(now.getHours(), now.getMinutes(), now.getSeconds());
    // Re-align on every tick, including callbacks delayed by a busy renderer.
    timer = win.setTimeout(tick, 1000 - now.getMilliseconds());
  };
  doc.addEventListener("visibilitychange", tick);
  tick();
}

export function applyTelemetryCounts(doc, counts = {}) {
  const setText = (id, v) => {
    const el = doc.getElementById(id);
    if (el) el.textContent = String(v);
  };
  // SPEC-2356 — toggle the ERROR alert state so the ERROR cell pulses when a
  // runtime error needs attention, and stays still otherwise. FR-039 (anshin)
  // applies the same alert toggle to the WAITING cell so "agents waiting for
  // input" is just as loud as runtime errors.
  const toggleAlert = (modifier, count) => {
    const cell = doc.querySelector(`.op-status-strip__cell--${modifier}`);
    if (!cell) return;
    if ((count ?? 0) > 0) cell.classList.add("op-status-strip__cell--alert");
    else cell.classList.remove("op-status-strip__cell--alert");
  };
  if ("error" in counts) toggleAlert("error", counts.error);
  if ("waiting" in counts) toggleAlert("waiting", counts.waiting);
  // SPEC-2356 operator chrome cleanup: the dead Sidebar Layers rows and their
  // per-layer counters are removed; telemetry now lives solely in the Status
  // Strip cells below.
  setText("op-strip-running", counts.running ?? 0);
  // Issue #3884 AC-3: break out the running agents that live as inline
  // terminals in the Issue window (SPEC-3671 `issue_preview` placement), so
  // "RUNNING 6" next to an empty Agents view explains itself. Hidden when every
  // running agent is in Agents, and for callers that omit the breakdown.
  const runningInline = Number(counts.running_inline ?? 0);
  const inlineSlot = doc.getElementById("op-strip-running-inline");
  if (inlineSlot) {
    inlineSlot.textContent = `${runningInline} in Issues`;
    inlineSlot.hidden = !(runningInline > 0);
  }
  const runningCell = doc.querySelector(".op-status-strip__cell--running");
  if (runningCell) {
    runningCell.title =
      runningInline > 0
        ? `${runningInline} of ${Number(counts.running ?? 0)} running agents are inline terminals in the Issue window`
        : "";
  }
  setText("op-strip-idle", counts.idle ?? 0);
  // SPEC #5016 AC-HS1: the cell keeps its FR-039 `waiting` styling contract
  // and shows the server BLOCK count (agents that need the operator).
  setText("op-strip-waiting", counts.waiting ?? 0);
  setText("op-strip-error", counts.error ?? 0);
  if ("branches" in counts) setText("op-strip-branches", counts.branches ?? "—");
}

export function subscribeWorkAttentionSummary(doc, model) {
  return model.subscribeFrame(state => state?.projection?.attentionSummary?.summary ?? "", summary => {
    const cell = doc.querySelector(".op-status-strip__cell--waiting");
    if (cell) cell.title = summary;
  });
}

export function subscribeIssueMonitorStatus(doc, model) {
  let selected;
  let selectedKey;
  return model.subscribeFrame(state => {
    // SPEC #5016 AC-HS1/HS2: read only the model's combined attention value.
    const summary = state.decisionAttention?.summary;
    // Subscribe to the strip's presentation, not unrelated Monitor settings.
    const key = JSON.stringify([issueMonitorStatusStripView(state.status ?? {},
      state.counts ?? { queue: 0, active: 0, maxActive: 1 }), summary]);
    if (!selected || selectedKey !== key) {
      selectedKey = key;
      selected = { status: state.status, counts: state.counts, summary };
    }
    return selected;
  }, state => renderIssueMonitorStatus(doc, state, state.summary));
}

export function renderIssueMonitorStatus(doc, { status = {}, counts = { queue: 0, active: 0, maxActive: 1 } } = {}, attentionSummary = "") {
  const cell = doc.getElementById("op-strip-issue-monitor");
  const value = doc.getElementById("op-strip-issue-monitor-value");
  if (!cell || !value) return;

  wireIssueMonitorStatusCell(doc, cell);
  const view = issueMonitorStatusStripView(status, counts);
  value.textContent = view.value;
  const needsHuman = doc.getElementById("op-strip-needs-human");
  const needsHumanLabel = view.needsHuman == null ? "" : `Needs human ${view.needsHuman}`;
  if (needsHuman) {
    needsHuman.hidden = !needsHumanLabel;
    needsHuman.textContent = needsHumanLabel;
  }
  const attention = attentionSummary || needsHumanLabel;
  cell.dataset.state = view.state;
  cell.setAttribute("aria-label", [`Open Issues: ${view.value}`, attention].filter(Boolean).join(". "));
  cell.setAttribute("title", ["Open Issues", view.title, attention].filter(Boolean).join(" | "));
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

function normalizedIssueMonitorQuotaHold(status) {
  const quotaHold = status?.quota_hold;
  if (!quotaHold || typeof quotaHold !== "object" || Array.isArray(quotaHold)) {
    return null;
  }
  const provider =
    typeof quotaHold.provider === "string" ? quotaHold.provider.trim() : "";
  const resetAt =
    typeof quotaHold.reset_at === "string" ? quotaHold.reset_at.trim() : "";
  return provider && resetAt ? { provider, reset_at: resetAt } : null;
}

function issueMonitorStatusStripView(status, counts) {
  const enabled = Boolean(status.enabled);
  const quotaHold = normalizedIssueMonitorQuotaHold(status);
  const rawState = String(status.state || (enabled ? "idle" : "disabled"));
  let state = enabled ? rawState : "disabled";
  const quotaHoldEligible = [
    "quota_hold",
    "active",
    "launching",
    "settings_required",
    "idle",
  ].includes(rawState);
  if (enabled && quotaHold && quotaHoldEligible) {
    state = "quota_hold";
  } else if (state === "quota_hold") {
    state = "idle";
  }
  const { queue, active, maxActive } = counts;
  const needsHuman = status.needs_human_count ?? null;

  if (state === "disabled") {
    return {
      state,
      value: "Off",
      needsHuman,
      title: "Auto-run: Off",
      alert: false,
    };
  }

  let label = "Idle";
  if (state === "error") label = "Error";
  else if (state === "auth_required") label = "Sign-in needed";
  else if (state === "launching" || state === "active") label = "Running";
  else if (state === "quota_hold") label = "On hold";

  const value = `${label} · ${queue} queued · ${active} of ${maxActive} active`;
  const lastError = typeof status.last_error === "string" ? status.last_error.trim() : "";
  const quotaHoldDetail = state === "quota_hold"
    ? `Provider ${quotaHold.provider} | Reset ${quotaHold.reset_at}`
    : "";
  const detail = [quotaHoldDetail, lastError].filter(Boolean).join(" | ");
  return {
    state,
    value,
    needsHuman,
    title: detail ? `Auto-run: ${value} | ${detail}` : `Auto-run: ${value}`,
    alert: state === "error" || state === "auth_required",
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
const RUNTIME_HEALTH_CPU_UNITS =
  "Aggregate CPU is logical-core-normalized host share (0–100%); process rows use 1 core = 100%.";
let runtimeHealthHideTimer = null;

export function subscribeRuntimeHealth(doc, model, options = {}) {
  return model.subscribeFrame(snapshot => snapshot, snapshot => {
    if (snapshot) applyRuntimeHealth(doc, snapshot, options);
  });
}

export function applyRuntimeHealth(doc, snapshot = {}, options = {}) {
  const cell = doc.getElementById("op-strip-runtime-health");
  const value = doc.getElementById("op-strip-runtime-health-value");
  if (!cell || !value) return;

  const queue = snapshot.queue || {};
  const state = runtimeHealthState(snapshot.state);
  const stateLabel = runtimeHealthStateLabel(state);
  const cpuLabel = formatRuntimeAggregateCpu(snapshot.cpu_percent);
  const memoryLabel = formatRuntimeMemory(snapshot.memory_bytes);
  cell.dataset.state = state;
  cell.hidden = false;
  value.textContent = `${stateLabel} ${cpuLabel} ${memoryLabel}`;
  const accessibleLabel = `Runtime performance ${stateLabel} ${cpuLabel} ${memoryLabel}. ${RUNTIME_HEALTH_CPU_UNITS}`;
  cell.setAttribute("aria-label", accessibleLabel);
  value.setAttribute("aria-label", accessibleLabel);
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
      return "High";
    case "warn":
      return "Elevated";
    case "warming":
      return "Warming";
    default:
      return "OK";
  }
}

function formatRuntimeCpu(cpu) {
  if (typeof cpu !== "number" || !Number.isFinite(cpu)) return "--";
  return `${Math.round(cpu)}%`;
}

function formatRuntimeAggregateCpu(cpu) {
  if (typeof cpu !== "number" || !Number.isFinite(cpu)) return "--";
  return formatRuntimeCpu(Math.min(100, Math.max(0, cpu)));
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
    RUNTIME_HEALTH_CPU_UNITS,
    `State: ${runtimeHealthStateLabel(snapshot.state)}`,
    `Processes: ${Number(snapshot.process_count) || 0}`,
    `Runners: ${Number(snapshot.runner_count) || 0}`,
    `Queued events: ${Number(queue.queued_entries) || 0}`,
    `Dropped events: ${signedDelta(queue.dropped_lossy_delta)}`,
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
  appendRuntimeHealthChip(doc, summary, "State", runtimeHealthStateLabel(snapshot.state));
  appendRuntimeHealthChip(doc, summary, "CPU", formatRuntimeAggregateCpu(snapshot.cpu_percent));
  appendRuntimeHealthChip(doc, summary, "Memory", formatRuntimeMemory(snapshot.memory_bytes));
  appendRuntimeHealthChip(
    doc,
    summary,
    "Processes",
    `${Number(snapshot.process_count) || 0}/${Number(snapshot.runner_count) || 0}`,
  );
  detail.appendChild(summary);

  const units = doc.createElement("p");
  units.className = "op-runtime-health-detail__units";
  units.textContent = RUNTIME_HEALTH_CPU_UNITS;
  detail.appendChild(units);

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

  const reduced = matchReduced(doc);
  const totalDelay = reduced.matches ? 250 : 1450;

  overlay.removeAttribute("aria-hidden");
  overlay.hidden = false;

  // SPEC-2356 — let the user dismiss the splash early by pressing any
  // key or clicking through it. The splash is purely decorative and
  // shouldn't block users who want to get into the workspace immediately.
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
// Global hotkeys (delegate to operator command bus)
// ------------------------------------------------------------

function wireGlobalHotkeys({ doc, hotkey }) {
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

// The project execution owner is independent of the browser's GUI connection.
export function subscribeProjectOwnerHealth(doc, model, { send, createFocusTrap = () => () => {} } = {}) {
  const cell = doc.querySelector("[data-project-owner-health]");
  let health = null, confirmationId = null, pending = false, releaseTrap = null, returnFocus = null;
  const node = (tag, className, text) => {
    const element = doc.createElement(tag);
    element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  };
  const restart = node("button", "text-button", "Restart daemon…");
  restart.type = "button";
  restart.dataset.ownerRestart = "";
  restart.hidden = true;
  cell?.append(restart);
  const modal = node("div", "modal-backdrop");
  modal.id = "owner-restart-modal";
  modal.setAttribute("aria-hidden", "true");
  const dialog = node("div", "modal-shell");
  dialog.setAttribute("role", "dialog");
  dialog.setAttribute("aria-modal", "true");
  dialog.setAttribute("aria-labelledby", "owner-restart-title");
  dialog.setAttribute("aria-describedby", "owner-restart-description");
  dialog.tabIndex = -1;
  const header = node("div", "modal-header");
  const title = node("h2", "", "Restart unresponsive daemon?");
  title.id = "owner-restart-title";
  header.append(title);
  const body = node("div", "modal-body");
  body.id = "owner-restart-description";
  body.textContent = "Restarting stops all agents hosted by this daemon, including agents in other open projects. Diagnostics are saved first. GWT then sends SIGTERM and, if the daemon does not exit during the grace period, SIGKILL. Unsaved terminal work may be lost.";
  const footer = node("div", "modal-footer");
  const cancel = node("button", "text-button", "Cancel");
  cancel.type = "button";
  cancel.dataset.ownerRestartCancel = "";
  const confirm = node("button", "wizard-button primary destructive", "Save diagnostics and restart");
  confirm.type = "button";
  confirm.dataset.ownerRestartConfirm = "";
  footer.append(cancel, confirm);
  dialog.append(header, body, footer);
  modal.append(dialog);
  doc.body.append(modal);
  function hide() {
    confirmationId = null;
    modal.classList.remove("open");
    modal.setAttribute("aria-hidden", "true");
    releaseTrap?.(); releaseTrap = null;
    returnFocus?.focus?.(); returnFocus = null;
  }
  restart.addEventListener("click", () => {
    if (health?.state !== "wedged" || !health.recovery_id || pending || !send) return;
    confirmationId = health.recovery_id;
    returnFocus = doc.activeElement;
    modal.classList.add("open");
    modal.setAttribute("aria-hidden", "false");
    releaseTrap = createFocusTrap(dialog, { document: doc });
    cancel.focus();
  });
  cancel.addEventListener("click", hide);
  modal.addEventListener("click", event => { if (event.target === modal) hide(); });
  const onKey = event => {
    if (event.key === "Escape" && modal.classList.contains("open")) {
      event.preventDefault(); event.stopPropagation(); hide();
    }
  };
  doc.addEventListener("keydown", onKey);
  confirm.addEventListener("click", () => {
    const current = model.read();
    if (!modal.classList.contains("open") || current?.state !== "wedged" || !confirmationId || current.recovery_id !== confirmationId || pending || !send) return;
    const recoveryId = confirmationId;
    pending = true;
    restart.disabled = true;
    hide();
    if (send({ kind: "restart_wedged_owner", confirmed: true, recovery_id: recoveryId }) !== "sent") {
      pending = false;
      restart.disabled = false;
    }
  });
  const unsubscribe = model.subscribeFrame(state => state, current => {
    const identityChanged = health?.recovery_id !== current?.recovery_id;
    health = current;
    if (health) applyProjectOwnerHealth(doc, health);
    restart.hidden = health?.state !== "wedged" || !health?.recovery_id || !send;
    // A backend refusal keeps the owner wedged. A new observation reopens
    // the explicit confirmation path; the backend consumes each token once.
    pending = false;
    restart.disabled = false;
    if (health?.state !== "wedged" || identityChanged) hide();
  });
  return () => {
    unsubscribe(); hide();
    doc.removeEventListener("keydown", onKey);
    restart.remove(); modal.remove();
  };
}

function applyProjectOwnerHealth(doc, health = {}) {
  const cell = doc.querySelector("[data-project-owner-health]");
  if (!cell) return;
  const state = ["healthy", "degraded", "wedged"].includes(health.state) ? health.state : "connecting";
  cell.dataset.state = state;
  cell.hidden = false;
  cell.title = health.detail || "";
  const value = cell.querySelector("[data-project-owner-health-value]");
  if (value) value.textContent = state === "healthy" ? "Ready" : state === "wedged" ? "Not responding" : state === "degraded" ? "Unavailable" : "Connecting";
}
