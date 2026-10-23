import { bindControlExplanations, setControlExplanation } from "./control-explanations.js";
import { createUiStateStore } from "./ui-state-store.js";
import { renderUiContent } from "./ui-content.js";
// SPEC-3064 Phase 3 (E6d) — Knowledge Bridge (Work Item Kanban)
// window surface extracted from app.js. Owns the per-window knowledge
// bridge state map (cache-backed entries, semantic search coalescing,
// detail correlation, auto-refresh timer, kanban hide-done preference),
// the Kanban rendering (columns, cards, drag targets, detail pane), the
// Kanban Drawer (slide-over detail with focus trap), the Knowledge window
// mount, and the knowledge_* receive() bodies. Pure movement from app.js:
// behavior, DOM output, and WS protocol are unchanged; the moved code
// keeps its original app.js indentation. Textual changes are limited to:
// in-module self-references through
// `*` became direct local calls
// (persistKanbanHideDone → writeKanbanHideDonePreference) and the mount's
// focus_window send goes through sendWindowFocus.
//
// deps:
// - send(message): forward a frontend event over the WebSocket bridge.
// - createNode: shared DOM helper owned by app.js.
// - windowMap / workspaceWindowById / getWorkspaceWindows: workspace window lookups.
// - pendingIndexOpenTargetsByPreset: index-open handoff targets by preset.
// - knowledgeKindForPreset(preset): Issue/SPEC kind mapping.
// - focusWindowLocally(windowId) / sendWindowFocus(windowId): focus paths.
// - focusOrSpawnPreset(preset): focus-or-spawn used by drawer actions.
// - openIssueLaunchWizard(windowId, issueNumber): launch wizard entry.
// - visibleBounds(): current canvas bounds for resume placement.
// - launchPending: shared Resume/Launch pending controller.
import { createFocusTrap } from "/focus-trap.js";
import { createLaunchOperationId } from "./launch-pending-controller.js";

// Keep only the visible window of large collections. Heights are measured per
// Issue identity; estimates apply only to rows that have never been mounted.
const keyedKnowledgeLists = new WeakMap();
function reconcileKnowledgeList(host, entries, options) {
  let view = keyedKnowledgeLists.get(host);
  if (!view) {
    const doc = host.ownerDocument;
    const cache = new Map();
    const heights = new Map();
    let items = [], settings, offsets = [], forced = -1, measuredWidth = null;
    const spacer = () => {
      const node = doc.createElement("div");
      node.className = "knowledge-virtual-spacer";
      node.setAttribute("aria-hidden", "true");
      return node;
    };
    function draw(measure = true) {
      const root = settings.scrollRoot;
      const virtual = settings.virtualize;
      if (measuredWidth !== root.clientWidth) heights.clear();
      measuredWidth = root.clientWidth;
      const gap = Number.parseFloat(doc.defaultView?.getComputedStyle?.(host).rowGap) || 0;
      offsets = [0];
      for (const entry of items) offsets.push(offsets.at(-1) + (heights.get(entry.number) || 160) + gap);
      const relativeTop = (host.getBoundingClientRect?.().top || 0) - (root.getBoundingClientRect?.().top || 0);
      const top = Math.max(0, root === host ? root.scrollTop || 0 : -relativeTop);
      const bottom = top + (root.clientHeight || 800);
      const active = doc.activeElement;
      const focusedNumber = Number(active?.closest?.("[data-issue-number]")?.dataset.issueNumber);
      const wanted = [];
      for (let index = 0; index < items.length; index++) {
        if (!virtual || (offsets[index + 1] >= top - 640 && offsets[index] <= bottom + 640) ||
            items[index].number === focusedNumber || index === forced) wanted.push(index);
      }
      const nodes = [];
      let changed = false;
      let previous = 0;
      const addSpace = (start, end) => {
        if (end <= start) return;
        // Reuse spacers as well so an identical update makes no DOM mutations.
        const node = [...host.children].find(child => child.dataset.virtualRange === `${start}:${end}`) || spacer();
        node.dataset.virtualRange = `${start}:${end}`;
        const height = `${Math.max(0, offsets[end] - offsets[start] - gap)}px`;
        if (node.style.height !== height) node.style.height = height;
        nodes.push(node);
      };
      const retained = new Set();
      for (const index of wanted) {
        if (virtual) addSpace(previous, index);
        const entry = items[index];
        const version = settings.version(entry);
        let saved = cache.get(entry.number);
        if (!saved || saved.version !== version) {
          changed = true;
          const node = settings.render(entry);
          if (saved?.node.querySelector(".knowledge-row-menu[open]")) node.querySelector(".knowledge-row-menu")?.setAttribute("open", "");
          const focusIndex = saved?.node === active ? -2 : saved?.node.contains(active)
            ? [...saved.node.querySelectorAll("button, input, summary, [tabindex]")].indexOf(active) : -1;
          saved = { node, version, focusIndex };
          cache.set(entry.number, saved);
        }
        nodes.push(saved.node);
        settings.update?.(saved.node, entry);
        retained.add(entry.number);
        previous = index + 1;
      }
      if (virtual) addSpace(previous, items.length);
      const keep = new Set(nodes);
      for (const child of [...host.childNodes]) if (!keep.has(child)) { child.remove(); changed = true; }
      let cursor = host.firstChild;
      for (const node of nodes) {
        if (node === cursor) cursor = cursor.nextSibling;
        else host.insertBefore(node, cursor);
      }
      while (cursor) { const next = cursor.nextSibling; cursor.remove(); cursor = next; }
      let resized = false;
      for (const [number, saved] of cache) {
        if (!retained.has(number)) { cache.delete(number); continue; }
        const height = saved.node.getBoundingClientRect?.().height;
        if (height > 0) {
          const margin = Number.parseFloat(doc.defaultView?.getComputedStyle?.(saved.node).marginBottom) || 0;
          resized ||= heights.get(number) !== height + margin;
          heights.set(number, height + margin);
        }
        if (saved.focusIndex === -2) {
          saved.node.focus({ preventScroll: true });
          saved.focusIndex = -1;
        } else if (saved.focusIndex >= 0) {
          saved.node.querySelectorAll("button, input, summary, [tabindex]")[saved.focusIndex]?.focus({ preventScroll: true });
          saved.focusIndex = -1;
        }
      }
      if (active?.isConnected && host.contains(active) && doc.activeElement !== active) active.focus?.({ preventScroll: true });
      if (changed) settings.mounted?.();
      if (virtual && resized && measure) draw(false);
    }
    const root = options.scrollRoot;
    const onScroll = () => { if (settings.virtualize && items.length) draw(); };
    root.addEventListener("scroll", onScroll, { passive: true });
    // Native Tab crosses virtual boundaries just as it crosses mounted rows.
    host.addEventListener("keydown", event => {
      if (!settings.virtualize || event.key !== "Tab" || event.ctrlKey || event.altKey || event.metaKey) return;
      const row = event.target.closest?.("[data-issue-number]");
      if (!row) return;
      const controls = [...(row.matches("button") ? [row] : []), ...row.querySelectorAll("button, input, summary, [tabindex]")]
        .filter(node => !node.disabled && node.tabIndex >= 0 && node.getClientRects().length);
      const direction = event.shiftKey ? -1 : 1;
      if (event.target !== controls[direction > 0 ? controls.length - 1 : 0]) return;
      const index = items.findIndex(entry => String(entry.number) === row.dataset.issueNumber) + direction;
      if (index < 0 || index >= items.length) return;
      event.preventDefault();
      view.reveal(items[index].number);
      const target = cache.get(items[index].number)?.node;
      const next = [...(target?.matches("button") ? [target] : []), ...(target?.querySelectorAll("button, input, summary, [tabindex]") || [])]
        .filter(node => !node.disabled && node.tabIndex >= 0 && node.getClientRects().length);
      next[direction > 0 ? 0 : next.length - 1]?.focus();
    });
    const observer = typeof ResizeObserver === "function" ? new ResizeObserver(onScroll) : null;
    observer?.observe(root);
    view = {
      update(next, config) {
        items = next; settings = config;
        const keys = new Set(items.map(entry => entry.number));
        for (const key of heights.keys()) if (!keys.has(key)) heights.delete(key);
        draw();
      },
      reveal(number) {
        forced = items.findIndex(entry => entry.number === number);
        if (forced < 0) return false;
        draw();
        cache.get(number)?.node.scrollIntoView?.({ block: "nearest", inline: "nearest" });
        forced = -1;
        return true;
      },
      dispose() { root.removeEventListener("scroll", onScroll); observer?.disconnect(); keyedKnowledgeLists.delete(host); },
    };
    keyedKnowledgeLists.set(host, view);
  }
  view.update(entries, options);
}

const MONITOR_STATE_VIEWS = Object.freeze({
  queued: Object.freeze({ label: "Queued", tone: "idle" }),
  not_ready: Object.freeze({ label: "Not ready", tone: "needs-input" }),
  hold_excluded: Object.freeze({ label: "On hold", tone: "needs-input" }),
  launching: Object.freeze({ label: "Launching", tone: "active" }),
  launched: Object.freeze({ label: "Launched", tone: "active" }),
  merged: Object.freeze({ label: "Merged", tone: "done" }),
  released: Object.freeze({ label: "Released", tone: "done" }),
  launch_failed: Object.freeze({ label: "Launch failed", tone: "blocked" }),
  agent_failed: Object.freeze({ label: "Agent failed", tone: "blocked" }),
  blocked_by_claim: Object.freeze({ label: "Blocked by claim", tone: "needs-input" }),
  blocked_by_dependency: Object.freeze({ label: "Blocked by dependency", tone: "needs-input" }),
  skipped: Object.freeze({ label: "Skipped", tone: "idle" }),
  needs_human: Object.freeze({ label: "Needs human", tone: "needs-input" }),
});

export function monitorStateView(value) {
  const state = typeof value === "string" ? value.trim().toLowerCase() : "";
  if (!state) return null;
  const known = MONITOR_STATE_VIEWS[state];
  return known
    ? { state, label: known.label, tone: known.tone }
    : { state, label: `Unknown (${state})`, tone: "needs-input" };
}

// SPEC-3671 FR-011: an auto-launched agent that errors or waits for a human ruling
// announces itself through this badge. It never opens a canvas window.
const ISSUE_PREVIEW_STATUS_VIEWS = Object.freeze({
  running: Object.freeze({ label: "Running", tone: "active" }),
  starting: Object.freeze({ label: "Starting", tone: "active" }),
  idle: Object.freeze({ label: "Idle", tone: "idle" }),
  waiting: Object.freeze({ label: "Needs input", tone: "needs-input" }),
  stopped: Object.freeze({ label: "Stopped", tone: "idle" }),
  error: Object.freeze({ label: "Error", tone: "blocked" }),
});

// Issue #3884: compact elapsed-time label for the Issue row status row
// ("<1m", "7m", "1h 05m", "1d 2h"); empty when the duration is unknown.
export function formatAgentElapsed(ms) {
  if (ms === null || ms === undefined || ms === "") return "";
  const value = Number(ms);
  if (!Number.isFinite(value) || value < 0) return "";
  const minutes = Math.floor(value / 60000);
  if (minutes < 1) return "<1m";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${String(minutes % 60).padStart(2, "0")}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

// SPEC #3885 T-020: the agent's own start time, broadcast by the backend with
// the window, is the elapsed clock. `windowRuntimeStateSince` only knows when
// this frontend last saw the state change, so it restarts at every reload and
// at every state transition; it stays the fallback for a window whose runtime
// the backend cannot name (a restored window with no live PTY).
function issueAgentElapsedMs(windowData, observedSince, now = Date.now()) {
  const started = Number(windowData?.runtime_started_at_ms);
  const since = Number.isFinite(started) && started > 0 ? started : Number(observedSince);
  if (!Number.isFinite(since) || since <= 0) return null;
  return Math.max(0, now - since);
}

function issuePreviewStatusView(windowData) {
  const status = String(windowData?.status || "").trim().toLowerCase();
  const known = ISSUE_PREVIEW_STATUS_VIEWS[status];
  return known
    ? { status, label: known.label, tone: known.tone }
    : { status, label: status ? `Unknown (${status})` : "Unknown", tone: "needs-input" };
}

function normalizeWorkBranch(value) {
  const text = String(value || "").trim();
  if (!text) return "";
  return text.replace(/^refs\/heads\//, "").replace(/^origin\//, "");
}

// SPEC-3671 FR-012: join an Issue row to the active Work projection the frontend
// already receives. The Issue row carries only the backend's correlation
// (`related_work_refs`); every displayed field stays owned by the projection, so
// there is no second derivation of Work state and no new data path.
export function issueWorkRowForEntry(projection, entry) {
  const refs = Array.isArray(entry?.related_work_refs) ? entry.related_work_refs : [];
  if (refs.length === 0) return null;
  const works = Array.isArray(projection?.active_works) ? projection.active_works : [];
  if (works.length === 0) return null;
  const refIds = new Set(refs.map((ref) => ref?.id).filter(Boolean));
  const byId = works.find((work) => refIds.has(work?.id));
  if (byId) return byId;
  const refBranches = new Set(
    refs.map((ref) => normalizeWorkBranch(ref?.branch)).filter(Boolean),
  );
  if (refBranches.size === 0) return null;
  return works.find((work) => refBranches.has(normalizeWorkBranch(work?.branch))) || null;
}

// SPEC-3671 FR-007 / FR-009: the previews the given Issue window is responsible for
// mirroring. A preview whose host Issue window no longer exists is adopted by any Issue
// window so an auto-launched agent is never left unreachable.
export function issuePreviewWindowsForIssue(windows, issueWindowId, issueNumber) {
  const number = Number(issueNumber);
  if (!Number.isFinite(number)) return [];
  const list = Array.isArray(windows) ? windows : [];
  const knownIds = new Set(list.map((windowData) => windowData?.id).filter(Boolean));
  return list.filter((windowData) => {
    const placement = windowData?.placement;
    if (placement?.kind !== "issue_preview") return false;
    if (Number(placement.issue_number) !== number) return false;
    return (
      placement.issue_window_id === issueWindowId ||
      !knownIds.has(placement.issue_window_id)
    );
  });
}

// SPEC #3885 Phase 2 (T-004 / FR-006): the Issue row state model. A row shows
// exactly one primary badge, at most two pieces of secondary information, and at
// most two visible actions; the remaining actions go to the row's overflow menu.
// The model is pure so the limits are testable without the DOM.
const ISSUE_ROW_SECONDARY_LIMIT = 2;
const ISSUE_ROW_ACTION_LIMIT = 2;
const ISSUE_ROW_LIVE_AGENT_STATUSES = new Set(["running", "starting", "idle", "waiting", "error"]);
// SPEC #3885 FR-015: an agent that is still running can be stopped; one that
// already exited or errored offers RESTART in its window chrome instead.
const ISSUE_ROW_STOPPABLE_AGENT_STATUSES = new Set([
  "running",
  "starting",
  "idle",
  "waiting",
]);
const ISSUE_ROW_LAUNCH_NOW_STATES = new Set(["queued", "launch_failed", "agent_failed"]);
// Issue #3628 (AC-3): the states that hold a row out of the queue. Launch Now
// only opens the wizard and never touches the hold, so returning a row to the
// queue *without* starting an agent had no control at all and meant hand-editing
// issue-monitor.json. Offered only where such a hold exists, so the button never
// promises a change that cannot happen.
const ISSUE_ROW_REQUEUE_STATES = new Set(["launch_failed", "agent_failed"]);
const ISSUE_ROW_WORK_LANE_VIEWS = Object.freeze({
  closed: Object.freeze({ label: "Done", tone: "done" }),
  remote: Object.freeze({ label: "Remote", tone: "remote" }),
  needs_attention: Object.freeze({ label: "Needs attention", tone: "needs-input" }),
  running: Object.freeze({ label: "Active", tone: "active" }),
  paused: Object.freeze({ label: "Paused", tone: "idle" }),
});

function issueEntryStateKey(entry) {
  return String(entry?.state || "open").toLowerCase() === "closed" ? "closed" : "open";
}

// Membership drives queue commands; the displayed column additionally reads lifecycle.
export function isIssueInTerminalQueue(entry) {
  return Number.isFinite(entry?.queue_position);
}

function issueStopReason(entry) {
  const message = String(entry?.error_message || "").trim();
  return entry?.monitor_state === "needs_human" && message.startsWith("stopped: ")
    ? message.slice("stopped: ".length) : null;
}

function issueLaunchAttemptsExhausted(entry) {
  return entry?.monitor_state === "needs_human" &&
    /^autonomous launch attempts exhausted \(\d+\/\d+\) with an unchanging failure:/.test(String(entry?.error_message || ""));
}

function issueLaunchRecovery(entry) {
  if (!["queued", "launch_failed", "agent_failed", "needs_human"].includes(entry?.monitor_state)) return "";
  const message = String(entry?.error_message || "");
  if (/has invalid lineage|unrelated Work lineage/.test(message)) {
    return "Work lineage · Next: workspace.close (superseded), then issue.monitor.requeue";
  }
  if (/owner mismatch|owner or execution container mismatch/.test(message)) {
    return "Work owner mismatch · Next: workspace.candidates";
  }
  if (message.includes("Launch timed out: no window was created within")) {
    return "Window timeout · Next: issue.monitor.requeue";
  }
  if (message.trim() && (["launch_failed", "agent_failed"].includes(entry.monitor_state) || issueLaunchAttemptsExhausted(entry))) {
    return `${entry.monitor_state === "agent_failed" ? "Agent failure" : "Launch failure"} · Next: issue.monitor.status`;
  }
  return "";
}

export function issueQueueColumn(entry, work = null, liveWindow = null) {
  if (issueEntryStateKey(entry) === "closed" || ["merged", "released"].includes(entry.monitor_state)) {
    // Closing an Issue does not stop its agent or release a monitor launch.
    const live = liveWindow && ISSUE_ROW_STOPPABLE_AGENT_STATUSES.has(issuePreviewStatusView(liveWindow).status);
    return live || Number(work?.active_agents) > 0 || ["launching", "launched"].includes(entry.monitor_state)
      ? "active" : "done";
  }
  if (issueStopReason(entry) !== null) return "backlog";
  if (liveWindow && ISSUE_ROW_STOPPABLE_AGENT_STATUSES.has(issuePreviewStatusView(liveWindow).status)) return "active";
  // Judgment holds are not live launches; Work activity can outlast the agent.
  if (["needs_human", "blocked_by_claim", "launch_failed", "agent_failed"].includes(entry.monitor_state)) return "backlog";
  if (["launching", "launched"].includes(entry.monitor_state)) return "active";
  if (work?.status_category === "blocked") return "backlog";
  if (Number(work?.active_agents) > 0) return "active";
  return isIssueInTerminalQueue(entry) ? "queued" : "backlog";
}

function issueEntryHasLabel(entry, name) {
  const labels = Array.isArray(entry?.labels) ? entry.labels : [];
  return labels.some((label) => String(label || "").trim().toLowerCase() === name);
}

function issueRowPrimaryView({ entry, attention, inlineWindow, canvasWindow }) {
  if (issueStopReason(entry) !== null) return { key: "monitor:held", label: "Held", tone: "idle" };
  const live = inlineWindow || canvasWindow;
  if (live) {
    const view = issuePreviewStatusView(live);
    if (ISSUE_ROW_LIVE_AGENT_STATUSES.has(view.status)) {
      return { key: `agent:${view.status}`, label: view.label, tone: view.tone };
    }
  }
  const monitor = monitorStateView(entry?.monitor_state);
  if (monitor) {
    return { key: `monitor:${monitor.state}`, label: monitor.label, tone: monitor.tone };
  }
  const lane = ISSUE_ROW_WORK_LANE_VIEWS[attention?.lane];
  if (lane) {
    return { key: `work:${attention.lane}`, label: lane.label, tone: lane.tone };
  }
  return issueEntryStateKey(entry) === "closed"
    ? { key: "issue:closed", label: "Closed", tone: "done" }
    : { key: "issue:open", label: "Open", tone: "idle" };
}

function issueQueuePriorityLabel(entry) {
  if (entry.priority_reason === "pm_demoted") return "Normal · PM demoted";
  if (entry.priority_reason === "urgent_limit_reached") return "Normal · Urgent limit reached";
  return entry.priority === "urgent" ? "Urgent" : "Normal";
}

function issueRowSecondaryItems({ entry, work, attention, primary, inlineWindow, canvasWindow }) {
  const items = [];
  if (issueEntryStateKey(entry) === "closed" && primary.key !== "issue:closed") {
    items.push({ kind: "chip", key: "closed", label: "Closed" });
  }
  const exclusion = String(entry?.exclusion_reason || "").trim();
  const stopReason = issueStopReason(entry);
  const recovery = stopReason === null ? issueLaunchRecovery(entry) : "";
  const failureReason = stopReason ?? (["launch_failed", "agent_failed", "needs_human"].includes(entry?.monitor_state) || recovery
    ? String(entry?.error_message || "").trim() : "");
  const attentionReason =
    attention?.lane === "needs_attention" ? String(attention.reason || "").trim() : "";
  const cleanupPending = (issueEntryStateKey(entry) === "closed" || ["merged", "released"].includes(entry?.monitor_state)) &&
    issueQueueColumn(entry, work, inlineWindow || canvasWindow) === "active";
  const reason = cleanupPending ? "Session cleanup pending"
    : stopReason ?? (entry?.readiness_diagnosis || failureReason || exclusion || attentionReason);
  if (reason) {
    items.push({ kind: "reason", key: "reason", label: reason });
  }
  if (recovery) items.push({ kind: "reason", key: "recovery", label: recovery, title: recovery });
  if (Number.isFinite(entry?.queue_position)) {
    const terminal = String(entry?.queue_terminal || "").trim();
    items.push({
      kind: "chip",
      key: "queue",
      label: `Queue ${entry.queue_position}${terminal ? ` · ${terminal}` : ""}${entry.queued_by ? ` · ${entry.queued_by}` : ""}`,
    });
    if (entry.priority === "urgent" || entry.priority_reason === "pm_demoted" ||
        entry.priority_reason === "urgent_limit_reached") {
      items.push({ kind: "chip", key: "queue-priority", label: issueQueuePriorityLabel(entry) });
    }
  }
  if (work?.pr_number) {
    const prState = String(work.pr_state || "").trim();
    items.push({
      kind: "chip",
      key: "pr",
      label: prState ? `PR #${work.pr_number} · ${prState}` : `PR #${work.pr_number}`,
      title: work.pr_url || "",
    });
  }
  if (entry?.is_spec || issueEntryHasLabel(entry, "gwt-spec")) {
    items.push({ kind: "chip", key: "spec", label: "Spec" });
  }
  if (issueEntryHasLabel(entry, "auto-merge")) {
    items.push({ kind: "chip", key: "auto-merge", label: "Auto-merge" });
  }
  return items.slice(0, ISSUE_ROW_SECONDARY_LIMIT);
}

function issueRowActionOrder({ entry, work, attention, inlineWindow, canvasWindow }) {
  const workActions = ["continue-work", "resume-work", "cleanup-work"];
  // SPEC #3885 FR-015: stopping the agent is always last and always in the
  // overflow menu, so a live run is never one stray click away from ending.
  if (canvasWindow) {
    return {
      order: ["focus-canvas-window", "configure-issue", ...workActions, "stop-agent"],
      limit: 1,
    };
  }
  if (inlineWindow) {
    return {
      order: ["windowize-issue-preview", "configure-issue", ...workActions, "stop-agent"],
      limit: 1,
    };
  }
  const monitor = monitorStateView(entry?.monitor_state);
  switch (monitor?.state) {
    case "queued":
      return {
        order: ["launch-now", "configure-issue", "queue-remove", "move-up", "move-down", ...workActions],
      };
    case "launch_failed":
    case "agent_failed":
      return {
        order: [
          "launch-now",
          "requeue-issue",
          "continue-work",
          "resume-work",
          "configure-issue",
          "cleanup-work",
        ],
      };
    case "merged":
    case "released":
      return { order: ["cleanup-work", "resume-work", "continue-work", "configure-issue"] };
    case "needs_human":
    case "launching":
    case "launched":
      return { order: ["continue-work", "resume-work", "configure-issue", "cleanup-work"] };
    default:
      break;
  }
  if (monitor) {
    return { order: ["configure-issue", ...workActions] };
  }
  if (work) {
    return {
      order:
        attention?.lane === "closed"
          ? ["cleanup-work", "resume-work", "continue-work"]
          : workActions,
    };
  }
  // SPEC #3165 TQ-9: a Backlog Issue is the one the user puts into the queue.
  // This is the requested feature's main direction, so it sits on the row next
  // to "Launch agent" rather than behind a separate surface.
  return {
    order: issueEntryStateKey(entry) === "open" ? ["queue-push", "launch-agent"] : [],
  };
}

function issueRowActionAvailable(action, { entry, work, queue, inlineWindow, canvasWindow }) {
  const monitor = monitorStateView(entry?.monitor_state);
  switch (action) {
    case "stop-agent": {
      const live = inlineWindow || canvasWindow;
      return (
        Boolean(live) &&
        ISSUE_ROW_STOPPABLE_AGENT_STATUSES.has(issuePreviewStatusView(live).status)
      );
    }
    case "launch-now":
      return ISSUE_ROW_LAUNCH_NOW_STATES.has(monitor?.state);
    case "queue-push":
      return issueEntryStateKey(entry) === "open" && !monitor;
    case "queue-remove":
      return Boolean(Number.isFinite(entry?.queue_position));
    case "requeue-issue":
      return ISSUE_ROW_REQUEUE_STATES.has(monitor?.state);
    case "configure-issue":
      return Boolean(monitor);
    case "move-up":
    case "move-down":
      return Boolean(queue) && Number.isFinite(queue.index) && queue.index >= 0;
    case "continue-work":
    case "resume-work":
      return Boolean(work);
    case "cleanup-work":
      return Boolean(work && (work.cleanup_candidate || work.cleanup_blocked_reason));
    case "open-window":
      return Boolean(inlineWindow || canvasWindow);
    case "move-to-top":
      return Boolean(queue) && Number.isFinite(queue.index) && queue.index >= 0;
    case "open-pr":
      return Boolean(String(work?.pr_url || "").trim());
    default:
      return true;
  }
}

export function issueRowStateModel({
  entry,
  work = null,
  attention = null,
  inlineWindow = null,
  canvasWindow = null,
  queue = null,
} = {}) {
  const context = { entry, work, attention, inlineWindow, canvasWindow, queue };
  const primary = issueRowPrimaryView(context);
  const secondary = issueRowSecondaryItems({ ...context, primary });
  const { order, limit = ISSUE_ROW_ACTION_LIMIT } = issueRowActionOrder(context);
  const available = order.filter((action) => issueRowActionAvailable(action, context));
  return {
    primary,
    secondary,
    actions: available.slice(0, limit),
    overflow: available.slice(limit),
  };
}

// SPEC #3885 Phase 5 (T-035 / FR-023): the detail pane's AC progress. Only the
// `- [ ] AC-N:` checklist lines count — the same shape the Issue Monitor's
// readiness gate reads — so the gauge never disagrees with "Missing AC".
const ISSUE_ACCEPTANCE_LINE = /^\s*[-*]\s+\[([ xX])\]\s+(AC-\d+):\s*(.*)$/;

export function issueAcceptanceProgress(markdown) {
  const items = [];
  for (const line of String(markdown || "").split(/\r?\n/)) {
    const match = ISSUE_ACCEPTANCE_LINE.exec(line);
    if (!match) continue;
    items.push({ id: match[2], done: match[1] !== " ", text: match[3].trim() });
  }
  return { items, done: items.filter((item) => item.done).length, total: items.length };
}

// SPEC #3885 Phase 5 (T-035 / FR-023): one action band per state, at most three
// visible actions plus ⋯. queue = Launch now / Move to top, running = Open
// window / Move to top (⋯ keeps Requeue and Stop), done = Open PR. Every other
// state falls back to the row's own order. Only actions the row model would
// allow are offered, so the band never shows a button that cannot act.
const ISSUE_DETAIL_ACTION_LIMIT = 3;
const ISSUE_DETAIL_TERMINAL_ACTIONS = new Set(["windowize-issue-preview", "focus-canvas-window"]);

function issueDetailPhase({ entry, inlineWindow, canvasWindow }) {
  const monitor = monitorStateView(entry?.monitor_state)?.state;
  if (inlineWindow || canvasWindow || monitor === "launching" || monitor === "launched") {
    return "running";
  }
  if (monitor === "queued") return "queue";
  if (monitor === "merged" || monitor === "released" || issueEntryStateKey(entry) === "closed") {
    return "done";
  }
  return "other";
}

export function issueDetailActionModel(context = {}) {
  const phase = issueDetailPhase(context);
  const row = issueRowStateModel(context);
  const rowActions = [...row.actions, ...row.overflow].filter(
    (action) => !ISSUE_DETAIL_TERMINAL_ACTIONS.has(action),
  );
  // The detail pane has always offered Launch agent for an open Issue with no
  // agent; the fallback band keeps it even where the row itself omits it.
  if (phase === "other" && issueEntryStateKey(context.entry) === "open" &&
      !rowActions.includes("launch-agent")) {
    rowActions.push("launch-agent");
  }
  const preferred =
    phase === "queue"
      ? ["launch-now", "move-to-top"]
      : phase === "running"
        ? ["open-window", "move-to-top"]
        : phase === "done"
          ? ["open-pr"]
          : rowActions.slice(0, ISSUE_DETAIL_ACTION_LIMIT);
  const actions = preferred
    .filter((action) => issueRowActionAvailable(action, context))
    .slice(0, ISSUE_DETAIL_ACTION_LIMIT);
  const overflow = rowActions.filter((action) => !actions.includes(action));
  return { phase, actions, overflow };
}

// SPEC #3885 T-005 / FR-003a: the canvas face of a Windowized agent. The link back
// to the Issue comes from the Work projection's agent rows (window id or session
// id) or from the ids this surface itself Windowized; only windows that are on
// the canvas count, so a preview that returned to the row is never doubled.
export function issueCanvasAgentWindowsForIssue(windows, work, rememberedIds, issueNumber) {
  const list = Array.isArray(windows) ? windows : [];
  const agents = Array.isArray(work?.agents) ? work.agents : [];
  const windowIds = new Set(agents.map((agent) => agent?.window_id).filter(Boolean));
  const sessionIds = new Set(agents.map((agent) => agent?.session_id).filter(Boolean));
  const remembered =
    rememberedIds instanceof Set ? rememberedIds : new Set(rememberedIds || []);
  const wanted = Number(issueNumber);
  return list.filter((windowData) => {
    if (!windowData?.id) return false;
    const kind = windowData.placement?.kind || "canvas";
    if (kind !== "canvas") return false;
    // SPEC #3885 FR-011: a Windowized agent carries its Issue durably, so a
    // window that names a different Issue is never this row's canvas face. The
    // Work-projection and remembered-id paths below only prove "this agent is on
    // the canvas", not which Issue owns it, and without this fence one Windowize
    // gives every Issue without an agent the same canvas face.
    const linked = Number(windowData.linked_issue_number);
    if (Number.isFinite(linked) && Number.isFinite(wanted) && linked !== wanted) {
      return false;
    }
    if (Number.isInteger(wanted) && wanted > 0 && linked === wanted) return true;
    if (remembered.has(windowData.id) || windowIds.has(windowData.id)) return true;
    return Boolean(windowData.session_id) && sessionIds.has(windowData.session_id);
  });
}

// SPEC #3885 Phase 2b (T-015 / FR-011): the canvas face of a Windowized agent is one
// composite piece — an Issue header above the interactive terminal — not a bare
// terminal window. The header reuses the row's own badge and secondary vocabulary so
// the same agent reads identically in the list and on the canvas.
const ISSUE_WINDOW_HEADER_ACTION_LIMIT = 2;

const ISSUE_WINDOW_HEADER_ACTIONS = Object.freeze([
  Object.freeze({
    action: "open-issue",
    label: "Open Issue",
    aria: (number) => `Open Issue #${number} in the Issue window`,
  }),
]);

export function issueWindowHeaderModel({
  windowData = null,
  entry = null,
  work = null,
  attention = null,
} = {}) {
  // FR-013: a session with no Issue behind it stays a bare terminal window.
  if (windowData?.is_pm || windowData?.preset === "shell") return null;
  const issueNumber = Number(windowData?.linked_issue_number);
  if (!Number.isFinite(issueNumber) || issueNumber <= 0) return null;
  // The header is the canvas face only; in the list the same agent is the row's
  // read-only status row, and two headers for one agent would double the controls.
  if ((windowData?.placement?.kind || "canvas") !== "canvas") return null;
  const primary = issueRowPrimaryView({
    entry,
    attention,
    inlineWindow: null,
    canvasWindow: windowData,
  });
  return {
    issueNumber,
    title: String(entry?.title || "").trim(),
    primary,
    secondary: issueRowSecondaryItems({ entry, work, attention, primary }),
    actions: ISSUE_WINDOW_HEADER_ACTIONS.slice(0, ISSUE_WINDOW_HEADER_ACTION_LIMIT).map(
      (view) => ({
        action: view.action,
        label: view.label,
        aria: view.aria(issueNumber),
      }),
    ),
  };
}

export function renderIssueWindowHeader(doc, model, onAction = () => {}) {
  if (!model) return null;
  const header = doc.createElement("header");
  header.className = "issue-window-header";
  header.setAttribute("data-issue-number", String(model.issueNumber));

  const main = doc.createElement("div");
  main.className = "issue-window-header-main";
  const number = doc.createElement("span");
  number.className = "issue-window-header-number";
  number.textContent = `#${model.issueNumber}`;
  const title = doc.createElement("span");
  title.className = "issue-window-header-title";
  title.textContent = model.title;
  const badge = doc.createElement("span");
  // Reuse the row badge's tone styling so one agent reads identically in both faces.
  badge.className = "issue-window-header-badge knowledge-row-badge";
  badge.setAttribute("data-tone", model.primary.tone);
  badge.setAttribute("data-state-key", model.primary.key);
  badge.textContent = model.primary.label;
  main.appendChild(number);
  main.appendChild(title);
  main.appendChild(badge);
  header.appendChild(main);

  if (model.secondary.length > 0) {
    const secondary = doc.createElement("div");
    secondary.className = "issue-window-header-secondary";
    for (const item of model.secondary) {
      const node = doc.createElement("span");
      node.className = "issue-window-header-secondary-item knowledge-row-secondary-item";
      node.setAttribute("data-kind", item.kind);
      node.setAttribute("data-key", item.key);
      node.textContent = item.label;
      if (item.title) node.title = item.title;
      secondary.appendChild(node);
    }
    header.appendChild(secondary);
  }

  const actions = doc.createElement("div");
  actions.className = "issue-window-header-actions";
  actions.setAttribute("role", "group");
  for (const action of model.actions) {
    const button = doc.createElement("button");
    button.type = "button";
    button.className = "wizard-button";
    button.setAttribute("data-action", action.action);
    button.setAttribute("aria-label", action.aria);
    button.textContent = action.label;
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      onAction(action.action);
    });
    actions.appendChild(button);
  }
  header.appendChild(actions);
  return header;
}

export function createKnowledgeKanbanSurface({
  send,
  // Semantic search must never use the reconnect queue. This dependency
  // performs one atomic OPEN check + socket.send and reports whether the
  // frame was written; a false result is owned by the retry lifecycle.
  sendKnowledgeSemanticSearchNow = () => false,
  createNode,
  windowMap,
  workspaceWindowById,
  isKnowledgeWindowVisible = () => true,
  getWorkspaceWindows,
  windowCatalogModel,
  scheduleFrame,
  pendingIndexOpenTargetsByPreset,
  knowledgeKindForPreset,
  focusWindowLocally,
  sendWindowFocus,
  focusOrSpawnPreset,
  openIssueLaunchWizard,
  visibleBounds,
  launchPending,
  // SPEC-3671 FR-007 / FR-008 / FR-010: the Issue preview pane. The terminal
  // runtime factory is the shared one from app.js; `readOnly` keeps every input
  // path unattached. `windowizeIssuePreviewWindow` performs the Canvas handoff.
  createTerminalRuntime,
  isAgentsSurfaceMounted = () => false,
  windowDisplayTitle,
  windowRoleBadgeLabel,
  windowizeIssuePreviewWindow,
  // Issue #3884: the Issue row status row reads the agent's live activity line
  // (backend dynamic title detail / status detail) and the instant its runtime
  // state was last observed to change.
  windowActivityDetail,
  windowRuntimeStateSince,
  // SPEC-3671 FR-012 / FR-013: Work state and Work actions on the Issue row. The
  // projection and the derivation helpers are the Work surface's own — the Issue
  // surface never re-derives lifecycle or attention rules.
  getActiveWorkProjection,
  renderOtherWork = () => {},
  workAttentionFor,
  formatWorkLifecycleLabel,
  continueWork,
  openWorkspaceResumePicker,
  openWorkspaceCleanup,
  getResumeBounds,
  // SPEC #3206 FR-017: surface errors (Issue Monitor last_error, Issue window
  // load/search failures) are reported to the notification center as error
  // rows; the bell + drawer are the single place diagnostic text is read.
  // Issue #5284 retains only linked state badges in the Issue window. Pure
  // seams keep the surface independent of the center's DOM shape.
  reportSurfaceError = () => {},
  resolveSurfaceError = () => {},
  reportSurfaceNotice = () => {},
  openSurfaceNotice = () => false,
}) {
      const knowledgeBridgeStateMap = new Map();
      const terminalPreviewText = new Map();
      const issueControlExplanations = new Map();
      const issueExplanationBindings = new WeakMap();
      const issueExplanationNodes = new Map();
      const issueExplanationModel = createUiStateStore({});
      let nextIssueExplanationId = 0;

      // SPEC-4777 T-7a: use one existing popover shell per Issue window. Root
      // delegation also covers cards and headings replaced by cache refreshes.
      function explainIssueControl(node, text, key, focusable = true) {
        const previous = issueExplanationBindings.get(node);
        issueExplanationBindings.set(node, { id: previous?.id ?? ++nextIssueExplanationId,
          windowId: previous?.windowId ?? null, text, key, focusable });
        return node;
      }

      function issueExplanationFocus(windowId) {
        const control = issueExplanationBindings.get(document.activeElement);
        return { focusedKey: control?.windowId === windowId ? control.key : null,
          visible: issueControlExplanations.get(windowId)?.visible === true };
      }

      function publishIssueControlExplanations(windowId, focus = issueExplanationFocus(windowId)) {
        const root = issueControlExplanations.get(windowId)?.root;
        if (!root) return;
        const controls = [];
        const nodes = new Map();
        for (const node of root.querySelectorAll("*")) {
          const binding = issueExplanationBindings.get(node);
          if (!binding) continue;
          const control = { ...binding, windowId };
          issueExplanationBindings.set(node, control);
          controls.push(control);
          nodes.set(control.id, node);
        }
        // Nodes belong to the view; the shared model holds plain presentation data.
        issueExplanationNodes.set(windowId, nodes);
        issueExplanationModel.update(model => ({ ...model, [windowId]: { controls, ...focus } }));
      }

      function bindIssueControlExplanations(windowId, root) {
        issueControlExplanations.get(windowId)?.dispose();
        const binding = bindControlExplanations({ document, root, id: `issue-control-explanation-${windowId}` });
        let unsubscribe = () => {};
        issueControlExplanations.set(windowId, {
          root,
          get visible() { return binding.visible; },
          restoreFocus: binding.restoreFocus,
          refresh: binding.refresh,
          dispose() {
            unsubscribe();
            issueExplanationNodes.delete(windowId);
            issueExplanationModel.update(model => {
              const next = { ...model };
              delete next[windowId];
              return next;
            });
            binding.dispose();
          },
        });
        unsubscribe = issueExplanationModel.subscribe(model => model[windowId], presentation => {
          if (!presentation) return;
          for (const control of presentation.controls) {
            const node = issueExplanationNodes.get(windowId)?.get(control.id);
            if (!node) continue;
            setControlExplanation(node, control.text, control.key, control.focusable);
          }
          if (presentation.focusedKey) issueControlExplanations.get(windowId)?.restoreFocus(
            root.querySelector(`[data-issue-explanation-key="${presentation.focusedKey}"]`), presentation.visible);
          binding.refresh();
        });
      }

      function issueQueueSourceExplanation(entry) {
        switch (entry.queued_by) {
          case "auto-refill": return "Auto-refill added this issue to the launch queue automatically.";
          case "urgent": return "The urgent label added this issue to the launch queue.";
          case "operator": return "An operator added this issue to the launch queue.";
          default: return entry.queued_by ? `Reported queue source: ${entry.queued_by}.` : "The queue source has not been reported.";
        }
      }

      function issueTierChips(issueNumber) {
        const facts = issueMonitorModel.read().status.tier_facts?.find(item => item.issue_number === issueNumber);
        const isTier = value => Number.isInteger(value) && value >= 0;
        if (!facts) return [];
        const chips = [];
        const confirmed = facts.confirmed_launch;
        const catalog = windowCatalogModel?.read().catalog;
        if (isTier(confirmed?.tier) && confirmed.tier <= 255
            && typeof confirmed.window_id === "string" && confirmed.window_id.trim()
            && typeof confirmed.delivery_id === "string" && confirmed.delivery_id.trim()) {
          const agent = catalog?.appState.tabs.flatMap(tab => tab.workspace?.windows || [])
            .find(windowData => windowData.id === confirmed.window_id);
          const linkedIssue = agent?.placement?.kind === "issue_preview"
            ? agent.placement.issue_number : agent?.linked_issue_number;
          const runtime = Object.hasOwn(catalog?.runtimeStates || {}, confirmed.window_id)
            ? catalog.runtimeStates[confirmed.window_id] : agent?.status;
          // Display normalization maps unknown to running; evidence must use the raw state.
          if (agent && Number(linkedIssue) === issueNumber
              && (agent.linked_issue_number == null || Number(agent.linked_issue_number) === issueNumber)
              && runtime === "running") {
            chips.push({ kind: "tier", key: "tier-running", label: `Running tier ${confirmed.tier}`,
              explanation: `Acknowledged launch tier ${confirmed.tier} (zero-based tiers); the matching agent window currently reports running.` });
          }
        }
        if (!isTier(facts.recorded_launch_tier) && !isTier(facts.recorded_landing_tier)) return chips;
        if (isTier(facts.minimum_tier_override)) {
          chips.push({ kind: "tier", key: "tier-minimum", label: `Min tier ${facts.minimum_tier_override}`,
            explanation: `Explicit minimum tier ${facts.minimum_tier_override} (tiers are zero-based). This minimum is not a profile lock; availability can select a higher tier. Recorded launch attempts describe history.` });
        }
        if (Number.isInteger(facts.agent_failures) && facts.agent_failures > 0) {
          const count = facts.agent_failures;
          const next = isTier(facts.next_minimum_tier);
          chips.push({ kind: "tier", key: "tier-failures",
            label: `${count} agent failure${count === 1 ? "" : "s"}${next ? ` → ≥ tier ${facts.next_minimum_tier}` : ""}`,
            explanation: `${count} classified agent failure${count === 1 ? "" : "s"}. Infrastructure and unknown failures are excluded. ${next
              ? `Next automatic selection starts at minimum tier ${facts.next_minimum_tier} (tiers are zero-based); availability can select a higher tier.`
              : "The next automatic minimum tier has not been reported."}` });
        }
        return chips;
      }

      function issueStateExplanation(primary, entry) {
        if (primary.key === "agent:error") {
          const cause = String(entry?.error_message || "").trim();
          return `The agent session reported an error. ${cause || "No error details were reported by the agent or Issue Monitor."}`;
        }
        const meanings = {
          "monitor:queued": "This issue is queued for the monitor to launch when capacity is available.",
          "monitor:not_ready": "This issue does not yet meet the monitor's launch requirements, such as a usable plan and tasks.",
          "monitor:hold_excluded": "A hold label excludes this issue from automatic launches.",
          "monitor:launching": "The monitor is starting an agent for this issue.",
          "monitor:launched": "The monitor launched an agent for this issue.",
          "monitor:merged": "The current execution is complete; the GitHub issue may remain open until release.",
          "monitor:released": "The monitor observed this issue closed on GitHub.",
          "monitor:launch_failed": "The monitor could not start an agent for this issue.",
          "monitor:agent_failed": "The launched agent did not complete this issue successfully.",
          "monitor:blocked_by_claim": "Another agent holds the execution claim, so the monitor will not launch duplicate work.",
          "monitor:skipped": "The monitor skipped this issue during its scan.",
          "monitor:needs_human": "This issue needs a human decision before automatic work can continue.",
          "monitor:held": "The PM deliberately stopped this issue. It stays held until an explicit recovery decision.",
          "agent:running": "The agent is working on this issue.",
          "agent:starting": "The agent session is starting.",
          "agent:idle": "The agent is idle; its session remains available.",
          "agent:waiting": "The agent is waiting for input or approval before it can continue.",
          "agent:stopped": "The agent session has stopped; its output remains available.",
          "agent:error": "The agent session reported an error.",
          "work:closed": "This Work is completed or discarded.",
          "work:remote": "This Work is only available remotely.",
          "work:needs_attention": "This Work needs attention before it can continue.",
          "work:running": "This Work is active.",
          "work:paused": "This Work is paused with no active agent.",
          "issue:open": "The issue is open on GitHub; this alone does not mean an agent is running.",
          "issue:closed": "The issue is closed on GitHub.",
        };
        return meanings[primary.key] || `Reported issue state: ${primary.label}.`;
      }
      // FR-017 bookkeeping: report each occurrence once (issue_monitor_status
      // is re-broadcast constantly, so only a CHANGED text is a new event) and
      // remember which of the two sources changed last for the summary line.
      const ISSUE_MONITOR_ERROR_KEY = "issue-monitor:last_error";
      let reportedIssueMonitorError = "";
      let issueMonitorErrorSequence = 0;
      let surfaceErrorSequence = 0;

      function issueMonitorErrorText() {
        return typeof issueMonitorModel.read().status?.last_error === "string"
          ? issueMonitorModel.read().status.last_error.trim()
          : "";
      }

      // Issue #3628 (AC-5) / SPEC #3885 FR-022: the fleet outage used to be a
      // banner row of its own; it now reaches the operator through the center
      // under its own key, so it never competes with `last_error` for a slot.
      const ISSUE_MONITOR_BLACKOUT_KEY = "issue-monitor:agent_blackout";
      let reportedIssueMonitorBlackout = "";
      const ISSUE_MONITOR_CAPACITY_KEY = "issue-monitor:capacity";
      let reportedIssueMonitorCapacity = "";

      function syncIssueMonitorBlackoutReport() {
        const blackout =
          typeof issueMonitorModel.read().status?.agent_blackout === "string"
            ? issueMonitorModel.read().status.agent_blackout.trim()
            : "";
        if (blackout === reportedIssueMonitorBlackout) return;
        reportedIssueMonitorBlackout = blackout;
        if (blackout) {
          reportSurfaceError({ key: ISSUE_MONITOR_BLACKOUT_KEY, title: "Issue Monitor", message: blackout });
        } else {
          resolveSurfaceError(ISSUE_MONITOR_BLACKOUT_KEY);
        }
      }

      function syncIssueMonitorErrorReport() {
        syncIssueMonitorBlackoutReport();
        syncIssueMonitorCapacityReport();
        const lastError = issueMonitorErrorText();
        if (lastError === reportedIssueMonitorError) return;
        reportedIssueMonitorError = lastError;
        if (lastError) {
          surfaceErrorSequence += 1;
          issueMonitorErrorSequence = surfaceErrorSequence;
          reportSurfaceError({ key: ISSUE_MONITOR_ERROR_KEY, title: "Issue Monitor", message: lastError });
        } else {
          resolveSurfaceError(ISSUE_MONITOR_ERROR_KEY);
        }
      }

      function issueWindowErrorKey(windowId) {
        return `issue-window:${windowId}:load`;
      }

      function syncIssueWindowErrorReport(windowId, state) {
        const message = typeof state?.error === "string" ? state.error : "";
        if (message === (state.reportedError || "")) return;
        state.reportedError = message;
        if (message) {
          surfaceErrorSequence += 1;
          state.errorSequence = surfaceErrorSequence;
          reportSurfaceError({ key: issueWindowErrorKey(windowId), title: "Issue window", message });
        } else {
          resolveSurfaceError(issueWindowErrorKey(windowId));
        }
      }

      const KNOWLEDGE_AUTO_REFRESH_INTERVAL_MS = 60000;
      let nextKnowledgeLoadRequestId = 1;
      let nextKnowledgeSearchRequestId = 1;
      let relatedWorkRefreshTimer = null;
      // Issue #5395: display/activity updates do not invalidate Issue correlation.
      function workAssociationKey(projection) {
        return JSON.stringify((projection?.active_works || [])
          .filter(work => work.linked_issue_numbers?.length)
          .map(work => [
            work.id, normalizeWorkBranch(work.branch),
            [...new Set(work.linked_issue_numbers || [])].sort((a, b) => a - b),
          ]).sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b))));
      }
      let issueWorkAssociationKey = workAssociationKey(getActiveWorkProjection?.());
      const issueMonitorModel = createUiStateStore({ inboxByIssue: {}, status: {
        enabled: false,
        state: "disabled",
        queue_len: 0,
        active_count: 0,
        max_active_agents: 1,
        max_active_agents_override: null,
        agent_capacity: null,
        total_candidates: 0,
        autonomous_mode: false,
        allowed_labels: [],
        label_excluded_count: 0,
        label_excluded_issues: [],
        quota_hold: null,
        github_budget: null,
      }}, { scheduleFrame });
      // Receive paths publish the window whose data changed; the synchronous
      // view pulls its current knowledge state without rendering in receive.
      const issueWindowViewModel = createUiStateStore({ windowId: null });
      const knowledgeBoardViewModel = createUiStateStore({ windowId: null });
      const issueDetailRenderKeys = new WeakMap();
      // Receive publishes invalidation; the view owns projection/layout reads.
      const knowledgeEventRefreshModel = createUiStateStore({ kind: null, revision: 0 });
      const knowledgeAutoRefreshModel = createUiStateStore({ revision: 0 });
      const knowledgeCompletionViewModel = createUiStateStore({ windowId: null });


      function issueMonitorSettingsSourceLabel(source) {
        switch (source) {
          case "saved":
            return "Saved";
          case "last_settings":
            return "Last settings";
          default:
            return "Missing saved profile";
        }
      }

      // Issue #4366 AC-6b: what launches actually use while a provider hold
      // diverts them. Rendered on its own line so the saved settings above it
      // never appear to change because of a hold.
      function issueMonitorEffectiveLaunchText(effective) {
        if (!effective || typeof effective !== "object" || Array.isArray(effective)) {
          return "";
        }
        const reason = typeof effective.reason === "string" ? effective.reason.trim() : "";
        if (!reason) return "";
        const summary = typeof effective.summary === "string" ? effective.summary.trim() : "";
        const agent = typeof effective.agent_id === "string" ? effective.agent_id.trim() : "";
        const target = summary || agent;
        return target
          ? `Launching with ${target} (${reason})`
          : `No launch candidate (${reason})`;
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

      function effectiveIssueMonitorState(status, quotaHold) {
        if (!status.enabled) return "disabled";
        const state = String(status.state || "idle");
        if (["disabled", "error", "auth_required"].includes(state)) {
          return state;
        }
        if (issueMonitorGitHubBudgetText(status)) return "github_budget_hold";
        if (
          quotaHold &&
          ["quota_hold", "active", "launching", "settings_required", "idle"].includes(state)
        ) {
          return "quota_hold";
        }
        return state === "quota_hold" ? "idle" : state;
      }

      function issueMonitorGitHubBudgetText(status) {
        const budget = status?.github_budget;
        if (!status?.enabled || !budget || typeof budget !== "object" || Array.isArray(budget)) return "";
        return Object.entries(budget).filter(([, resource]) => resource?.throttled === true)
          .map(([name, resource]) => {
            const reason = typeof resource.reason === "string" ? resource.reason.trim() : "";
            const resume = typeof resource.backoff_until === "string" ? resource.backoff_until.trim() : "";
            return `GitHub API reads limited: ${name}: ${reason || "GitHub budget throttled"} | Retry ${resume || "time unavailable"}`;
          }).join("\n");
      }

      // #5321: current backend failures remain red; history stays in the bell.
      function issueMonitorPillView(state) {
        const status = issueMonitorModel.read().status;
        if (state === "disabled") return { label: "Stopped", tone: "idle" };
        if (state === "github_budget_hold") return { label: "Limited · GitHub", tone: "idle" };
        if (state === "error") return { label: "⚠ Error", tone: "blocked" };
        if (["quota_hold", "auth_required", "settings_required", "update_drain"].includes(state)) {
          return { label: "Waiting · Hold", tone: "idle" };
        }
        if (status.agent_blackout) return { label: "⚠ Error", tone: "blocked" };
        if (Number(status.active_count) >= Number(status.max_active_agents) && Number(status.queue_len) > 0) {
          return { label: "Waiting · Full", tone: "idle" };
        }
        if (!Number(status.active_count) && !Number(status.queue_len)) {
          return { label: "Waiting · Empty", tone: "idle" };
        }
        return { label: "Running", tone: "active" };
      }

      function issueMonitorCapacityNotice() {
        const status = issueMonitorModel.read().status;
        const capacity = status.agent_capacity;
        if (!capacity) return null;
        const override = status.max_active_agents_override;
        const manual = Number.isInteger(override) && override > 0;
        const autoRetained = !manual && Number.isInteger(status.max_active_agents) && status.max_active_agents > 0;
        const machineWorkerLimit = Math.max(0, (capacity.machine_budget ?? 0)
          - (capacity.other_pm_agents ?? 0) - Math.max(capacity.own_pm_agents ?? 0, 1));
        const excess = manual ? override - machineWorkerLimit : 0;
        const incomplete = !capacity.measurement_complete;
        const attention = incomplete || excess > 0;
        const message = incomplete
          ? manual
            ? `Capacity pending. Manual limit ${override}; no action needed.`
            : autoRetained
              ? `Capacity pending. Auto limit ${status.max_active_agents} retained while measurements refresh.`
            : "Capacity pending. Wait or set Max active manually."
          : excess > 0
            ? `${excess} over recommended. Lower Max active if slow.`
            : `Capacity ready. ${capacity.recommended_worker_limit} workers recommended.`;
        const shownReasons = new Set();
        const uniqueReason = (reason) => String(reason || "").split(/[;\n]+/)
          .map(part => part.trim()).filter(part => {
            if (!part || shownReasons.has(part)) return false;
            shownReasons.add(part);
            return true;
          }).join("; ");
        const lines = [
          `Recommended: ${capacity.recommended_worker_limit} monitor workers · ${capacity.recommended_implementation_count} implementation agents · ${capacity.recommended_total_count} total including PM`,
          `Machine budget: ${capacity.machine_budget ?? "unknown"} · Machine live: ${capacity.machine_live_agents} · This project: ${capacity.own_live_agents} (PM: ${capacity.own_pm_agents}) · Other projects: ${capacity.other_live_agents}`,
          `GUI CPU reserved: ${Number(capacity.gui_cpu_millicores) / 1000} cores`,
          uniqueReason(capacity.reason),
        ];
        for (const constraint of capacity.constraints || []) {
          const resource = constraint.resource === "disk" ? "Disk" : String(constraint.resource).toUpperCase();
          const reason = uniqueReason(constraint.reason);
          const slots = constraint.capacity > Number.MAX_SAFE_INTEGER ? "unlimited" : constraint.capacity ?? "unknown";
          lines.push(`${resource}: ${slots} agent slots${constraint.binding ? " · limiting" : ""}${reason ? ` · ${reason}` : ""}`);
        }
        if (attention) lines.push("Verification may not finish. Timing-dependent test failures may block unrelated PRs.");
        return {
          key: ISSUE_MONITOR_CAPACITY_KEY, title: "Issue Monitor capacity",
          level: manual || (incomplete && autoRetained) || !attention ? "info" : "warn", message,
          details: lines.filter(Boolean).join("\n"), attention,
          badge: incomplete ? "Capacity pending · Details"
            : excess > 0 ? "Over recommended · Details" : `Capacity ${capacity.recommended_worker_limit} · Details`,
        };
      }

      function syncIssueMonitorCapacityReport() {
        const notice = issueMonitorCapacityNotice();
        const fingerprint = notice?.attention ? JSON.stringify(notice) : "";
        if (fingerprint === reportedIssueMonitorCapacity) return;
        reportedIssueMonitorCapacity = fingerprint;
        if (fingerprint) reportSurfaceNotice(notice);
        else resolveSurfaceError(ISSUE_MONITOR_CAPACITY_KEY);
      }

      function renderIssueMonitorCapacity(bar) {
        const status = issueMonitorModel.read().status;
        const badge = bar.querySelector(".knowledge-monitor-capacity-badge");
        const notice = issueMonitorCapacityNotice();
        badge.hidden = !notice;
        badge.textContent = notice?.badge || "";
        badge.dataset.tone = notice?.level === "warn" ? "warning" : "info";
      }

      function createIssueNoticeBadge(label, key, message) {
        // Renderer-failure fallback must not call the failing injected renderer.
        const badge = document.createElement("button");
        badge.type = "button";
        badge.className = "knowledge-row-badge knowledge-notification-badge";
        badge.dataset.tone = "blocked";
        badge.setAttribute("aria-haspopup", "dialog");
        badge.textContent = `${label} · Details`;
        badge.addEventListener("click", () => {
          if (!openSurfaceNotice(key)) {
            reportSurfaceError({ key, title: "Issue window", message });
            openSurfaceNotice(key);
          }
        });
        return badge;
      }

      function renderIssueMonitorControls(element, windowId) {
        const bar = element?.querySelector(".knowledge-monitor-bar");
        if (!bar) return;
        const maxActive = Math.max(
          0,
          Number.parseInt(String(issueMonitorModel.read().status.max_active_agents ?? 1), 10) || 0,
        );
        renderIssueMonitorCapacity(bar);
        const quotaHold = normalizedIssueMonitorQuotaHold(issueMonitorModel.read().status);
        const state = effectiveIssueMonitorState(issueMonitorModel.read().status, quotaHold);
        const pill = bar.querySelector(".knowledge-monitor-pill");
        if (pill) {
          const view = issueMonitorPillView(state);
          pill.textContent = view.label;
          pill.dataset.tone = view.tone;
          pill.dataset.state = state;
          // Issue #3628 (AC-5): the outage outranks a per-issue error, which
          // already occupies `last_error` in the notification center.
          const errorKey = view.tone === "blocked"
            ? issueMonitorModel.read().status.agent_blackout ? ISSUE_MONITOR_BLACKOUT_KEY
              : state === "error" && issueMonitorErrorText() ? ISSUE_MONITOR_ERROR_KEY : ""
            : "";
          pill.dataset.notificationKey = errorKey;
          pill.disabled = false;
          pill.setAttribute("aria-disabled", String(!errorKey));
          pill.title = [
            errorKey ? "Details in notifications" : state === "quota_hold"
              ? `Provider ${quotaHold.provider} | Reset ${quotaHold.reset_at}` : "",
            issueMonitorGitHubBudgetText(issueMonitorModel.read().status),
          ].filter(Boolean).join(" | ");
          if (errorKey) pill.setAttribute("aria-haspopup", "dialog");
          else pill.removeAttribute("aria-haspopup");
          explainIssueControl(pill, `Issue Monitor status: ${view.label}. ${pill.title || "The monitor checks the launch queue for eligible issues."}`);
        }
        const active = bar.querySelector('[data-metric="active"]');
        if (active) {
          const bridgeState = knowledgeBridgeStateMap.get(windowId);
          const count = bridgeState ? filteredIssueEntries(bridgeState)
            .filter(entry => issueQueueColumnForEntry(entry, windowId) === "active").length : 0;
          active.textContent = `Active ${count}/${maxActive}`;
          explainIssueControl(active, `Issues in the Active column. Monitor agent capacity: ${issueMonitorModel.read().status.active_count || 0}/${maxActive}.`);
        }
        const queue = bar.querySelector('[data-metric="queue"]');
        if (queue) {
          queue.textContent = `Queue ${issueMonitorModel.read().status.queue_len || 0}`;
          queue.title = issueMonitorModel.read().status.total_candidates
            ? `Total ${issueMonitorModel.read().status.total_candidates}`
            : "";
          explainIssueControl(queue, `Issues waiting in the launch queue. ${queue.title}`.trim());
        }
        // Issue #4366 AC-6b: the saved settings and the held fallback stay two
        // separate lines, now of the ⚙ tooltip.
        const settings = bar.querySelector('[data-action="monitor-settings"]');
        const source = issueMonitorModel.read().status.launch_profile_source;
        if (settings) {
          const profile =
            issueMonitorModel.read().status.launch_profile_summary || "configure before auto start";
          const lines = [
            `Agent settings ${issueMonitorSettingsSourceLabel(source)}: ${profile}`,
          ];
          const effective = issueMonitorEffectiveLaunchText(
            issueMonitorModel.read().status.effective_launch_profile,
          );
          if (effective) lines.push(effective);
          settings.title = lines.join("\n");
          explainIssueControl(settings, settings.title);
        }
        const toggle = element.querySelector('[data-action="monitor-toggle"]');
        if (toggle) {
          const enabled = Boolean(issueMonitorModel.read().status.enabled);
          toggle.textContent = enabled ? "Stop" : "Start";
          toggle.dataset.enabled = enabled ? "true" : "false";
          toggle.classList.toggle("primary", !enabled);
        }
        // Issue #3561: the Autonomous control is a WAI-ARIA switch whose label
        // names the setting and whose aria-checked + state word carry the
        // current value. It renders only what the server status says — the
        // click handler never writes a local optimistic value.
        const autonomous = element.querySelector('[data-action="monitor-autonomous"]');
        if (autonomous) {
          const enabled = Boolean(issueMonitorModel.read().status.autonomous_mode);
          autonomous.setAttribute("aria-checked", enabled ? "true" : "false");
          autonomous.dataset.enabled = enabled ? "true" : "false";
          const stateWord = autonomous.querySelector(".knowledge-monitor-switch__state");
          if (stateWord) stateWord.textContent = enabled ? "On" : "Off";
        }

      }

      function applyIssueMonitorStatus(nextStatus) {
        issueMonitorModel.update(model => ({ ...model, status: {
          ...model.status,
          ...(nextStatus || {}),
          max_active_agents_override: nextStatus?.max_active_agents_override ?? null,
          agent_capacity: nextStatus?.agent_capacity ?? null,
          quota_hold: normalizedIssueMonitorQuotaHold(nextStatus),
          github_budget: nextStatus?.github_budget ?? null,
          // Issue #4366 AC-6b: omitted once the hold clears, so it must not
          // survive from the previous status the way merged fields do.
          effective_launch_profile: nextStatus?.effective_launch_profile ?? null,
          // Empty facts are omitted on the wire; never retain an older snapshot.
          tier_facts: Array.isArray(nextStatus?.tier_facts) ? nextStatus.tier_facts : [],
          // Issue #3628: omitted (skip_serializing_if) once the fleet recovers.
          agent_blackout: nextStatus?.agent_blackout ?? null,
          update_drain: nextStatus?.update_drain ?? null,
        }}));
      }

      function applyIssueMonitorInbox(items) {
        const inboxByIssue = Object.fromEntries((Array.isArray(items) ? items : [])
          .filter(item => Number.isFinite(item?.issue?.number))
          .map(item => [item.issue.number, item]));
        issueMonitorModel.update(model => ({ ...model, inboxByIssue }));
      }

      function applyIssueMonitorQueueResult(event) {
        const action = {push:"added to the queue", remove:"removed from the queue", move:"queue order updated"}[event.operation];
        if (!action) return;
        const issues = (event.issue_numbers || []).map(number => `Issue #${number}`).join(", ");
        for (const [windowId, state] of knowledgeBridgeStateMap) {
          if (state.kind !== "issue") continue;
          state.queueFeedback = event.error ? `${issues}: ${event.error}` : `${issues}: ${action}.`;
          issueWindowViewModel.update(() => ({windowId}));
        }
      }

      function scheduleIssueMonitorProjectionRefresh() {
        knowledgeEventRefreshModel.update(model => ({ kind: "monitor", revision: model.revision + 1 }));
      }

      // SPEC #3885 Phase 5 (T-033 / FR-022): "+ New" registers an Issue from a
      // popover built on the shared modal primitive, so the window keeps no
      // always-visible quick-register input. One popover serves every Issue
      // window; it is created on first use and reused.
      let issueNewPopover = null;

      function ensureIssueNewPopover() {
        if (issueNewPopover) return issueNewPopover;
        const backdrop = createNode("div", "modal-backdrop issue-new-popover");
        backdrop.setAttribute("aria-hidden", "true");
        const dialog = createNode("div", "modal-shell issue-new-dialog");
        dialog.setAttribute("role", "dialog");
        dialog.setAttribute("aria-modal", "true");
        dialog.setAttribute("aria-labelledby", "issue-new-heading");
        dialog.tabIndex = -1;

        const header = createNode("header", "modal-header");
        const heading = createNode("h2", "", "New Issue");
        heading.id = "issue-new-heading";
        header.appendChild(heading);

        const content = createNode("div", "modal-body issue-new-body");
        const titleLabel = createNode("label", "issue-new-field");
        titleLabel.appendChild(createNode("span", "", "Title"));
        const title = createNode("input", "issue-new-title");
        title.type = "text";
        title.dataset.role = "issue-new-title";
        title.placeholder = "Issue title";
        titleLabel.appendChild(title);
        const autoMergeLabel = createNode("label", "issue-new-check");
        const autoMerge = createNode("input", "");
        autoMerge.type = "checkbox";
        autoMerge.dataset.role = "issue-new-auto-merge";
        autoMergeLabel.appendChild(autoMerge);
        autoMergeLabel.appendChild(createNode("span", "", "auto-merge"));
        content.appendChild(titleLabel);
        content.appendChild(autoMergeLabel);

        const footer = createNode("footer", "modal-footer");
        const cancel = createNode("button", "text-button", "Cancel");
        cancel.type = "button";
        cancel.dataset.action = "issue-new-cancel";
        const register = createNode("button", "wizard-button", "Register");
        register.type = "button";
        register.dataset.action = "issue-new-register";
        const registerLaunch = createNode("button", "wizard-button primary", "Register & launch");
        registerLaunch.type = "button";
        registerLaunch.dataset.action = "issue-new-register-launch";
        footer.appendChild(cancel);
        footer.appendChild(register);
        footer.appendChild(registerLaunch);

        dialog.appendChild(header);
        dialog.appendChild(content);
        dialog.appendChild(footer);
        backdrop.appendChild(dialog);
        document.body.appendChild(backdrop);

        let releaseTrap = null;
        let returnFocus = null;
        const close = () => {
          backdrop.classList.remove("open");
          backdrop.setAttribute("aria-hidden", "true");
          releaseTrap?.();
          releaseTrap = null;
          returnFocus?.focus?.();
          returnFocus = null;
        };
        const submit = (launch) => {
          const value = String(title.value || "").trim();
          if (!value) {
            title.focus();
            return;
          }
          send({
            kind: "quick_register_issue",
            title: value,
            launch,
            auto_merge: autoMerge.checked === true,
          });
          close();
        };
        backdrop.addEventListener("mousedown", (event) => event.stopPropagation());
        backdrop.addEventListener("click", (event) => {
          if (event.target === backdrop) close();
        });
        dialog.addEventListener("keydown", (event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            close();
          } else if (event.key === "Enter" && event.target === title) {
            event.preventDefault();
            submit(false);
          }
        });
        cancel.addEventListener("click", close);
        register.addEventListener("click", () => submit(false));
        registerLaunch.addEventListener("click", () => submit(true));

        issueNewPopover = {
          open() {
            title.value = "";
            autoMerge.checked = false;
            returnFocus = document.activeElement;
            backdrop.classList.add("open");
            backdrop.removeAttribute("aria-hidden");
            releaseTrap = createFocusTrap(dialog, { document });
            title.focus();
          },
          close,
        };
        return issueNewPopover;
      }

      function wireIssueMonitorControls(body, windowId) {
        body
          .querySelector('[data-action="issue-new"]')
          ?.addEventListener("click", (event) => {
            event.stopPropagation();
            ensureIssueNewPopover().open();
          });
        const bar = body.querySelector(".knowledge-monitor-bar");
        if (!bar) return;
        bar.addEventListener("mousedown", (event) => event.stopPropagation());
        for (const action of ["monitor-settings", "monitor-setup"]) {
          bar
            .querySelector(`[data-action="${action}"]`)
            ?.addEventListener("click", () => {
              if (action === "monitor-settings" && !body.closest(".settings-root")) {
                document.dispatchEvent(new document.defaultView.CustomEvent("settings:open", { detail: { section: "issues" } }));
              } else if (action === "monitor-setup") {
                send({ kind: "issue_monitor_configure_profile" });
              }
            });
        }
        bar.querySelector(".knowledge-monitor-capacity-badge")?.addEventListener("click", () => {
          const notice = issueMonitorCapacityNotice();
          if (!notice) return;
          reportSurfaceNotice(notice);
          openSurfaceNotice(notice.key);
        });
        bar.querySelector(".knowledge-monitor-pill")?.addEventListener("click", () => {
          const key = bar.querySelector(".knowledge-monitor-pill").dataset.notificationKey;
          if (!key) return;
          if (!openSurfaceNotice(key)) {
            const message = key === ISSUE_MONITOR_BLACKOUT_KEY
              ? issueMonitorModel.read().status.agent_blackout : issueMonitorErrorText();
            reportSurfaceError({ key, title: "Issue Monitor", message });
            openSurfaceNotice(key);
          }
        });
        bar
          .querySelector('[data-action="monitor-toggle"]')
          ?.addEventListener("click", () => {
            send({
              kind: "set_issue_monitor_enabled",
              enabled: !Boolean(issueMonitorModel.read().status.enabled),
            });
          });
        bar
          .querySelector('[data-action="monitor-autonomous"]')
          ?.addEventListener("click", () => {
            send({
              kind: "set_issue_monitor_autonomous_mode",
              enabled: !Boolean(issueMonitorModel.read().status.autonomous_mode),
            });
          });
        renderIssueMonitorControls(body, windowId);
        send({ kind: "list_issue_monitor" });
      }


      // SPEC-2017 US-9 — Kanban Drawer (slide-over detail). Reuses the
      // SPEC-2356 .op-drawer pattern; backdrop click and Esc both
      // dismiss it; createFocusTrap keeps Tab within the dialog while
      // open. State is module-scoped because only one Drawer is open
      // at a time even when multiple Kanban windows exist.
      let kanbanDrawerFocusReturn = null;
      let kanbanDrawerFocusTrapRelease = null;
      let kanbanDrawerActiveContext = null;
      function openKanbanDrawer(context) {
        const drawer = document.getElementById("kanban-drawer");
        const backdrop = document.getElementById("kanban-drawer-backdrop");
        if (!drawer || !backdrop) return;
        kanbanDrawerActiveContext = context || null;
        kanbanDrawerFocusReturn = document.activeElement;
        backdrop.hidden = false;
        backdrop.dataset.open = "true";
        drawer.hidden = false;
        drawer.dataset.open = "true";
        renderKanbanDrawerBody();
        try { drawer.focus({ preventScroll: true }); }
        catch { drawer.focus(); }
        if (typeof kanbanDrawerFocusTrapRelease === "function") {
          kanbanDrawerFocusTrapRelease();
        }
        kanbanDrawerFocusTrapRelease = createFocusTrap(drawer, { document });
      }

      function closeKanbanDrawer() {
        const drawer = document.getElementById("kanban-drawer");
        const backdrop = document.getElementById("kanban-drawer-backdrop");
        if (!drawer || !backdrop) return;
        if (drawer.dataset.open !== "true") return;
        drawer.dataset.open = "false";
        backdrop.dataset.open = "false";
        // Hide after the transition so prefers-reduced-motion users
        // still see the focus trap dismantle cleanly.
        backdrop.hidden = true;
        drawer.hidden = true;
        if (typeof kanbanDrawerFocusTrapRelease === "function") {
          kanbanDrawerFocusTrapRelease();
          kanbanDrawerFocusTrapRelease = null;
        }
        if (
          kanbanDrawerFocusReturn &&
          typeof kanbanDrawerFocusReturn.focus === "function"
        ) {
          try { kanbanDrawerFocusReturn.focus({ preventScroll: true }); }
          catch { kanbanDrawerFocusReturn.focus(); }
        }
        kanbanDrawerFocusReturn = null;
        kanbanDrawerActiveContext = null;
      }

      function renderKanbanDrawerBody() {
        const body = document.getElementById("kanban-drawer-body");
        const titleEl = document.getElementById("kanban-drawer-title");
        const footer = document.getElementById("kanban-drawer-footer");
        if (!body || !titleEl || !footer) return;
        const context = kanbanDrawerActiveContext;
        if (!context) {
          body.innerHTML = "";
          footer.innerHTML = "";
          titleEl.textContent = "Detail";
          return;
        }
        const state = ensureKnowledgeBridgeState(context.windowId, context.kind);
        const detail = state.detail;
        body.innerHTML = "";
        footer.innerHTML = "";
        titleEl.textContent = detail?.title || "Loading detail";
        if (state.detailLoading || !detail) {
          body.appendChild(
            createNode(
              "div",
              "kanban-drawer-section-body",
              state.detailLoading ? "Loading detail" : "No cached detail available",
            ),
          );
          return;
        }
        if (detail.subtitle) {
          body.appendChild(
            createNode("div", "knowledge-detail-subtitle", detail.subtitle),
          );
        }
        const displayLabels = visibleKnowledgeLabels(detail.labels || []);
        const stalePhase = staleKnowledgePhaseWarning(detail);
        if (displayLabels.length > 0 || stalePhase) {
          const labelRow = createNode("div", "knowledge-label-row");
          for (const label of displayLabels) {
            labelRow.appendChild(createNode("span", "knowledge-chip", label));
          }
          if (stalePhase) {
            labelRow.appendChild(
              createNode("span", "kanban-card-chip kanban-card-chip--warning", stalePhase),
            );
          }
          body.appendChild(labelRow);
        }
        for (const section of detail.sections || []) {
          const card = createNode("section", "kanban-drawer-section");
          card.appendChild(
            createNode("div", "kanban-drawer-section-title", section.title),
          );
          card.appendChild(
            renderUiContent(document, section.content, "kanban-drawer-section-body"),
          );
          body.appendChild(card);
        }
        if (
          detail.launch_issue_number !== null &&
          detail.launch_issue_number !== undefined
        ) {
          const launchButton = createNode(
            "button",
            "wizard-button primary",
            "Launch Agent",
          );
          launchButton.type = "button";
          launchButton.addEventListener("click", () => {
            openIssueLaunchWizard(context.windowId, detail.launch_issue_number);
          });
          footer.appendChild(launchButton);
        }
      }

      function ensureKnowledgeBridgeState(windowId, knowledgeKind) {
        if (!knowledgeBridgeStateMap.has(windowId)) {
          knowledgeBridgeStateMap.set(windowId, {
            kind: normalizeKnowledgeKind(knowledgeKind),
            entries: [],
            baseEntries: [],
            entriesRevision: null,
            selectedNumber: null,
            // SPEC #3885 FR-014 / AC-14: the Issue window's view mode. List is
            // the default; split lays the running Issues out as detail +
            // terminal pairs. `splitPairSizes` remembers which pairs the user
            // grew (T-005) so a data refresh does not shrink them back.
            viewMode: "list",
            issueDetailView: "issue",
            previewHidden: true,
            splitPairSizes: new Map(),
            // SPEC #3170 FR-101: independent monotonically increasing
            // explicit-selection generation; 0 means no explicit selection.
            selectionGeneration: 0,
            // SPEC #3170 FR-099: silent semantic retry window (frontend
            // owned). generation invalidates stale timers; index walks the
            // fixed 5/10/20/30/30… ladder; active marks a degraded query so
            // reconnect can restart the sequence at 5 seconds.
            semanticRetryTimer: null,
            semanticRetryIndex: 0,
            semanticRetryGeneration: 0,
            semanticRetryActive: false,
            semanticRetryTyped: false,
            searchGeneration: 0,
            searchIntentKind: normalizeKnowledgeKind(knowledgeKind),
            searchIntentQuery: "",
            inFlightSearchIntent: null,
            queuedSearchIntent: null,
            detail: null,
            query: "",
            loading: false,
            refreshing: false,
            searching: false,
            detailLoading: false,
            pendingSearchTimer: null,
            loadRequestId: 0,
            ownedLoadRequestIds: new Set(),
            loadSelectionGeneration: 0,
            loadSelectedNumber: null,
            detailRequestId: 0,
            detailRequestSelectionGeneration: 0,
            detailRequestNumber: null,
            searchRequestId: 0,
            inFlightSearchRequestId: 0,
            searchInFlight: false,
            queuedSearchQuery: "",
            queuedLoadRefresh: false,
            loadRecoveryTimer: null,
            loadSlow: false,
            error: "",
            emptyMessage: "",
            baseEmptyMessage: "",
            refreshEnabled: true,
            // SPEC-2017 — Kanban state. hideDone hydrates from
            // localStorage so the user's preference survives reloads;
            // dndSnapshot stores the pre-drop column index to enable
            // optimistic-UI rollback when phase write-back fails;
            // pendingPhaseUpdates tracks in-flight requests so cards
            // render a spinner until the server confirms the move.
            hideDone: readKanbanHideDonePreference(),
            issueStateFilter: "open",
            issueLaneFilter: "all",
            dndSnapshot: null,
            pendingPhaseUpdates: new Map(),
            autoRefreshTimer: null,
            eventRefreshDirty: false,
            workSnapshotDirty: false,
            autoRefreshDirty: false,
            autoRefreshChanged: false,
            autoRefreshRevision: null,
            revisionPending: false,
            cacheRevision: null,
          });
        }
        const state = knowledgeBridgeStateMap.get(windowId);
        const nextKind = normalizeKnowledgeKind(knowledgeKind || state.kind);
        if (state.kind && nextKind && state.kind !== nextKind) {
          invalidateKnowledgeSearchOwner(state, nextKind, state.query.trim());
        }
        state.kind = nextKind || state.kind;
        if (state.hideDone === undefined) {
          state.hideDone = readKanbanHideDonePreference();
        }
        if (!["open", "closed", "all"].includes(state.issueStateFilter)) {
          state.issueStateFilter = "open";
        }
        if (!state.pendingPhaseUpdates) {
          state.pendingPhaseUpdates = new Map();
        }
        return state;
      }

      function knowledgeAutoRefreshIsBusy(state) {
        return (
          state.loading ||
          state.refreshing ||
          state.searching ||
          state.searchInFlight ||
          state.pendingSearchTimer !== null ||
          state.semanticRetryTimer !== null ||
          Boolean(state.inFlightSearchIntent) ||
          Boolean(state.queuedSearchIntent) ||
          state.semanticRetryActive === true
        );
      }

      function ensureKnowledgeAutoRefresh(windowId, knowledgeKind) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        if (state.autoRefreshTimer !== null) {
          return;
        }
        state.autoRefreshTimer = setInterval(() => {
          if (
            knowledgeBridgeStateMap.get(windowId) !== state ||
            !windowMap.get(windowId)
          ) {
            clearInterval(state.autoRefreshTimer);
            state.autoRefreshTimer = null;
            return;
          }
          state.autoRefreshDirty = true;
          refreshVisibleKnowledgeWindows();
        }, KNOWLEDGE_AUTO_REFRESH_INTERVAL_MS);
      }

      function knowledgeWindowIsVisible(windowId) {
        const data = workspaceWindowById(windowId);
        const element = windowMap.get(windowId);
        return Boolean(data && element && !document.hidden &&
          !element.closest?.("[hidden]") && isKnowledgeWindowVisible(windowId));
      }

      function refreshVisibleKnowledgeWindows() {
        knowledgeAutoRefreshModel.update(model => ({ revision: model.revision + 1 }));
      }

      function flushVisibleKnowledgeWindows() {
        for (const [windowId, state] of knowledgeBridgeStateMap) {
          if (!state.autoRefreshDirty || state.revisionPending || !state.refreshEnabled ||
              knowledgeAutoRefreshIsBusy(state) || !knowledgeWindowIsVisible(windowId)) continue;
          state.autoRefreshDirty = false;
          if (state.autoRefreshChanged) {
            state.autoRefreshChanged = false;
            requestKnowledgeBridge(windowId, state.kind, false);
            continue;
          }
          state.revisionPending = true;
          send({ kind: "check_knowledge_revision", id: windowId, knowledge_kind: state.kind });
        }
      }

      function applyKnowledgeCacheRevision(event) {
        const state = knowledgeBridgeStateMap.get(event.id);
        if (!state || !state.revisionPending ||
            normalizeKnowledgeKind(event.knowledge_kind) !== normalizeKnowledgeKind(state.kind)) return;
        state.revisionPending = false;
        if (event.revision !== null && event.revision === state.cacheRevision) return;
        state.autoRefreshChanged = true;
        state.autoRefreshRevision = event.revision ?? null;
        state.autoRefreshDirty = true;
        refreshVisibleKnowledgeWindows();
      }

      function readKanbanHideDonePreference() {
        try {
          if (typeof localStorage === "undefined") return false;
          return localStorage.getItem("kanban-hide-done") === "1";
        } catch (_err) {
          return false;
        }
      }

      function writeKanbanHideDonePreference(value) {
        try {
          if (typeof localStorage === "undefined") return;
          if (value) {
            localStorage.setItem("kanban-hide-done", "1");
          } else {
            localStorage.removeItem("kanban-hide-done");
          }
        } catch (_err) {
          // localStorage may be unavailable in private mode; ignore.
        }
      }

      function clearKnowledgeBridgeState(windowId) {
        for (const host of windowMap.get(windowId)?.querySelectorAll(".issue-queue-items, .kanban-column-body") || []) {
          keyedKnowledgeLists.get(host)?.dispose();
        }
        issueControlExplanations.get(windowId)?.dispose();
        issueControlExplanations.delete(windowId);
        terminalPreviewText.delete(windowId);
        const state = knowledgeBridgeStateMap.get(windowId);
        state?.monitorSubscriptions?.forEach(unsubscribe => unsubscribe());
        if (state?.reportedError) {
          // FR-017: a closed window's load error is no longer actionable.
          resolveSurfaceError(issueWindowErrorKey(windowId));
          state.reportedError = "";
        }
        if (state?.renderError) {
          resolveSurfaceError(`issue-window:${windowId}:render`);
        }
        if (state?.pendingSearchTimer !== null && state?.pendingSearchTimer !== undefined) {
          clearTimeout(state.pendingSearchTimer);
          state.pendingSearchTimer = null;
        }
        // AS-17.2: window destroy invalidates the silent retry owner.
        invalidateKnowledgeSemanticRetry(state);
        if (state) {
          state.queuedSearchQuery = "";
          state.queuedSearchIntent = null;
          state.inFlightSearchIntent = null;
          state.searchGeneration = (state.searchGeneration || 0) + 1;
          state.searchInFlight = false;
          state.inFlightSearchRequestId = 0;
          state.detailRequestId = 0;
          state.queuedLoadRefresh = false;
          state.loadSlow = false;
          if (state.loadRecoveryTimer !== null) {
            clearTimeout(state.loadRecoveryTimer);
            state.loadRecoveryTimer = null;
          }
          state.pendingPhaseUpdates?.clear();
          state.dndSnapshot = null;
          if (state.autoRefreshTimer !== null) {
            clearInterval(state.autoRefreshTimer);
            state.autoRefreshTimer = null;
          }
        }
        knowledgeBridgeStateMap.delete(windowId);
        if (![...knowledgeBridgeStateMap.values()].some(state => normalizeKnowledgeKind(state.kind) === "issue")) {
        }
        if (
          knowledgeBridgeStateMap.size === 0 &&
          relatedWorkRefreshTimer !== null
        ) {
          clearTimeout(relatedWorkRefreshTimer);
          relatedWorkRefreshTimer = null;
        }
      }

      function knowledgeEntriesAreEmpty(state) {
        return (
          (!Array.isArray(state.entries) || state.entries.length === 0) &&
          (!Array.isArray(state.baseEntries) || state.baseEntries.length === 0)
        );
      }

      function clearKnowledgeLoadRecoveryTimer(state) {
        if (state.loadRecoveryTimer === null) {
          return;
        }
        clearTimeout(state.loadRecoveryTimer);
        state.loadRecoveryTimer = null;
      }

      function scheduleKnowledgeLoadRecovery(windowId, knowledgeKind, requestId) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        clearKnowledgeLoadRecoveryTimer(state);
        state.loadRecoveryTimer = setTimeout(() => {
          state.loadRecoveryTimer = null;
          if (
            knowledgeBridgeStateMap.get(windowId) !== state ||
            !workspaceWindowById(windowId)
          ) {
            return;
          }
          if (
            !state.loading ||
            state.loadRequestId !== requestId
          ) {
            return;
          }
          // Issue #5413: a slow cache read is still in flight. Show progress
          // instead of issuing another read that adds backend load.
          state.loadSlow = true;
          state.loadRecoveryTimer = setTimeout(() => {
            state.loadRecoveryTimer = null;
            if (
              knowledgeBridgeStateMap.get(windowId) !== state ||
              !workspaceWindowById(windowId) ||
              !state.loading ||
              state.loadRequestId !== requestId
            ) {
              return;
            }
            state.loading = false;
            state.refreshing = false;
            state.loadSlow = false;
            state.error = "Timed out loading cache-backed data";
            renderKnowledgeBridge(windowId);
          }, 55000);
          renderKnowledgeBridge(windowId);
        }, 5000);
      }

      function finishKnowledgeLoad(state, windowId, knowledgeKind) {
        clearKnowledgeLoadRecoveryTimer(state);
        state.loading = false;
        state.refreshing = false;
        state.loadSlow = false;
        const queuedRefresh = state.queuedLoadRefresh;
        state.queuedLoadRefresh = false;
        if (queuedRefresh && workspaceWindowById(windowId)) {
          requestKnowledgeBridge(windowId, knowledgeKind, true);
          return true;
        }
        resumeKnowledgeEventRefresh();
        return false;
      }

      function requestKnowledgeBridge(windowId, knowledgeKind, refresh = false) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        if (state.loading) {
          state.queuedLoadRefresh = state.queuedLoadRefresh || Boolean(refresh);
          return;
        }
        // Any newly dispatched load covers earlier event invalidation, including
        // a hidden window's event when its cache revision changed on reveal.
        state.eventRefreshDirty = false;
        if (state.pendingSearchTimer !== null) {
          clearTimeout(state.pendingSearchTimer);
          state.pendingSearchTimer = null;
        }
        const requestId = nextKnowledgeLoadRequestId++;
        state.loadRequestId = requestId;
        state.ownedLoadRequestIds.add(requestId);
        while (state.ownedLoadRequestIds.size > 4) {
          state.ownedLoadRequestIds.delete(
            state.ownedLoadRequestIds.values().next().value,
          );
        }
        state.loadSelectionGeneration = state.selectionGeneration;
        state.loadSelectedNumber = state.selectedNumber;
        state.loading = true;
        state.loadSlow = false;
        state.refreshing = Boolean(refresh);
        state.searching = false;
        state.queuedLoadRefresh = false;
        state.error = "";
        const effectiveKind = knowledgeKind || state.kind;
        send({
          kind: "load_knowledge_bridge",
          id: windowId,
          knowledge_kind: effectiveKind,
          request_id: requestId,
          selected_number: state.selectedNumber ?? null,
          refresh,
        });
        scheduleKnowledgeLoadRecovery(windowId, effectiveKind, requestId);
      }

      function resyncKnowledge(windowIds) {
        for (const windowId of windowIds) {
          const state = knowledgeBridgeStateMap.get(windowId);
          if (!state) continue;
          clearKnowledgeLoadRecoveryTimer(state);
          invalidateKnowledgeSearchOwner(state, state.kind, state.query);
          state.inFlightSearchIntent = null;
          state.searchInFlight = false;
          state.inFlightSearchRequestId = 0;
          state.searching = false;
          state.detailRequestId = 0;
          state.revisionPending = false;
          state.ownedLoadRequestIds.clear();
          state.loading = false;
          // A dropped phase reply cannot settle a pending marker. Reload the
          // authoritative phase without retrying the write or rolling it back.
          state.pendingPhaseUpdates.clear();
          state.dndSnapshot = null;
          // The full read includes selected detail; keep query and selection.
          requestKnowledgeBridge(windowId, state.kind, false);
        }
      }

      function scheduleKnowledgeRelatedWorkRefresh(forceReload = false) {
        knowledgeEventRefreshModel.update(model => ({ kind: "projection", forceReload, revision: model.revision + 1 }));
      }

      function refreshKnowledgeWorkSnapshot(forceReload) {
        const projection = getActiveWorkProjection?.();
        const issueKey = workAssociationKey(projection);
        for (const state of knowledgeBridgeStateMap.values()) {
          state.eventRefreshDirty ||= forceReload || issueKey !== issueWorkAssociationKey;
          state.workSnapshotDirty = true;
        }
        issueWorkAssociationKey = issueKey;
        scheduleKnowledgeEventRefresh();
      }

      function updateKnowledgeWorkSnapshot(state) {
        if (!state.detail?.related_works) return;
        state.detail.related_works = state.detail.related_works.map(work => {
          const live = issueWorkRowForEntry(getActiveWorkProjection?.(), { related_work_refs: [work] });
          if (!live) return work;
          return { ...work, title: live.title, status_category: live.status_category,
            branch: live.branch, worktree_path: live.worktree_path, updated_at: live.updated_at,
            agents: (work.agents || []).map(agent => {
              const current = live.agents?.find(item => item.session_id === agent.session_id);
              return current ? { ...agent, ...current } : agent;
            }) };
        });
      }

      function scheduleKnowledgeEventRefresh() {
        if (relatedWorkRefreshTimer !== null) {
          clearTimeout(relatedWorkRefreshTimer);
        }
        relatedWorkRefreshTimer = setTimeout(() => {
          relatedWorkRefreshTimer = null;
          for (const [windowId, state] of knowledgeBridgeStateMap.entries()) {
            const windowData = workspaceWindowById(windowId);
            if (!windowData) {
              continue;
            }
            if (!isKnowledgeWindowVisible(windowId)) continue;
            if (state.workSnapshotDirty) {
              state.workSnapshotDirty = false;
              updateKnowledgeWorkSnapshot(state);
              renderKnowledgeBridge(windowId);
            }
            if (state.eventRefreshDirty && !state.loading) {
              state.eventRefreshDirty = false;
              requestKnowledgeBridge(windowId, state.kind, false);
            }
          }
        }, 2000);
      }

      // Called after document, project-tab, and window visibility has settled.
      function resumeKnowledgeEventRefresh() {
        knowledgeEventRefreshModel.update(model => ({ kind: "resume", revision: model.revision + 1 }));
      }

      function resumeVisibleKnowledgeEventRefresh() {
        if (relatedWorkRefreshTimer !== null) return;
        if ([...knowledgeBridgeStateMap].some(([id, state]) =>
          isKnowledgeWindowVisible(id) &&
          (state.workSnapshotDirty || (state.eventRefreshDirty && !state.loading)))) {
          scheduleKnowledgeEventRefresh();
        }
      }

      // AS-17.7 (T-953): immediate local fallback rows for a query — match
      // by number, title, metadata line, or label, case-insensitively.
      function applyLocalKnowledgeFilter(state, query) {
        const queryLower = query.toLowerCase();
        const numberQuery = queryLower.replace(/^#/, "");
        const matches = (entry) => {
          if (!entry) {
            return false;
          }
          if (numberQuery && String(entry.number ?? "").includes(numberQuery)) {
            return true;
          }
          if ((entry.title || "").toLowerCase().includes(queryLower)) {
            return true;
          }
          if ((entry.meta || "").toLowerCase().includes(queryLower)) {
            return true;
          }
          const labels = Array.isArray(entry.labels) ? entry.labels : [];
          return labels.some((label) =>
            String(label).toLowerCase().includes(queryLower),
          );
        };
        state.entries = (state.baseEntries || []).filter(matches);
      }

      function restoreKnowledgeBaseEntries(state) {
        state.entries = Array.isArray(state.baseEntries)
          ? state.baseEntries.slice()
          : [];
        state.emptyMessage = state.baseEmptyMessage || "";
        if (
          state.selectionGeneration === 0 &&
          state.selectedNumber &&
          !state.entries.some((entry) => entry.number === state.selectedNumber)
        ) {
          state.selectedNumber =
            state.entries.length > 0 ? state.entries[0].number : null;
        }
      }

      function replaceKnowledgeEntry(entries, fresh) {
        if (!fresh || !Array.isArray(entries)) {
          return false;
        }
        const index = entries.findIndex((entry) => entry.number === fresh.number);
        if (index < 0) {
          return false;
        }
        entries[index] = fresh;
        return true;
      }

      function knowledgeDetailRequestMatches(state, event) {
        if (!event.request_id) {
          // ID-less compatibility is restricted to generation zero. Once a
          // user has selected anything explicitly, identity cannot be proven
          // even if an A→B→A sequence happens to end on the same number.
          return (
            state.selectionGeneration === 0 &&
            event.detail?.number === state.selectedNumber
          );
        }
        if (event.request_id === state.detailRequestId) {
          return (
            state.detailRequestSelectionGeneration === state.selectionGeneration &&
            state.detailRequestNumber === state.selectedNumber &&
            event.detail?.number === state.selectedNumber
          );
        }
        if (event.request_id === state.loadRequestId) {
          if (state.loadSelectionGeneration !== state.selectionGeneration) {
            return false;
          }
          if (state.selectionGeneration === 0 && state.loadSelectedNumber === null) {
            return event.detail?.number === state.selectedNumber;
          }
          return (
            state.loadSelectedNumber === state.selectedNumber &&
            event.detail?.number === state.selectedNumber
          );
        }
        return false;
      }

      function normalizeKnowledgeKind(value) {
        return typeof value === "string" ? value.trim().toLowerCase() : "";
      }

      function isSilentSemanticKind(kind) {
        // Both Issue and SPEC presets normalize to the backend `issue` kind.
        return normalizeKnowledgeKind(kind) === "issue";
      }

      function isKnowledgeSemanticRetryDirective(value) {
        if (typeof value !== "object" || value === null) {
          return false;
        }
        const fields = Object.keys(value);
        return (
          fields.length === 3 &&
          Object.prototype.hasOwnProperty.call(value, "error_code") &&
          Object.prototype.hasOwnProperty.call(value, "retryable") &&
          Object.prototype.hasOwnProperty.call(value, "retry_after_ms") &&
          value.retryable === true &&
          value.retry_after_ms === 5000 &&
          (value.error_code === "INDEX_NOT_READY" ||
            value.error_code === "SEARCH_UNAVAILABLE")
        );
      }

      // SPEC #3170 FR-099: fixed silent retry ladder for typed transient
      // semantic failures — 5s, 10s, 20s, 30s, then 30s indefinitely.
      const KNOWLEDGE_SEMANTIC_RETRY_DELAYS = [5000, 10000, 20000, 30000];

      function invalidateKnowledgeSemanticRetry(state) {
        if (!state) {
          return;
        }
        if (state.semanticRetryTimer !== null) {
          clearTimeout(state.semanticRetryTimer);
          state.semanticRetryTimer = null;
        }
        state.semanticRetryIndex = 0;
        state.semanticRetryActive = false;
        state.semanticRetryTyped = false;
        state.semanticRetryGeneration = (state.semanticRetryGeneration || 0) + 1;
      }

      function invalidateKnowledgeSearchOwner(state, nextKind, nextQuery) {
        if (state.pendingSearchTimer !== null) {
          clearTimeout(state.pendingSearchTimer);
          state.pendingSearchTimer = null;
        }
        invalidateKnowledgeSemanticRetry(state);
        state.searchGeneration = (state.searchGeneration || 0) + 1;
        state.searchIntentKind = normalizeKnowledgeKind(nextKind);
        state.searchIntentQuery = String(nextQuery || "").trim();
        state.queuedSearchIntent = state.inFlightSearchIntent && state.searchIntentQuery
          ? {
              generation: state.searchGeneration,
              kind: state.searchIntentKind,
              query: state.searchIntentQuery,
              selectionGeneration: state.selectionGeneration,
            }
          : null;
      }

      function updateKnowledgeSearchIntent(state, knowledgeKind, query) {
        const kind = normalizeKnowledgeKind(knowledgeKind || state.kind);
        const normalizedQuery = String(query || "").trim();
        if (
          state.searchIntentKind !== kind ||
          state.searchIntentQuery !== normalizedQuery
        ) {
          invalidateKnowledgeSearchOwner(state, kind, normalizedQuery);
        }
        return {
          generation: state.searchGeneration,
          kind,
          query: normalizedQuery,
          selectionGeneration: state.selectionGeneration,
        };
      }

      function knowledgeSearchIntentIsCurrent(state, intent) {
        return Boolean(
          intent &&
          intent.generation === state.searchGeneration &&
          intent.kind === normalizeKnowledgeKind(state.kind) &&
          intent.kind === state.searchIntentKind &&
          intent.query === state.query.trim() &&
          intent.query === state.searchIntentQuery,
        );
      }

      function scheduleKnowledgeSemanticRetry(windowId, knowledgeKind, state) {
        if (state.semanticRetryTimer !== null) {
          clearTimeout(state.semanticRetryTimer);
          state.semanticRetryTimer = null;
        }
        const delay =
          KNOWLEDGE_SEMANTIC_RETRY_DELAYS[
            Math.min(
              state.semanticRetryIndex,
              KNOWLEDGE_SEMANTIC_RETRY_DELAYS.length - 1,
            )
          ];
        state.semanticRetryIndex += 1;
        state.semanticRetryActive = true;
        const retryGeneration = state.semanticRetryGeneration || 0;
        const intent = updateKnowledgeSearchIntent(
          state,
          knowledgeKind || state.kind,
          state.query,
        );
        state.semanticRetryTimer = setTimeout(() => {
          state.semanticRetryTimer = null;
          const liveState = knowledgeBridgeStateMap.get(windowId);
          if (liveState !== state) {
            return;
          }
          if (retryGeneration !== (state.semanticRetryGeneration || 0)) {
            // Stale timer from an invalidated retry window (AS-17.2).
            return;
          }
          if (!workspaceWindowById(windowId) || !knowledgeSearchIntentIsCurrent(state, intent)) {
            return;
          }
          const latestIntent = {
            ...intent,
            selectionGeneration: state.selectionGeneration,
          };
          if (state.inFlightSearchIntent) {
            // One in-flight attempt, one latest queued intent.
            state.queuedSearchIntent = latestIntent;
            state.queuedSearchQuery = latestIntent.query;
            return;
          }
          const sentNow = sendKnowledgeSemanticSearch(windowId, latestIntent);
          if (!sentNow && state.semanticRetryTyped === true) {
            scheduleKnowledgeSemanticRetry(windowId, latestIntent.kind, state);
          }
        }, delay);
      }

      // SPEC #3170 AS-17.2: disconnect invalidates every retry owner;
      // reconnect restarts a degraded still-open window/query at 5 seconds.
      function handleKnowledgeTransportChange(online) {
        for (const [windowId, state] of knowledgeBridgeStateMap.entries()) {
          if (!online && state.revisionPending) {
            state.revisionPending = false;
            state.autoRefreshDirty = true;
          }
          if (!isSilentSemanticKind(state.kind)) {
            continue;
          }
          if (!online) {
            const query = state.query.trim();
            const wasActive = Boolean(query) && Boolean(
              state.semanticRetryActive ||
              state.searchInFlight ||
              state.inFlightSearchIntent ||
              state.pendingSearchTimer !== null
            );
            const wasTyped = state.semanticRetryTyped === true;
            if (state.pendingSearchTimer !== null) {
              clearTimeout(state.pendingSearchTimer);
              state.pendingSearchTimer = null;
            }
            invalidateKnowledgeSemanticRetry(state);
            state.semanticRetryActive = wasActive;
            state.semanticRetryTyped = wasActive && wasTyped;
            state.searchGeneration = (state.searchGeneration || 0) + 1;
            state.searchIntentKind = normalizeKnowledgeKind(state.kind);
            state.searchIntentQuery = query;
            state.queuedSearchIntent = query
              ? {
                  generation: state.searchGeneration,
                  kind: state.searchIntentKind,
                  query,
                  selectionGeneration: state.selectionGeneration,
                }
              : null;
            state.queuedSearchQuery = query;
            state.inFlightSearchIntent = null;
            state.searchInFlight = false;
            state.inFlightSearchRequestId = 0;
            state.searching = false;
            continue;
          }
          if (!state.semanticRetryActive) {
            continue;
          }
          if (!workspaceWindowById(windowId)) {
            continue;
          }
          if (!state.query.trim()) {
            continue;
          }
          state.semanticRetryIndex = 0;
          scheduleKnowledgeSemanticRetry(windowId, state.kind, state);
        }
        if (online) refreshVisibleKnowledgeWindows();
      }

      function sendKnowledgeSemanticSearch(windowId, intent) {
        const state = knowledgeBridgeStateMap.get(windowId);
        if (
          !state ||
          !workspaceWindowById(windowId) ||
          state.inFlightSearchIntent ||
          !knowledgeSearchIntentIsCurrent(state, intent)
        ) {
          return false;
        }
        const requestId = nextKnowledgeSearchRequestId++;
        const message = {
          kind: "search_knowledge_bridge",
          id: windowId,
          knowledge_kind: intent.kind,
          query: intent.query,
          request_id: requestId,
          selected_number: state.selectedNumber ?? null,
        };
        state.searchRequestId = requestId;
        state.inFlightSearchRequestId = requestId;
        state.searchInFlight = true;
        state.searching = true;
        state.inFlightSearchIntent = { ...intent, requestId };
        const sentNow = isSilentSemanticKind(intent.kind)
          ? sendKnowledgeSemanticSearchNow(message)
          : (send(message), true);
        if (!sentNow) {
          if (state.inFlightSearchIntent?.requestId === requestId) {
            state.searching = false;
            state.searchInFlight = false;
            state.inFlightSearchRequestId = 0;
            state.inFlightSearchIntent = null;
          }
          state.semanticRetryActive = true;
          return false;
        }
        if (
          state.queuedSearchIntent?.generation === intent.generation &&
          state.queuedSearchIntent?.kind === intent.kind &&
          state.queuedSearchIntent?.query === intent.query
        ) {
          state.queuedSearchIntent = null;
          state.queuedSearchQuery = "";
        }
        return true;
      }

      function dispatchLatestKnowledgeSearchIntent(windowId, state) {
        const nextIntent = state.queuedSearchIntent;
        state.queuedSearchIntent = null;
        state.queuedSearchQuery = "";
        if (knowledgeSearchIntentIsCurrent(state, nextIntent)) {
          return sendKnowledgeSemanticSearch(windowId, {
            ...nextIntent,
            selectionGeneration: state.selectionGeneration,
          });
        }
        state.searching = false;
        return false;
      }

      function scheduleKnowledgeSearch(windowId, knowledgeKind) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        if (state.pendingSearchTimer !== null) {
          clearTimeout(state.pendingSearchTimer);
          state.pendingSearchTimer = null;
        }
        const query = state.query.trim();
        const intent = updateKnowledgeSearchIntent(state, knowledgeKind, query);
        state.error = "";
        if (!query) {
          state.searching = false;
          state.queuedSearchQuery = "";
          state.queuedSearchIntent = null;
          restoreKnowledgeBaseEntries(state);
          renderKnowledgeBridge(windowId);
          return;
        }
        // AS-17.7: local number/title/metadata/label filtering from
        // baseEntries is visible immediately; the semantic completion later
        // replaces it with authoritative rows.
        applyLocalKnowledgeFilter(state, query);
        queueKnowledgeSearch(windowId, state, intent);
        renderKnowledgeBridge(windowId);
      }

      // Dispatch scheduling is separate from changing the visible results so
      // an unchanged base list can resume a cancelled timer without filtering
      // authoritative semantic results or replacing their DOM.
      function queueKnowledgeSearch(windowId, state, intent) {
        if (state.loading && state.baseEntries.length === 0) {
          state.searching = true;
          return;
        }
        if (
          isSilentSemanticKind(intent.kind) &&
          state.semanticRetryActive &&
          state.semanticRetryTimer !== null &&
          !state.inFlightSearchIntent
        ) {
          state.searching = false;
          return;
        }
        if (state.inFlightSearchIntent) {
          state.queuedSearchIntent = intent;
          state.queuedSearchQuery = intent.query;
          state.searching = true;
          return;
        }
        state.searching = true;
        state.pendingSearchTimer = setTimeout(() => {
          state.pendingSearchTimer = null;
          const liveState = knowledgeBridgeStateMap.get(windowId);
          if (liveState !== state || !workspaceWindowById(windowId)) {
            return;
          }
          if (!knowledgeSearchIntentIsCurrent(state, intent)) {
            return;
          }
          if (!intent.query) {
            state.searching = false;
            restoreKnowledgeBaseEntries(state);
            renderKnowledgeBridge(windowId);
            return;
          }
          if (state.inFlightSearchIntent) {
            state.queuedSearchIntent = {
              ...intent,
              selectionGeneration: state.selectionGeneration,
            };
            state.queuedSearchQuery = intent.query;
            renderKnowledgeBridge(windowId);
            return;
          }
          sendKnowledgeSemanticSearch(windowId, {
            ...intent,
            selectionGeneration: state.selectionGeneration,
          });
        }, 250);
      }

      function dispatchKnowledgeDetailRequest(
        windowId,
        knowledgeKind,
        number,
        { explicit = false } = {},
      ) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        const previousNumber = state.selectedNumber;
        const openingPreview = explicit && state.kind === "issue" && state.previewHidden;
        if (explicit) {
          state.selectedNumber = number;
          if (state.kind === "issue") state.previewHidden = false;
          // Issue/SPEC selection is a local transition before any I/O.
          state.selectionGeneration = (state.selectionGeneration || 0) + 1;
          state.error = "";
          const findRow = (rows) =>
            Array.isArray(rows)
              ? rows.find((entry) => entry && entry.number === number)
              : null;
          const row = findRow(state.entries) || findRow(state.baseEntries) ||
            (state.kind === "issue" ? issueDetailEntry(state, number) : null);
          const authoritative = state.detail && state.detail.number === number;
          if (!authoritative) {
            state.detail = row
              ? {
                  number: row.number,
                  title: row.title || "",
                  subtitle: `#${row.number}`,
                  state: row.state || "",
                  phase: row.phase ?? null,
                  labels: Array.isArray(row.labels) ? row.labels.slice() : [],
                  sections: [],
                  launch_issue_number: row.number,
                  related_works: [],
                }
              : null;
          }
        } else if (number !== state.selectedNumber) {
          return false;
        }
        state.detailLoading = true;
        const requestId = nextKnowledgeLoadRequestId++;
        state.detailRequestId = requestId;
        state.detailRequestSelectionGeneration = state.selectionGeneration;
        state.detailRequestNumber = number;
        const effectiveKind = knowledgeKind || state.kind;
        if (explicit) {
          renderKnowledgeSelection(windowId, state, previousNumber, { focusPreview: openingPreview });
        } else {
          renderKnowledgeDetailOnly(windowId, state);
        }
        send({
          kind: "select_knowledge_bridge_entry",
          id: windowId,
          knowledge_kind: effectiveKind,
          request_id: requestId,
          number,
        });
        return true;
      }

      function requestKnowledgeDetail(windowId, knowledgeKind, number) {
        return dispatchKnowledgeDetailRequest(
          windowId,
          knowledgeKind,
          number,
          { explicit: true },
        );
      }

      // SPEC-2017 US-8 — push a Kanban phase change to the backend.
      // The optimistic UI move lives in renderKanbanCard's drop handler;
      // this helper just wires the WebSocket request and reserves a
      // request_id so knowledge_bridge_phase_updated can correlate the
      // response back to a specific drop. target_phase=null means
      // "Backlog" — the backend strips every phase/* label.
      function sendUpdateKnowledgePhase(windowId, issueNumber, targetPhase) {
        const requestId = nextKnowledgeLoadRequestId++;
        send({
          kind: "update_knowledge_bridge_phase",
          id: windowId,
          request_id: requestId,
          issue_number: issueNumber,
          target_phase: targetPhase,
        });
        return requestId;
      }


      function knowledgeHeading(kind) {
        switch (kind) {
          case "issue":
            return "Cached work items";
          case "spec":
            return "Cached work items";
          default:
            return "Knowledge Bridge";
        }
      }

      function knowledgeSearchPlaceholder(kind) {
        switch (kind) {
          case "issue":
            return "Semantic search work items";
          case "spec":
            return "Semantic search work items";
          default:
            return "Search";
        }
      }

      const KNOWLEDGE_PHASES = new Set([
        "draft",
        "planning",
        "implementation",
        "review",
        "done",
      ]);

      function isKnowledgePhaseLabel(label) {
        return typeof label === "string" && label.startsWith("phase/");
      }

      function canonicalKnowledgePhase(phase) {
        const value = String(phase || "");
        return KNOWLEDGE_PHASES.has(value) ? value : null;
      }

      function knowledgePhaseFromLabels(labels = []) {
        for (const label of Array.isArray(labels) ? labels : []) {
          if (!isKnowledgePhaseLabel(label)) continue;
          const phase = canonicalKnowledgePhase(label.slice("phase/".length));
          if (phase) return phase;
        }
        return null;
      }

      function effectiveKnowledgePhase(entry) {
        if (entry?.state === "closed") return "done";
        return canonicalKnowledgePhase(entry?.phase)
          || knowledgePhaseFromLabels(entry?.labels)
          || "backlog";
      }

      function knowledgePhaseDisplayName(phase) {
        switch (phase) {
          case "draft":
            return "Draft";
          case "planning":
            return "Planning";
          case "implementation":
            return "Implementation";
          case "review":
            return "Review";
          case "done":
            return "Done";
          default:
            return "Backlog";
        }
      }

      function visibleKnowledgeLabels(labels = []) {
        return (Array.isArray(labels) ? labels : []).filter(
          (label) => !isKnowledgePhaseLabel(label),
        );
      }

      function staleKnowledgePhaseWarning(entry) {
        const storedPhase = canonicalKnowledgePhase(entry?.phase)
          || knowledgePhaseFromLabels(entry?.labels);
        if (entry?.state === "closed" && storedPhase && storedPhase !== "done") {
          return `Stored phase/${storedPhase}; lifecycle is Done`;
        }
        return "";
      }

      function knowledgeDetailChip(detail, knowledgeKind = "spec") {
        if (knowledgeKind === "issue") {
          const rawState = String(detail?.state || "open").toLowerCase();
          return {
            className: rawState === "closed" ? "closed" : "open",
            label: rawState === "closed" ? "Closed" : "Open",
          };
        }
        const effectivePhase = effectiveKnowledgePhase(detail);
        const rawState = String(detail?.state || "").toLowerCase();
        if (
          rawState
          && rawState !== "open"
          && rawState !== "closed"
          && effectivePhase === "backlog"
        ) {
          return {
            className: rawState,
            label: rawState,
          };
        }
        return {
          className: effectivePhase === "done" ? "closed" : "open",
          label: knowledgePhaseDisplayName(effectivePhase),
        };
      }

      function appendKnowledgeRelatedCountChips(container, entry, className) {
        const relatedWorkCount = entry.related_work_count || 0;
        const relatedSessionCount = entry.related_session_count || 0;
        if (relatedWorkCount > 0) {
          container.appendChild(
            createNode(
              "span",
              className,
              `${relatedWorkCount} work${relatedWorkCount === 1 ? "" : "s"}`,
            ),
          );
        }
        if (relatedSessionCount > 0) {
          container.appendChild(
            createNode(
              "span",
              className,
              `${relatedSessionCount} session${relatedSessionCount === 1 ? "" : "s"}`,
            ),
          );
        }
      }

      function shortRelatedSessionId(value) {
        const text = String(value || "").trim();
        if (text.length <= 12) {
          return text || "unknown";
        }
        return `${text.slice(0, 8)}...${text.slice(-4)}`;
      }

      function knowledgeRelatedWorkPendingKey(sessionId) {
        return `session:${sessionId}`;
      }

      function isKnowledgeRelatedResumePending(sessionId) {
        return Boolean(
          sessionId
            && launchPending
            && launchPending.isPending(knowledgeRelatedWorkPendingKey(sessionId)),
        );
      }

      function addKnowledgeRelatedSessionId(target, value) {
        const text = String(value || "").trim();
        if (text) {
          target.add(text);
        }
      }

      function knowledgeRelatedLiveSessionIds(agent, session) {
        const ids = new Set();
        addKnowledgeRelatedSessionId(ids, session?.agent_session_id);
        addKnowledgeRelatedSessionId(ids, session?.session_id);
        if (session?.is_active !== false) {
          addKnowledgeRelatedSessionId(ids, agent?.session_id);
        }
        return ids;
      }

      function knowledgeRelatedLiveWindowCandidates() {
        const windows = [];
        const seen = new Set();
        const append = (windowData) => {
          if (!windowData) {
            return;
          }
          const key = windowData.id || windowData.session_id || windows.length;
          if (seen.has(key)) {
            return;
          }
          seen.add(key);
          windows.push(windowData);
        };
        if (windowMap) {
          for (const windowData of windowMap.values()) {
            append(windowData);
          }
        }
        if (typeof getWorkspaceWindows === "function") {
          for (const windowData of getWorkspaceWindows() || []) {
            append(windowData);
          }
        }
        return windows;
      }

      function isKnowledgeRelatedLiveWindow(windowData) {
        const status = String(windowData?.status || "").toLowerCase();
        return status !== "stopped" && status !== "error";
      }

      function buildKnowledgeRelatedLiveSessionWindowsByConversation(works) {
        const windowsBySessionId = new Map();
        for (const windowData of knowledgeRelatedLiveWindowCandidates()) {
          if (!isKnowledgeRelatedLiveWindow(windowData)) {
            continue;
          }
          const sessionId = String(windowData?.session_id || "").trim();
          if (sessionId) {
            windowsBySessionId.set(sessionId, windowData);
          }
        }
        const liveSessionWindowsByConversation = new Map();
        for (const work of works || []) {
          for (const agent of work?.agents || []) {
            const liveWindow = windowsBySessionId.get(
              String(agent?.session_id || "").trim(),
            );
            if (!liveWindow) {
              continue;
            }
            for (const session of agent?.sessions || []) {
              const conversation = String(session?.agent_session_id || "").trim();
              if (conversation && !liveSessionWindowsByConversation.has(conversation)) {
                liveSessionWindowsByConversation.set(conversation, liveWindow);
              }
            }
          }
        }
        return liveSessionWindowsByConversation;
      }

      function findKnowledgeRelatedLiveWindow(agent, session, liveSessionWindowsByConversation) {
        const conversationWindow = session?.agent_session_id
          ? liveSessionWindowsByConversation?.get(String(session.agent_session_id).trim())
          : null;
        if (conversationWindow && isKnowledgeRelatedLiveWindow(conversationWindow)) {
          return conversationWindow;
        }
        const sessionIds = knowledgeRelatedLiveSessionIds(agent, session);
        if (sessionIds.size === 0) {
          return null;
        }
        for (const windowData of knowledgeRelatedLiveWindowCandidates()) {
          const liveIds = [
            windowData?.session_id,
            windowData?.agent_session_id,
          ]
            .map((value) => String(value || "").trim())
            .filter(Boolean);
          if (!liveIds.some((value) => sessionIds.has(value))) {
            continue;
          }
          if (!isKnowledgeRelatedLiveWindow(windowData)) {
            continue;
          }
          return windowData;
        }
        return null;
      }

      function focusKnowledgeRelatedSession(agent, session, liveSessionWindowsByConversation) {
        const liveWindow = findKnowledgeRelatedLiveWindow(
          agent,
          session,
          liveSessionWindowsByConversation,
        );
        if (!liveWindow?.id) {
          return false;
        }
        focusWindowLocally(liveWindow.id);
        sendWindowFocus(liveWindow.id);
        return true;
      }

      function resumeKnowledgeRelatedSession(agent, session) {
        const sessionId = String(agent?.session_id || "").trim();
        if (!sessionId) {
          return false;
        }
        const bounds = typeof visibleBounds === "function" ? visibleBounds() : null;
        if (!bounds) {
          return false;
        }
        const operationId = createLaunchOperationId("resume");
        if (
          launchPending
          && !launchPending.begin(
            knowledgeRelatedWorkPendingKey(sessionId),
            "Resume",
            operationId,
          )
        ) {
          return false;
        }
        send({
          kind: "resume_workspace_agent",
          operation_id: operationId,
          session_id: sessionId,
          agent_session_id: session?.agent_session_id || null,
          bounds,
        });
        return true;
      }

      function renderKnowledgeRelatedSessionAction(agent, session, liveSessionWindowsByConversation) {
        const liveWindow = findKnowledgeRelatedLiveWindow(
          agent,
          session,
          liveSessionWindowsByConversation,
        );
        if (liveWindow) {
          const button = createNode("button", "wizard-button is-compact", "Focus");
          button.type = "button";
          button.dataset.action = "focus-related-session";
          button.dataset.sessionId = agent.session_id || "";
          if (session?.agent_session_id) {
            button.dataset.agentSessionId = session.agent_session_id;
            button.setAttribute("aria-label", `Focus conversation ${session.agent_session_id}`);
          } else {
            button.setAttribute("aria-label", "Focus related session");
          }
          button.addEventListener("click", () => {
            focusKnowledgeRelatedSession(agent, session, liveSessionWindowsByConversation);
          });
          return button;
        }
        if (!agent?.session_id || session?.resumable === false) {
          return null;
        }
        const button = createNode("button", "wizard-button is-compact", "Resume");
        button.type = "button";
        button.dataset.action = "resume-related-session";
        button.dataset.sessionId = agent.session_id;
        if (session?.agent_session_id) {
          button.dataset.agentSessionId = session.agent_session_id;
          button.setAttribute("aria-label", `Resume conversation ${session.agent_session_id}`);
        } else {
          button.setAttribute("aria-label", "Resume related session");
        }
        if (isKnowledgeRelatedResumePending(agent.session_id)) {
          button.disabled = true;
          button.textContent = "Resuming...";
          button.classList.add("is-pending");
        }
        button.addEventListener("click", () => {
          if (resumeKnowledgeRelatedSession(agent, session)) {
            button.disabled = true;
            button.textContent = "Resuming...";
            button.classList.add("is-pending");
          }
        });
        return button;
      }

      function renderKnowledgeRelatedWorks(detail) {
        // Display labels are derived without changing the cached provider identity.
        const works = (Array.isArray(detail?.related_works) ? detail.related_works : [])
          .map(work => ({
            ...work,
            agents: (work.agents || []).map(agent => ({
              ...agent,
              display_name: agent.agent_name
                ? `${agent.agent_name} (${agent.display_name || agent.agent_id || "Agent"})`
                : agent.display_name,
            })),
          }));
        if (works.length === 0) {
          return null;
        }

        const section = createNode("section", "knowledge-section knowledge-related-works");
        section.appendChild(createNode("div", "knowledge-section-title", "Related Work"));
        const list = createNode("div", "knowledge-related-work-list");
        const liveSessionWindowsByConversation =
          buildKnowledgeRelatedLiveSessionWindowsByConversation(works);
        const projectedWorks = (getActiveWorkProjection?.()?.active_works || [])
          .flatMap(workspace => workspace.works || []);
        for (const work of works) {
          const card = createNode("article", "knowledge-related-work");
          card.dataset.workId = work.id;
          const head = createNode("div", "knowledge-related-work-head");
          head.appendChild(
            createNode("div", "knowledge-related-work-title", work.title || "Untitled work"),
          );
          if (work.status_category) {
            head.appendChild(
              createNode(
                "span",
                `knowledge-related-status knowledge-related-status--${work.status_category}`,
                work.status_category,
              ),
            );
          }
          card.appendChild(head);

          // Closing targets a launch-scoped Work, never its branch Workspace.
          const projectedWork = projectedWorks.find(item => item.id === work.id);
          if (projectedWork && !["done", "discarded"].includes(projectedWork.lifecycle_state) &&
              (projectedWork.manual_close_allowed === true || projectedWork.close_blocked_reason)) {
            const actions = createNode("div", "workspace-detail-work-actions");
            for (const [action, label, closeKind] of [
              ["close-work-done", "Done", "done"],
              ["close-work-discard", "Discard", "discarded"],
            ]) {
              const button = createNode("button", "wizard-button is-compact", label);
              button.type = "button";
              button.dataset.action = action;
              button.dataset.workId = projectedWork.id;
              if (closeKind === "discarded") button.classList.add("destructive");
              if (projectedWork.manual_close_allowed !== true) {
                button.disabled = true;
                button.title = projectedWork.close_blocked_reason;
                button.setAttribute("aria-label", `${label} unavailable: ${projectedWork.close_blocked_reason}`);
              } else {
                button.addEventListener("click", () =>
                  send({ kind: "close_work", work_id: projectedWork.id, close_kind: closeKind }),
                );
              }
              actions.appendChild(button);
            }
            card.appendChild(actions);
          }

          const meta = createNode("div", "knowledge-related-work-meta");
          if (work.branch) {
            meta.appendChild(createNode("span", "knowledge-meta-copy", work.branch));
          }
          if (work.worktree_path) {
            meta.appendChild(createNode("span", "knowledge-meta-copy", work.worktree_path));
          }
          if (meta.childElementCount > 0) {
            card.appendChild(meta);
          }

          const agents = Array.isArray(work.agents) ? work.agents : [];
          for (const agent of agents) {
            const agentNode = createNode("div", "knowledge-related-agent");
            agentNode.appendChild(
              createNode(
                "div",
                "knowledge-related-agent-name",
                agent.display_name || agent.agent_id || "Agent",
              ),
            );
            const sessions = Array.isArray(agent.sessions) ? agent.sessions : [];
            for (const session of sessions) {
              const sessionNode = createNode(
                "div",
                `knowledge-related-session${session.is_active ? " is-active" : ""}`,
              );
              sessionNode.appendChild(
                createNode(
                  "span",
                  "knowledge-related-session-label",
                  `Session ${shortRelatedSessionId(session.agent_session_id)}`,
                ),
              );
              sessionNode.appendChild(
                createNode(
                  "span",
                  "knowledge-related-session-state",
                  session.is_active ? "Current" : "Past",
                ),
              );
              const action = renderKnowledgeRelatedSessionAction(
                agent,
                session,
                liveSessionWindowsByConversation,
              );
              if (action) {
                sessionNode.appendChild(action);
              }
              agentNode.appendChild(sessionNode);
            }
            if (sessions.length === 0) {
              agentNode.appendChild(
                createNode("div", "knowledge-related-session-empty", "No session yet"),
              );
            }
            card.appendChild(agentNode);
          }

          list.appendChild(card);
        }
        section.appendChild(list);
        return section;
      }



      function filteredKnowledgeEntries(state) {
        const query = state.query.trim().toLowerCase();
        if (!query) {
          return state.entries;
        }
        return state.entries.filter((entry) =>
          [
            `#${entry.number}`,
            entry.title,
            entry.meta,
            ...(entry.labels || []),
          ]
            .join(" ")
            .toLowerCase()
            .includes(query),
        );
      }

      function filteredIssueEntries(state) {
        // `state.entries` is already the immediate local filter while a
        // request is pending and becomes the authoritative semantic result
        // set on completion. Reapplying substring filtering here would hide
        // valid semantic matches whose wording differs from the query.
        const entries = new Map((Array.isArray(state.entries) ? state.entries : [])
          .map(entry => [entry.number, entry]));
        // The inbox is already a complete row source. A separate knowledge
        // load supplies details, but may still be pending when Monitor data
        // arrives at startup. Keep semantic search's result set authoritative.
        if (!state.query.trim()) {
          for (const live of Object.values(issueMonitorModel.read().inboxByIssue)) {
            const issue = live.issue;
            if (entries.has(issue.number)) continue;
            const labels = Array.isArray(issue.labels) ? issue.labels : [];
            entries.set(issue.number, { number: issue.number, title: issue.title,
              state: issue.state, labels, is_spec: labels.includes("gwt-spec"),
              monitor_state: live.state, exclusion_reason: live.exclusion_reason,
              meta: "", linked_branch_count: 0, related_work_count: 0, related_session_count: 0 });
          }
        }
        return [...entries].map(([, entry]) => queueProjectedEntry(entry));
      }

      function queueProjectedEntry(entry) {
        const live = issueMonitorModel.read().inboxByIssue[entry.number];
        if (live) entry = { ...entry, state: live.issue.state ?? entry.state, monitor_state: live.state,
          exclusion_reason: live.exclusion_reason ?? null };
        const diagnosis = live?.error_message;
        entry = { ...entry, error_message: typeof diagnosis === "string" ? diagnosis : null,
          readiness_diagnosis: typeof diagnosis === "string" &&
          diagnosis.startsWith("SessionStart readiness pending:") ? diagnosis : null };
        if (!Array.isArray(issueMonitorModel.read().status.terminal_queue)) return entry;
        const index = issueMonitorModel.read().status.terminal_queue.findIndex(item => item.number === entry.number);
        const queued = index < 0 ? null : issueMonitorModel.read().status.terminal_queue[index];
        return { ...entry, queue_position: index < 0 ? null : index + 1,
          queued_by: queued?.queued_by,
          priority: queued?.priority,
          priority_reason: queued?.priority_reason,
          assigned_by: queued?.assigned_by,
          assigned_at: queued?.assigned_at,
          monitor_state: index >= 0 && (!entry.monitor_state || entry.monitor_state === "queued")
            ? "queued" : index < 0 && entry.monitor_state === "queued" ? null : entry.monitor_state };
      }

      function issueQueueColumnForEntry(entry, windowId) {
        const work = issueWorkRowForEntry(getActiveWorkProjection?.(), entry);
        const faces = issueRowFaces(windowId, entry, work);
        const live = faces.inlineWindow || faces.canvasWindow;
        const owned = live?.linked_issue_number == null || Number(live.linked_issue_number) === entry.number;
        return issueQueueColumn(entry, work, owned ? live : null);
      }

      function renderIssueQueueBoard(windowId, state, list, entries) {
        const feedback = list.querySelector(".issue-queue-feedback") || createNode("div", "issue-queue-feedback");
        feedback.setAttribute("role", "status");
        feedback.textContent = state.queueFeedback || "";
        const board = list.querySelector(".issue-queue-board") || createNode("div", "issue-queue-board");
        if (!board.parentNode) list.prepend(feedback, board);
        // Drop handlers read this current snapshot, never a previous render's array.
        state.queueBoardEntries = entries;
        state.queueSelection ??= new Set();
        const queued = canonicalQueuedKnowledgeEntries(state);
        const queuePositions = new Map(queued.map((entry, index) => [entry.number, index]));
        for (const phase of ["backlog", "queued", "active", "done"]) {
          const column = board.querySelector(`[data-queue-column="${phase}"]`) || createNode("section", "issue-queue-column");
          column.dataset.queueColumn = phase;
          const label = phase[0].toUpperCase() + phase.slice(1);
          column.setAttribute("aria-label", `${label} column`);
          const items = entries.filter(entry => issueQueueColumnForEntry(entry, windowId) === phase);
          if (phase === "queued") items.sort((a,b) => a.queue_position - b.queue_position);
          const explanations = {
            backlog: "Open issues that are not queued and have no running agent.",
            queued: "Issues waiting to launch. Their queue positions show the launch order.",
            active: "Issues with an agent launching or running. Card output is a read-only preview.",
            done: "Closed issues or issues the monitor reports as merged or released.",
          };
          let heading = column.querySelector("h3");
          let body = column.querySelector(".issue-queue-items");
          if (!heading) {
            heading = createNode("h3", "issue-queue-heading");
            body = createNode("div", "issue-queue-items");
            column.append(heading, body);
          }
          const headingText = `${label} · ${items.length}`;
          if (heading.textContent !== headingText) heading.textContent = headingText;
          explainIssueControl(heading, explanations[phase], `column-${phase}`);
          reconcileKnowledgeList(body, items, {
            scrollRoot: list.closest(".knowledge-list-pane") || list,
            virtualize: entries.length > 200,
            mounted: () => publishIssueControlExplanations(windowId),
            version: entry => {
              const work = issueWorkRowForEntry(getActiveWorkProjection?.(), entry);
              const faces = issueRowFaces(windowId, entry, work);
              return JSON.stringify([{ ...entry, queue_position: entry.queue_position ?? null, monitor_state: entry.monitor_state ?? null }, work, faces, faces.inlineWindow ? issueAgentElapsedLabel(faces.inlineWindow) : null,
                work ? workAttentionFor?.(work) : null,
                issueTierChips(entry.number), queuePositions.has(entry.number) ? [queuePositions.get(entry.number), queued.length] : null,
                phase === "active" ? issueProjectWindows(windowId).filter(target =>
                  target.agent_id && (Number(target.linked_issue_number ?? target.placement?.issue_number) === entry.number ||
                    work?.agents?.some(agent => agent.window_id === target.id || (target.session_id && agent.session_id === target.session_id)))) : null]);
            },
            update: (row, entry) => {
              row.classList.toggle("selected", state.selectedNumber === entry.number);
              const select = row.querySelector(".knowledge-row-select");
              if (state.selectedNumber === entry.number) {
                if (select.getAttribute("aria-current") !== "true") select.setAttribute("aria-current", "true");
              } else if (select.hasAttribute("aria-current")) select.removeAttribute("aria-current");
              const checkbox = row.querySelector('[data-action="queue-select"]');
              if (checkbox) checkbox.checked = state.queueSelection.has(entry.number);
            },
            render: entry => {
              const row = renderIssueRow(windowId, state, entry);
              if (phase === "active") {
                const work = issueWorkRowForEntry(getActiveWorkProjection?.(), entry);
                const agents = work?.agents || [];
                const windows = issueProjectWindows(windowId);
                for (const target of windows) {
                  if (!target.agent_id || target.preset === "pm" ||
                      !ISSUE_ROW_STOPPABLE_AGENT_STATUSES.has(target.status)) continue;
                  const linked = Number(target.linked_issue_number ?? target.placement?.issue_number);
                  const belongs = Number.isFinite(linked)
                    ? linked === entry.number
                    : agents.some(agent => agent.window_id === target.id ||
                      (target.session_id && agent.session_id === target.session_id));
                  if (!belongs) continue;
                  if (!row.classList.contains("has-live-output")) {
                    row.querySelector(".issue-agent-status")?.remove();
                    row.classList.add("has-live-output");
                  }
                  const output = renderIssueAgentStatusRow(windowId, state, entry,
                    target.placement?.kind === "issue_preview"
                      ? { inlineWindow: target } : { canvasWindow: target });
                  output.classList.add("issue-card-output");
                  output.setAttribute("role", "group");
                  const title = windowDisplayTitle?.(target) || target.title || target.id;
                  output.setAttribute("aria-label", `Read-only live output: ${title}`);
                  output.querySelector(".issue-agent-status-output")?.remove();
                  const label = output.querySelector(".issue-agent-status-meta");
                  label.classList.add("issue-card-output-label");
                  label.textContent = `Read-only · ${label.textContent}`;
                  const screen = createNode("div", "issue-card-output-screen");
                  screen.appendChild(createNode("pre", "issue-card-output-text",
                    terminalPreviewText.get(target.id) ?? "Waiting for output"));
                  output.appendChild(screen);
                  row.appendChild(output);
                }
              }
              if (phase === "backlog" || phase === "queued") {
                const selection = createNode("label", "issue-queue-select");
                const checkbox = createNode("input");
                checkbox.type = "checkbox";
                checkbox.dataset.action = "queue-select";
                checkbox.checked = state.queueSelection.has(entry.number);
                checkbox.setAttribute("aria-label", `Select issue #${entry.number} for queue move`);
                checkbox.addEventListener("click", event => {
                  event.stopPropagation();
                  if (state.queueSelection.has(entry.number)) state.queueSelection.delete(entry.number);
                  else state.queueSelection.add(entry.number);
                });
                selection.addEventListener("click", event => event.stopPropagation());
                selection.append(checkbox, createNode("span", "", "Select"));
                row.prepend(selection);
                row.draggable = true;
                row.addEventListener("dragstart", event => {
                  event.dataTransfer?.setData("text/plain", String(entry.number));
                });
              }
              return row;
            },
          });
          let empty = column.querySelector(".knowledge-empty");
          if (!items.length && !empty) {
            empty = createNode("div", "knowledge-empty", phase === "queued"
              ? "Nothing will launch until an issue is queued." : `No ${phase} items`);
            column.appendChild(empty);
          } else if (items.length) empty?.remove();
          if (column.parentNode) continue;
          column.addEventListener("dragover", event => event.preventDefault());
          column.addEventListener("drop", event => {
            event.preventDefault();
            const explain = message => {
              state.queueFeedback = message;
              issueWindowViewModel.update(() => ({windowId}));
            };
            const raw = event.dataTransfer?.getData("text/plain") || "";
            const number = /^\d+$/.test(raw) ? Number(raw) : NaN;
            if (!Number.isSafeInteger(number) || number <= 0) {
              explain("Drop an issue card into Backlog or Queued.");
              return;
            }
            const currentEntries = state.queueBoardEntries || [];
            const entry = currentEntries.find(item => item.number === number);
            if (!entry) {
              explain(`Issue #${number} is no longer on this board. Refresh and try again.`);
              return;
            }
            if (phase === "active" || phase === "done") {
              explain(`${label} is controlled by the monitor and work lifecycle; drop into Backlog or Queued.`);
              return;
            }
            const origin = issueQueueColumnForEntry(entry, windowId);
            if (origin !== "backlog" && origin !== "queued") {
              explain("Active and Done issues cannot be moved into the queue.");
              return;
            }
            let message;
            if (phase === "queued" && origin === "queued") {
              const target = event.target?.closest?.("[data-issue-number]");
              const queued = canonicalQueuedKnowledgeEntries(state);
              const targetIndex = queued.findIndex(item => item.number === Number(target?.dataset.issueNumber));
              message = {kind:"issue_monitor_queue_move", issue_number:number, position:targetIndex < 0 ? Math.max(0, queued.length - 1) : targetIndex};
            } else if (origin !== phase) {
              const numbers = state.queueSelection.has(number)
                ? currentEntries.filter(item => state.queueSelection.has(item.number) && issueQueueColumnForEntry(item, windowId) === origin).map(item => item.number)
                : [number];
              message = {kind:phase === "queued" ? "issue_monitor_queue_push" : "issue_monitor_queue_remove", issue_numbers:numbers};
            } else {
              explain(`Issue #${number} is already in ${label}.`);
              return;
            }
            try {
              const outcome = send(message);
              explain(outcome === "unavailable" || outcome === false
                ? "Queue change could not be sent. Reconnect and try again."
                : outcome === "queued"
                  ? "Queue change is waiting for reconnection; it has not been confirmed."
                  : "Queue change requested; waiting for server confirmation.");
            } catch (error) {
              explain(`Queue change could not be sent: ${error.message || error}`);
            }
          });
          board.appendChild(column);
        }
      }

      function kanbanEmptyMessage(state, phase) {
        if (state.searching) return "Searching";
        if (state.loading) return "Loading";
        if (phase === "backlog") return "No backlog items";
        return "Empty";
      }

      // SPEC-2017 US-8 — wire dragover / dragenter / dragleave / drop on
      // a Kanban column once. dragover preventDefault is required for
      // the drop event to fire; we also light up .is-drop-target as a
      // visual affordance. drop translates the column data-phase into
      // an `update_knowledge_bridge_phase` request, optimistically
      // moves the card DOM, and registers a pending entry so the card
      // shows a spinner until the response confirms.
      function wireKanbanColumnDropTarget(windowId, column) {
        column.addEventListener("dragover", (event) => {
          event.preventDefault();
          if (event.dataTransfer) {
            event.dataTransfer.dropEffect = "move";
          }
        });
        column.addEventListener("dragenter", (event) => {
          event.preventDefault();
          column.classList.add("is-drop-target");
        });
        column.addEventListener("dragleave", (event) => {
          // dragleave fires for child element transitions; only clear
          // the marker when leaving the column itself.
          if (event.target === column) {
            column.classList.remove("is-drop-target");
          }
        });
        column.addEventListener("drop", (event) => {
          event.preventDefault();
          column.classList.remove("is-drop-target");
          const raw = event.dataTransfer?.getData("text/plain");
          const issueNumber = raw ? Number.parseInt(raw, 10) : NaN;
          if (!Number.isFinite(issueNumber)) {
            return;
          }
          const state = ensureKnowledgeBridgeState(
            windowId,
            knowledgeKindForPreset(workspaceWindowById(windowId)?.preset),
          );
          const phaseKey = column.dataset.phase;
          if (!phaseKey) return;
          const targetPhase = phaseKey === "backlog" || phaseKey === "done"
            ? phaseKey === "done"
              ? "done"
              : null
            : phaseKey;
          // Optimistic UI: rewrite the entry's phase locally and
          // rerender so the card lands in the target column instantly.
          if (Array.isArray(state.entries)) {
            const index = state.entries.findIndex(
              (entry) => entry.number === issueNumber,
            );
            if (index >= 0) {
              state.entries[index] = {
                ...state.entries[index],
                phase: targetPhase,
                has_unknown_phase: false,
              };
            }
          }
          if (!state.pendingPhaseUpdates) {
            state.pendingPhaseUpdates = new Map();
          }
          state.pendingPhaseUpdates.set(
            issueNumber,
            sendUpdateKnowledgePhase(windowId, issueNumber, targetPhase),
          );
          renderKnowledgeBridge(windowId);
        });
      }

      function renderKanbanCard(windowId, state, entry) {
        const card = createNode("button", "kanban-card");
        card.type = "button";
        card.dataset.issueNumber = String(entry.number);
        const effectivePhase = effectiveKnowledgePhase(entry);
        // Plain (non-spec) Issues cannot be moved through phase columns
        // because they carry no canonical phase labels. We surface a
        // (plain) chip and disable HTML5 D&D so the user understands
        // the constraint at a glance.
        const isPlain = entry.is_spec === false;
        const isClosed = String(entry?.state || "").toLowerCase() === "closed";
        card.draggable = !isPlain && !isClosed;
        if (isPlain) {
          card.classList.add("kanban-card--plain");
        }
        if (state.selectedNumber === entry.number) {
          card.classList.add("is-selected");
          // SPEC-2356 — selected card announces aria-current="true" so
          // screen readers read which Kanban card is currently shown
          // in the detail pane (parallel to project tabs and the old
          // knowledge-row pattern).
          card.setAttribute("aria-current", "true");
        } else {
          card.removeAttribute("aria-current");
        }
        if (state.pendingPhaseUpdates && state.pendingPhaseUpdates.has(entry.number)) {
          card.classList.add("is-pending");
        }

        const head = createNode("div", "kanban-card-head");
        head.appendChild(
          createNode("span", "kanban-card-number", `#${entry.number}`),
        );
        const phaseChip = createNode(
          "span",
          `kanban-card-chip kanban-card-chip--phase-${effectivePhase}`,
          knowledgePhaseDisplayName(effectivePhase),
        );
        head.appendChild(phaseChip);
        card.appendChild(head);

        card.appendChild(
          createNode("div", "kanban-card-title", entry.title),
        );

        const meta = createNode("div", "kanban-card-meta");
        if (isPlain) {
          meta.appendChild(
            createNode("span", "kanban-card-chip kanban-card-chip--plain", "(plain)"),
          );
        }
        if (entry.has_unknown_phase) {
          meta.appendChild(
            createNode(
              "span",
              "kanban-card-chip kanban-card-chip--warning",
              "Unknown phase",
            ),
          );
        }
        if (Number.isFinite(entry.match_score)) {
          meta.appendChild(
            createNode(
              "span",
              "kanban-card-chip",
              `${entry.match_score}% match`,
            ),
          );
        }
        if ((entry.linked_branch_count || 0) > 0) {
          meta.appendChild(
            createNode(
              "span",
              "kanban-card-chip",
              `${entry.linked_branch_count} branch${entry.linked_branch_count === 1 ? "" : "es"}`,
            ),
          );
        }
        appendKnowledgeRelatedCountChips(meta, entry, "kanban-card-chip");
        if (meta.childElementCount > 0) {
          card.appendChild(meta);
        }

        card.addEventListener("click", () => {
          // The selected card stays in the split-pane detail view. We
          // always request detail (cheap; cache-backed) so selecting the
          // same card still pulls live comment / linked-branch updates.
          requestKnowledgeDetail(windowId, state.kind, entry.number);
        });

        // SPEC-2017 US-8 — D&D wire-up. Plain (is_spec=false) and closed
        // cards skip these handlers entirely (draggable=false above) so
        // they can still be clicked but never picked up.
        if (!isPlain && !isClosed) {
          card.addEventListener("dragstart", (event) => {
            // Snapshot the original entry so a failed write-back can
            // restore it; the snapshot keeps the entire entry value
            // because labels / phase / state all change on success.
            state.dndSnapshot = {
              issueNumber: entry.number,
              entry: { ...entry },
              originPhase: effectiveKnowledgePhase(entry),
            };
            card.classList.add("is-dragging");
            if (event.dataTransfer) {
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData("text/plain", String(entry.number));
            }
          });
          card.addEventListener("dragend", () => {
            card.classList.remove("is-dragging");
          });
        }
        return card;
      }

      // SPEC-3671 FR-007 / FR-008 / FR-009 / FR-010 / FR-011: the read-only live
      // mirror of the agent working on the selected Issue. Exactly one terminal is
      // mounted, and the only control it offers is Windowize.
      function renderIssueAgentPreview(windowId, state) {
        const entry = issueDetailEntry(state, state.selectedNumber);
        const work = entry ? issueWorkRowForEntry(getActiveWorkProjection?.(), entry) : null;
        const faces = entry ? issueRowFaces(windowId, entry, work) : null;
        const target = faces?.inlineWindow;
        if (!target) {
          return null;
        }
        const section = createNode("section", "issue-preview");
        section.dataset.windowId = target.id;
        section.dataset.issueNumber = String(state.selectedNumber);

        const header = createNode("div", "issue-preview-header");
        const titleWrap = createNode("div", "issue-preview-title-wrap");
        header.appendChild(createNode("span", "issue-preview-mode", "Read-only preview"));
        titleWrap.appendChild(
          createNode(
            "div",
            "issue-preview-title",
            windowDisplayTitle?.(target) || target.title || target.id,
          ),
        );
        titleWrap.appendChild(
          createNode(
            "div",
            "issue-preview-meta",
            windowRoleBadgeLabel?.(target) || target.agent_id || "Agent",
          ),
        );
        header.appendChild(titleWrap);

        const statusView = issuePreviewStatusView(target);
        const badge = createNode("span", "knowledge-monitor-chip", statusView.label);
        badge.dataset.tone = statusView.tone;
        badge.dataset.status = statusView.status;
        explainIssueControl(badge, issueStateExplanation({ key: `agent:${statusView.status}`, label: statusView.label }, entry), `preview-state-${target.id}`);
        header.appendChild(badge);

        const action = faces.canvasWindow ? "focus-canvas-window" : "windowize-issue-preview";
        const windowize = createNode("button", "wizard-button", faces.canvasWindow ? "Focus window" : "Windowize");
        windowize.type = "button";
        windowize.dataset.action = action;
        windowize.setAttribute("aria-label", faces.canvasWindow ? "Focus existing Issue window" : "Windowize agent preview");
        windowize.addEventListener("click", (event) => {
          event.preventDefault();
          event.stopPropagation();
          runIssueRowAction(action, { windowId, state, entry, target, ...faces });
        });
        header.appendChild(windowize);
        section.appendChild(header);

        const shell = createNode("div", "issue-preview-terminal");
        const terminalRoot = createNode("div", "terminal-root");
        // The mirror is read-only, but a stray mousedown must still not start a
        // window drag on the host Issue window.
        terminalRoot.addEventListener("mousedown", (event) => event.stopPropagation());
        shell.appendChild(terminalRoot);
        section.appendChild(shell);
        createTerminalRuntime?.(target.id, terminalRoot, { readOnly: true });
        return section;
      }

      // Issue #3884 AC-6 (PM ruling 2026-09-02) / SPEC #3885 T-004: the read-only
      // status row an Issue row carries for its auto-launched agent — name, last
      // activity line, elapsed time, Windowize — shown whether or not the row is
      // selected. The agent state itself is the row's single primary badge
      // (AC-5), so the status row carries no second badge. It mounts no terminal;
      // Windowize stays the only hand-off (FR-010). After Windowize the same slot
      // shows a "Shown in Agents" face (FR-012) that offers focus, never a second
      // input face for the PTY.
      function renderIssueAgentStatusRow(windowId, state, entry, faces) {
        const target = faces.inlineWindow || faces.canvasWindow;
        if (!target) {
          return null;
        }
        const onCanvas = !faces.inlineWindow;
        const row = createNode("div", "issue-agent-status");
        row.dataset.windowId = target.id;
        // Not `data-issue-number`: that attribute identifies the Issue row / card
        // itself for selection lookups, and the status row must not alias it.
        row.dataset.agentIssue = String(entry.number);
        row.setAttribute("aria-label", `Agent status for Issue #${entry.number}`);
        const meta = windowRoleBadgeLabel?.(target) || target.agent_id || "Agent";

        const titleWrap = createNode("div", "issue-agent-status-title-wrap");
        titleWrap.appendChild(
          createNode(
            "div",
            "issue-agent-status-title",
            windowDisplayTitle?.(target) || target.title || target.id,
          ),
        );
        titleWrap.appendChild(
          createNode(
            "div",
            "issue-agent-status-meta",
            onCanvas ? `${meta} · Shown in Agents` : meta,
          ),
        );
        row.appendChild(titleWrap);

        const context = { windowId, state, entry, target };
        if (onCanvas) {
          row.classList.add("is-on-canvas");
          row.appendChild(
            createNode(
              "div",
              "issue-agent-status-placeholder",
              "Shown in Agents. Input goes to the Agents tab.",
            ),
          );
          row.appendChild(issueRowActionButton("focus-canvas-window", context));
          return row;
        }

        const statusView = issuePreviewStatusView(target);
        const elapsed = createNode(
          "span",
          "issue-agent-status-elapsed",
          issueAgentElapsedLabel(target),
        );
        elapsed.title = elapsed.textContent ? `${statusView.label} for ${elapsed.textContent}` : "";
        row.appendChild(elapsed);

        const work = issueWorkRowForEntry(getActiveWorkProjection?.(), entry);
        const existingCanvas = issueRowFaces(windowId, entry, work).canvasWindow;
        row.appendChild(issueRowActionButton(
          existingCanvas ? "focus-canvas-window" : "windowize-issue-preview",
          existingCanvas ? { ...context, target: existingCanvas } : context,
        ));

        // Last in DOM order: the activity line spans the full width on its own
        // grid row, so it must follow every first-row cell (title / elapsed /
        // Windowize) or auto-placement pushes Windowize below it.
        const output = createNode(
          "div",
          "issue-agent-status-output",
          String(windowActivityDetail?.(target) || "").trim(),
        );
        output.title = output.textContent;
        row.appendChild(output);
        return row;
      }

      // SPEC #3885 FR-014 / T-018: one pair of the split view — the Issue's own
      // header above its interactive terminal. Everything except the terminal
      // comes from the row's state model, so the same agent reads identically
      // in both view modes and neither face invents its own vocabulary.
      function renderIssueSplitPair(windowId, state, entry) {
        const work = issueWorkRowForEntry(getActiveWorkProjection?.(), entry);
        const attention = work ? workAttentionFor?.(work) || null : null;
        const faces = issueRowFaces(windowId, entry, work);
        const target = faces.inlineWindow || faces.canvasWindow;
        if (!target) {
          return null;
        }
        const model = issueRowStateModel({
          entry,
          work,
          attention,
          inlineWindow: faces.inlineWindow,
          canvasWindow: faces.canvasWindow,
        });
        const context = { windowId, state, entry, work, queue: null, target, ...faces };
        const pair = createNode("div", "issue-split-pair");
        pair.setAttribute("role", "listitem");
        pair.dataset.issueNumber = String(entry.number);
        pair.dataset.windowId = target.id;
        const expanded = state.splitPairSizes.get(entry.number) === "expanded";
        pair.dataset.size = expanded ? "expanded" : "normal";
        if (state.selectedNumber === entry.number) {
          pair.classList.add("selected");
          pair.setAttribute("aria-current", "true");
        }

        const header = createNode("div", "issue-split-header");
        header.addEventListener("click", (event) => {
          if (event.target?.closest?.(".knowledge-row-actions")) return;
          requestKnowledgeDetail(windowId, state.kind, entry.number);
        });
        const titleWrap = createNode("div", "issue-split-title-wrap");
        titleWrap.appendChild(
          createNode("div", "issue-split-title", entry.title || `Issue #${entry.number}`),
        );
        titleWrap.appendChild(createNode("div", "issue-split-number", `#${entry.number}`));
        header.appendChild(titleWrap);
        const badge = createNode("span", "knowledge-row-badge", model.primary.label);
        badge.dataset.tone = model.primary.tone;
        badge.dataset.stateKey = model.primary.key;
        explainIssueControl(badge, issueStateExplanation(model.primary, entry), `split-state-${entry.number}`);
        header.appendChild(badge);
        const elapsed = createNode("span", "issue-split-elapsed", issueAgentElapsedLabel(target));
        elapsed.title = elapsed.textContent
          ? `${model.primary.label} for ${elapsed.textContent}`
          : "";
        header.appendChild(elapsed);

        const actions = createNode("div", "knowledge-row-actions");
        actions.setAttribute("role", "group");
        actions.setAttribute("aria-label", `Issue #${entry.number} actions`);
        // In the split view the terminal hand-off (Windowize / Focus) belongs to
        // the pair itself, so it is shown rather than moved to a status row.
        for (const action of model.actions) {
          actions.appendChild(issueRowActionButton(action, context));
        }
        actions.appendChild(renderIssueSplitSizeToggle(windowId, state, entry, expanded));
        if (model.overflow.length > 0) {
          actions.appendChild(renderIssueRowMenu(model.overflow, context));
        }
        header.appendChild(actions);
        pair.appendChild(header);

        const output = createNode(
          "div",
          "issue-split-output",
          String(windowActivityDetail?.(target) || "").trim(),
        );
        output.title = output.textContent;
        pair.appendChild(output);

        if (!faces.inlineWindow) {
          // FR-003a / US-4: the agent is on the canvas, so this face is a status
          // face only — a second terminal would double the input path.
          pair.classList.add("is-on-canvas");
          pair.appendChild(
            createNode(
              "div",
              "issue-split-placeholder",
              "Shown in Agents. Input goes to the Agents tab.",
            ),
          );
          return pair;
        }

        const shell = createNode("div", "issue-split-terminal");
        const terminalRoot = createNode("div", "terminal-root");
        // A stray mousedown inside the terminal must not start a window drag on
        // the host Issue window.
        terminalRoot.addEventListener("mousedown", (event) => event.stopPropagation());
        shell.appendChild(terminalRoot);
        pair.appendChild(shell);
        // FR-003: the split view is one of the two faces that may take input, so
        // the shared runtime is reparented here interactive, not mirrored.
        createTerminalRuntime?.(target.id, terminalRoot, { readOnly: false });
        return pair;
      }

      // SPEC #3885 T-005: a pair grows and shrinks in place. The size lives in
      // the surface state, so a data refresh keeps it and the terminal runtime
      // is only reparented, never rebuilt.
      function renderIssueSplitSizeToggle(windowId, state, entry, expanded) {
        const label = expanded ? "Shrink" : "Expand";
        const button = createNode(
          "button",
          "wizard-button is-compact knowledge-row-action",
          label,
        );
        button.type = "button";
        button.dataset.action = "toggle-pair-size";
        button.setAttribute("aria-expanded", expanded ? "true" : "false");
        button.setAttribute("aria-label", `${label} the agent pane for Issue #${entry.number}`);
        button.addEventListener("click", (event) => {
          event.preventDefault();
          event.stopPropagation();
          if (expanded) {
            state.splitPairSizes.delete(entry.number);
          } else {
            state.splitPairSizes.set(entry.number, "expanded");
          }
          renderKnowledgeBridge(windowId);
        });
        return button;
      }

      // SPEC #3885 T-020: one elapsed-time source for both faces of an agent.
      function issueAgentElapsedLabel(target) {
        const elapsed = issueAgentElapsedMs(target, windowRuntimeStateSince?.(target?.id));
        return elapsed === null ? "" : formatAgentElapsed(elapsed);
      }

      // SPEC #3885 Phase 5 (T-035 / FR-023): the Issue detail pane, top to
      // bottom: number + branch, title, state pill + PR / labels, one action
      // band, the AC gauge and checklist, the agent preview, the body folded to
      // its headings (only the first, the summary, open), Related work. The
      // pill reuses the row primary badge so both read the same word.
      function issueDetailEntry(state, number) {
        for (const list of [state.entries, state.baseEntries]) {
          const found = (Array.isArray(list) ? list : []).find(
            (entry) => entry?.number === number,
          );
          if (found) return found;
        }
        return filteredIssueEntries(state).find(entry => entry.number === number) || null;
      }

      function renderIssueDetailAcceptance(detail) {
        const text = (detail.sections || []).map((section) => section?.body || "").join("\n");
        const progress = issueAcceptanceProgress(text);
        if (progress.total === 0) return null;
        const block = createNode("section", "issue-detail-ac");
        block.setAttribute("aria-label", "Acceptance criteria");
        const head = createNode("div", "issue-detail-ac-head");
        head.appendChild(createNode("span", "issue-detail-ac-title", "Acceptance criteria"));
        head.appendChild(
          createNode("span", "issue-detail-ac-count", `${progress.done} / ${progress.total}`),
        );
        block.appendChild(head);
        const gauge = createNode("div", "issue-detail-ac-gauge");
        gauge.setAttribute("role", "progressbar");
        gauge.setAttribute("aria-label", "Acceptance criteria done");
        gauge.setAttribute("aria-valuemin", "0");
        gauge.setAttribute("aria-valuemax", String(progress.total));
        gauge.setAttribute("aria-valuenow", String(progress.done));
        const fill = createNode("span", "issue-detail-ac-fill");
        fill.style.width = `${Math.round((progress.done / progress.total) * 100)}%`;
        gauge.appendChild(fill);
        block.appendChild(gauge);
        const list = createNode("ul", "issue-detail-ac-list");
        for (const item of progress.items) {
          const row = createNode("li", "issue-detail-ac-item");
          row.dataset.done = item.done ? "true" : "false";
          row.appendChild(createNode("span", "issue-detail-ac-mark", item.done ? "✓" : "○"));
          row.appendChild(createNode("span", "issue-detail-ac-id", item.id));
          row.appendChild(createNode("span", "issue-detail-ac-text", item.text));
          list.appendChild(row);
        }
        block.appendChild(list);
        return block;
      }

      function renderIssueDetailActions(context) {
        const model = issueDetailActionModel(context);
        const band = createNode("div", "knowledge-detail-actions issue-detail-actions");
        band.setAttribute("role", "group");
        band.setAttribute("aria-label", `Issue #${context.entry.number} actions`);
        band.dataset.phase = model.phase;
        for (const action of model.actions) {
          if (action === "open-pr") {
            const link = createNode("a", "wizard-button is-compact primary", "Open PR");
            link.dataset.action = "open-pr";
            link.href = context.work.pr_url;
            link.target = "_blank";
            link.rel = "noopener noreferrer";
            band.appendChild(link);
            continue;
          }
          band.appendChild(issueRowActionButton(action, context));
        }
        if (model.overflow.length > 0) {
          band.appendChild(renderIssueRowMenu(model.overflow, context));
        }
        return band;
      }

      function syncIssuePreviewVisibility(element, state) {
        const root = element?.querySelector(".issue-bridge-root");
        if (!root) return;
        const hidden = state.viewMode !== "split" && state.previewHidden;
        root.dataset.previewHidden = String(hidden);
        const toggle = root.querySelector('[data-action="toggle-issue-preview"]');
        if (toggle) {
          toggle.hidden = hidden || state.viewMode === "split";
          toggle.textContent = "Close preview";
          toggle.setAttribute("aria-expanded", String(!hidden));
        }
      }

      function closeIssuePreview(windowId, state) {
        state.previewHidden = true;
        renderKnowledgeBridge(windowId);
        for (const host of windowMap.get(windowId)?.querySelectorAll(".issue-queue-items") || []) {
          keyedKnowledgeLists.get(host)?.reveal(state.selectedNumber);
        }
        windowMap.get(windowId)?.querySelector(`.knowledge-row[data-issue-number="${state.selectedNumber}"] .knowledge-row-select`)?.focus();
      }

      function focusIssuePreview(windowId, element) {
        const detail = element.querySelector(".knowledge-detail-pane");
        const explanations = issueControlExplanations.get(windowId);
        if (explanations) explanations.restoreFocus(detail, false);
        else detail?.focus();
      }

      function renderIssueDetailPane(windowId, state, detailPane, { agentPreview = true } = {}) {
        // #5396: list/status refreshes and duplicate detail replies must retain
        // the native disclosure, scroll and focus state of an unchanged preview.
        const number = state.detail?.number ?? state.selectedNumber;
        const cachedEntry = issueDetailEntry(state, number);
        const entry = cachedEntry || state.detail ? queueProjectedEntry(cachedEntry || {
          number: Number(number), title: state.detail.title,
          state: state.detail.state, labels: state.detail.labels || [],
        }) : null;
        const work = entry ? issueWorkRowForEntry(getActiveWorkProjection?.(), entry) : null;
        const attention = work ? workAttentionFor?.(work) || null : null;
        const faces = entry ? issueRowFaces(windowId, entry, work) : {};
        const queue = canonicalQueuedKnowledgeEntries(state);
        const relatedWorks = state.detail?.related_works || [];
        const liveSessions = buildKnowledgeRelatedLiveSessionWindowsByConversation(relatedWorks);
        const relatedActions = relatedWorks.flatMap(relatedWork =>
          (relatedWork.agents || []).flatMap(agent =>
            (agent.sessions || []).map(session => [
              isKnowledgeRelatedResumePending(agent.session_id),
              findKnowledgeRelatedLiveWindow(agent, session, liveSessions)?.id || null,
            ])));
        const key = JSON.stringify([
          state.detail, state.detailLoading, state.selectedNumber, state.previewHidden,
          state.issueDetailView, agentPreview, state.error, entry, work, attention,
          relatedActions,
          agentPreview && state.issueDetailView === "output" && Boolean(isAgentsSurfaceMounted()),
          queue.findIndex(item => item.number === entry?.number), queue.length,
          [faces.inlineWindow, faces.canvasWindow].map(target => target ? [
            target.id, target.status, windowDisplayTitle?.(target) || target.title,
            windowRoleBadgeLabel?.(target) || target.agent_id,
          ] : null),
        ]);
        if (issueDetailRenderKeys.get(detailPane) === key && !state.renderError) return;
        renderIssueDetailPaneContents(windowId, state, detailPane, { agentPreview });
        issueDetailRenderKeys.set(detailPane, key);
      }

      function renderIssueDetailPaneContents(windowId, state, detailPane, { agentPreview = true } = {}) {
        detailPane.innerHTML = "";
        detailPane.hidden = agentPreview && state.previewHidden === true;
        if (detailPane.hidden) return;
        const tabs = createNode("div", "knowledge-state-filter modal-header issue-detail-tabs");
        tabs.setAttribute("role", "group");
        tabs.setAttribute("aria-label", "Issue preview content");
        for (const [view, label] of [["issue", "Issue"], ["output", "Output"]]) {
          const tab = createNode("button", "", label);
          tab.type = "button";
          tab.dataset.issueDetailView = view;
          tab.setAttribute("aria-pressed", String(state.issueDetailView === view));
          tab.classList.toggle("is-active", state.issueDetailView === view);
          tab.addEventListener("click", () => {
            state.issueDetailView = view;
            renderKnowledgeDetailPane(windowId, state, detailPane, { agentPreview });
          });
          tabs.appendChild(tab);
        }
        if (agentPreview) {
          const close = createNode("button", "wizard-button is-compact issue-preview-close", "×");
          close.type = "button";
          close.dataset.action = "close-issue-preview";
          close.setAttribute("aria-label", "Close preview");
          explainIssueControl(close, "Close the selected issue preview and return to the full-width board.", "close-preview");
          close.addEventListener("click", () => closeIssuePreview(windowId, state));
          tabs.appendChild(close);
          detailPane.appendChild(tabs);
        }
        if (agentPreview && state.issueDetailView === "output") {
          const preview = agentPreview && state.selectedNumber != null
            ? renderIssueAgentPreview(windowId, state) : null;
          if (preview) detailPane.appendChild(preview);
          else {
            const entry = issueDetailEntry(state, state.selectedNumber);
            const work = entry ? issueWorkRowForEntry(getActiveWorkProjection?.(), entry) : null;
            const canvasWindow = entry ? issueRowFaces(windowId, entry, work).canvasWindow : null;
            if (canvasWindow) {
              detailPane.appendChild(createNode("div", "knowledge-detail-empty issue-preview-empty",
                "Shown in Agents. Use Focus window to view the agent output."));
              detailPane.appendChild(issueRowActionButton("focus-canvas-window", {
                windowId, state, entry, work, canvasWindow, target: canvasWindow,
              }));
              return;
            }
            const column = entry ? issueQueueColumnForEntry(queueProjectedEntry(entry), windowId) : null;
            const reason = state.selectedNumber == null ? "Select an Issue to view its output."
              : !agentPreview ? "Agent output is shown in the split view."
              : column === "queued" ? "Waiting in queue. No agent has started."
              : column === "done" ? "This issue is completed. No agent is running."
              : "No agent is running for this issue.";
            detailPane.appendChild(createNode("div", "knowledge-detail-empty issue-preview-empty", reason));
          }
          return;
        }
        const detail = state.detail;
        if (!detail) {
          const empty = createNode(
            "div",
            "knowledge-detail-empty",
            state.detailLoading ? "Loading detail" : "Select an Issue",
          );
          if (!state.detailLoading) {
            empty.appendChild(
              createNode(
                "div",
                "issue-detail-hint",
                "Select a card to see its acceptance criteria and next action.",
              ),
            );
          }
          detailPane.appendChild(empty);
          return;
        }
        const number = Number(detail.number ?? state.selectedNumber);
        const entry = queueProjectedEntry(issueDetailEntry(state, number) || {
          number,
          title: detail.title,
          state: detail.state,
          labels: detail.labels || [],
        });
        const work = issueWorkRowForEntry(getActiveWorkProjection?.(), entry);
        const attention = work ? workAttentionFor?.(work) || null : null;
        const faces = issueRowFaces(windowId, entry, work);
        const queued = canonicalQueuedKnowledgeEntries(state);
        const queueIndex = queued.findIndex((queuedEntry) => queuedEntry.number === entry.number);
        const queue = queueIndex >= 0 ? { index: queueIndex, length: queued.length } : null;
        const context = {
          windowId,
          state,
          entry,
          work,
          attention,
          queue,
          inlineWindow: faces.inlineWindow,
          canvasWindow: faces.canvasWindow,
          target: faces.inlineWindow || faces.canvasWindow,
        };
        const row = issueRowStateModel(context);

        const header = createNode("div", "knowledge-detail-header issue-detail-header");
        const id = createNode("div", "issue-detail-id");
        // SPEC #3170 FR-101: `.knowledge-detail-subtitle` stays the visible
        // identity of the selection the frame-mismatch probes read.
        id.appendChild(
          createNode("span", "knowledge-detail-subtitle issue-detail-number", `#${number}`),
        );
        if (work?.branch) {
          id.appendChild(createNode("span", "issue-detail-branch", work.branch));
        }
        header.appendChild(id);
        header.appendChild(createNode("h3", "knowledge-detail-title issue-detail-title", detail.title));

        const status = createNode("div", "issue-detail-status");
        const pill = createNode("span", "knowledge-row-badge", row.primary.label);
        pill.dataset.tone = row.primary.tone;
        pill.dataset.stateKey = row.primary.key;
        explainIssueControl(pill, issueStateExplanation(row.primary, entry), `detail-state-${number}`);
        status.appendChild(pill);
        if (work?.pr_number) {
          const prState = String(work.pr_state || "").trim();
          const pr = createNode(
            "span",
            "knowledge-row-secondary-item",
            prState ? `PR #${work.pr_number} · ${prState}` : `PR #${work.pr_number}`,
          );
          pr.dataset.key = "pr";
          status.appendChild(pr);
        }
        for (const label of visibleKnowledgeLabels(detail.labels || [])) {
          status.appendChild(createNode("span", "knowledge-chip", label));
        }
        header.appendChild(status);
        if (entry.queued_by) {
          const source = entry.queued_by === "auto-refill" ? "Auto-refill"
            : entry.queued_by === "urgent" ? "Urgent label" : "Operator";
          header.appendChild(createNode("div", "issue-detail-provenance", `Queued by: ${source}`));
          explainIssueControl(header.lastElementChild, issueQueueSourceExplanation(entry), `provenance-${number}`);
        }
        if (queue) {
          header.appendChild(createNode("div", "issue-detail-priority", `Priority: ${issueQueuePriorityLabel(entry)}`));
          header.appendChild(createNode("div", "issue-detail-priority-assignment", `Priority assigned by: ${entry.assigned_by || "Unknown"} · Assigned at: ${entry.assigned_at || "Not observed"}`));
        }
        header.appendChild(renderIssueDetailActions(context));
        detailPane.appendChild(header);

        const acceptance = renderIssueDetailAcceptance(detail);
        if (acceptance) detailPane.appendChild(acceptance);

        const scroll = createNode("div", "knowledge-detail-scroll workspace-scroll modal-body issue-detail-body");
        if (state.detailLoading) {
          scroll.appendChild(createNode("div", "knowledge-detail-empty", "Loading detail"));
        }
        (detail.sections || []).forEach((section, index) => {
          const card = createNode("details", "knowledge-section");
          card.open = index === 0;
          card.appendChild(createNode("summary", "knowledge-section-title", section.title));
          card.appendChild(renderUiContent(document, section.content, "knowledge-section-body"));
          scroll.appendChild(card);
        });
        const relatedWorks = renderKnowledgeRelatedWorks(detail);
        if (relatedWorks) {
          scroll.appendChild(relatedWorks);
        }
        if (scroll.childElementCount === 0) {
          scroll.appendChild(
            createNode("div", "knowledge-detail-empty", "No cached detail available"),
          );
        }
        detailPane.appendChild(scroll);
      }

      function renderKnowledgeDetailPane(windowId, state, detailPane, { agentPreview = true } = {}) {
        if (state.kind === "issue") {
          issueWindowViewModel.update(() => ({ windowId, detailOnly: true }));
          return;
        }
        detailPane.innerHTML = "";
        // In split mode the agent already has an interactive face in its pair;
        // a second, read-only one would show the same PTY twice.
        const preview = agentPreview ? renderIssueAgentPreview(windowId, state) : null;
        if (preview) {
          detailPane.appendChild(preview);
        }
        const detail = state.detail;
        if (!detail) {
          detailPane.appendChild(
            createNode(
              "div",
              "knowledge-detail-empty",
              state.detailLoading ? "Loading detail" : "Select a cached item",
            ),
          );
          return;
        }

        const header = createNode("div", "knowledge-detail-header");
        const head = createNode("div", "");
        const headRow = createNode("div", "knowledge-detail-head");
        headRow.appendChild(createNode("h3", "knowledge-detail-title", detail.title));
        const detailChip = knowledgeDetailChip(detail, state.kind);
        headRow.appendChild(
          createNode(
            "span",
            `knowledge-state-chip ${detailChip.className}`,
            detailChip.label,
          ),
        );
        head.appendChild(headRow);
        if (detail.subtitle) {
          head.appendChild(
            createNode("div", "knowledge-detail-subtitle", detail.subtitle),
          );
        }
        const displayLabels = visibleKnowledgeLabels(detail.labels || []);
        const stalePhase = state.kind === "issue" ? "" : staleKnowledgePhaseWarning(detail);
        if (displayLabels.length > 0 || stalePhase) {
          const labelRow = createNode("div", "knowledge-label-row");
          for (const label of displayLabels) {
            labelRow.appendChild(createNode("span", "knowledge-chip", label));
          }
          if (stalePhase) {
            labelRow.appendChild(
              createNode("span", "kanban-card-chip kanban-card-chip--warning", stalePhase),
            );
          }
          head.appendChild(labelRow);
        }
        header.appendChild(head);

        const actions = createNode("div", "knowledge-detail-actions");
        if (detail.launch_issue_number !== null && detail.launch_issue_number !== undefined) {
          const launchButton = createNode("button", "wizard-button primary", "Launch Agent");
          launchButton.type = "button";
          launchButton.addEventListener("click", () =>
            openIssueLaunchWizard(windowId, detail.launch_issue_number),
          );
          actions.appendChild(launchButton);
        }
        if (actions.childElementCount > 0) {
          header.appendChild(actions);
        }
        detailPane.appendChild(header);

        const scroll = createNode("div", "knowledge-detail-scroll workspace-scroll");
        if (state.detailLoading) {
          scroll.appendChild(
            createNode("div", "knowledge-detail-empty", "Loading detail"),
          );
        }
        for (const section of detail.sections || []) {
          const card = createNode("section", "knowledge-section");
          card.appendChild(
            createNode("div", "knowledge-section-title", section.title),
          );
          card.appendChild(renderUiContent(document, section.content, "knowledge-section-body"));
          scroll.appendChild(card);
        }
        const relatedWorks = renderKnowledgeRelatedWorks(detail);
        if (relatedWorks) {
          scroll.appendChild(relatedWorks);
        }
        if (scroll.childElementCount === 0) {
          scroll.appendChild(
            createNode("div", "knowledge-detail-empty", "No cached detail available"),
          );
        }
        detailPane.appendChild(scroll);
      }

      function renderKnowledgeDetailOnly(windowId, state, { focusPreview = false } = {}) {
        if (!windowMap.get(windowId)?.querySelector(".knowledge-detail-pane")) return;
        if (state.kind === "issue") {
          issueWindowViewModel.update(() => ({ windowId, detailOnly: true, focusPreview }));
          return;
        }
        knowledgeBoardViewModel.update(() => ({ windowId, detailOnly: true }));
      }

      function renderKnowledgeDetailView(windowId, state) {
        const element = windowMap.get(windowId);
        const detailPane = element?.querySelector(".knowledge-detail-pane");
        if (!detailPane) {
          return;
        }
        renderKnowledgeDetailPane(windowId, state, detailPane, { agentPreview: state.viewMode !== "split" });
      }

      function renderKnowledgeSelection(windowId, state, previousNumber, { focusPreview = false } = {}) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const updateNode = (number, selected) => {
          if (number === null || number === undefined) {
            return;
          }
          for (const node of element.querySelectorAll(
            `[data-issue-number="${Number(number)}"]`,
          )) {
            node.classList.toggle(
              "selected",
              selected && node.classList.contains("knowledge-row"),
            );
            node.classList.toggle(
              "is-selected",
              selected && node.classList.contains("kanban-card"),
            );
            const currentTarget = node.classList.contains("knowledge-row")
              ? node.querySelector(".knowledge-row-select")
              : node;
            if (selected) {
              currentTarget?.setAttribute("aria-current", "true");
            } else {
              currentTarget?.removeAttribute("aria-current");
            }
          }
        };
        updateNode(previousNumber, false);
        updateNode(state.selectedNumber, true);
        renderKnowledgeStatusOnly(windowId, state);
        renderKnowledgeDetailOnly(windowId, state, { focusPreview });
      }

      function formatRefreshedTime(ms) {
        const date = new Date(ms);
        return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
      }

      function renderKnowledgeStatusOnly(windowId, state) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const issueSurface = isSilentSemanticKind(state.kind);
        if (issueSurface) {
          // FR-017: Issue window errors go to the notification center, never
          // a persistent red band.
          syncIssueWindowErrorReport(windowId, state);
        }
        const view = knowledgeStatusView(state, issueSurface);
        // SPEC #3885 Phase 5 (T-032 / FR-021): the Issue window has no status
        // row; the cached count, the refreshed time and any loading state are
        // read from the ↻ tooltip.
        const refresh = element.querySelector("[data-action='refresh-knowledge']");
        if (issueSurface && refresh) {
          const cached = Array.isArray(state.baseEntries) ? state.baseEntries.length : 0;
          const parts = ["Refresh cached work items", `${cached} cached`];
          if (state.refreshedAt) {
            parts.push(`Refreshed ${formatRefreshedTime(state.refreshedAt)}`);
          }
          if (view) parts.push(view.text);
          refresh.title = parts.join(" · ");
        }
        const status = element.querySelector(".knowledge-status");
        if (!status) {
          return;
        }
        status.className = "knowledge-status";
        status.textContent = "";
        if (view) {
          status.classList.add("visible", view.tone);
          status.textContent = view.text;
        }
      }

      function knowledgeStatusView(state, issueSurface) {
        if (state.error) {
          // Issue windows keep the status line empty on error (a failed load
          // is not "no items"); other kinds keep their red band.
          return issueSurface ? null : { tone: "error", text: state.error };
        }
        if (!issueSurface && state.searching) {
          return { tone: "info", text: "Searching semantic index" };
        }
        if (state.loading && state.loadSlow) {
          return {
            tone: "info",
            text: `${issueSurface ? "Loading cache-backed work items" : "Loading cache-backed data"} (taking longer than usual)`,
          };
        }
        if (state.loading && state.entries.length > 0) {
          return {
            tone: "info",
            text: state.refreshing
              ? issueSurface
                ? "Refreshing cached work items"
                : "Refreshing cached knowledge"
              : issueSurface
                ? "Loading cache-backed work items"
                : "Loading cache-backed data",
          };
        }
        if (state.loading && state.entries.length === 0) {
          return {
            tone: "info",
            text: issueSurface ? "Loading cache-backed work items" : "Loading cache-backed data",
          };
        }
        if (state.entries.length === 0 && !state.searching) {
          return {
            tone: "info",
            text: state.emptyMessage || (issueSurface ? "No cached work items" : "No cached items"),
          };
        }
        return null;
      }

      function canonicalQueuedKnowledgeEntries(state) {
        if (Array.isArray(issueMonitorModel.read().status.terminal_queue)) return issueMonitorModel.read().status.terminal_queue;
        const source = Array.isArray(state.baseEntries) && state.baseEntries.length > 0
          ? state.baseEntries
          : state.entries;
        return (Array.isArray(source) ? source : []).map(queueProjectedEntry)
          .filter(
            (entry) =>
              isIssueInTerminalQueue(entry),
          )
          .slice()
          .sort(
            (left, right) =>
              Number(left.queue_position) - Number(right.queue_position) ||
              Number(left.number) - Number(right.number),
          );
      }

      function moveQueuedKnowledgeEntry(windowId, state, issueNumber, direction) {
        const queued = canonicalQueuedKnowledgeEntries(state);
        const index = queued.findIndex((entry) => entry.number === issueNumber);
        const targetIndex = index + direction;
        if (index < 0 || targetIndex < 0 || targetIndex >= queued.length) return;
        send({ kind: "issue_monitor_queue_move", issue_number: issueNumber, position: targetIndex });
      }

      function moveQueuedKnowledgeEntryToTop(windowId, state, issueNumber) {
        const queued = canonicalQueuedKnowledgeEntries(state);
        const index = queued.findIndex((entry) => entry.number === issueNumber);
        if (index <= 0) return;
        send({ kind: "issue_monitor_queue_move", issue_number: issueNumber, position: 0 });
      }

      // SPEC #3885 T-004 (FR-006): every Issue action the row can offer, keyed by
      // the `data-action` the tests and the Playwright specs address.
      const ISSUE_ROW_ACTION_VIEWS = Object.freeze({
        "launch-now": Object.freeze({ label: "Launch now", aria: "Launch now" }),
        "configure-issue": Object.freeze({
          label: "Settings",
          aria: "Project Agent settings for",
        }),
        "queue-push": Object.freeze({
          label: "Add to queue",
          aria: "Add to queue",
        }),
        "queue-remove": Object.freeze({
          label: "Remove from queue",
          aria: "Remove from queue",
        }),
        "move-up": Object.freeze({ label: "↑ Move up", aria: "Move up" }),
        "move-down": Object.freeze({ label: "↓ Move down", aria: "Move down" }),
        "continue-work": Object.freeze({ label: "Continue work", aria: "Continue work on" }),
        "resume-work": Object.freeze({ label: "Resume", aria: "Resume work on" }),
        "cleanup-work": Object.freeze({ label: "Clean Up", aria: "Clean up work for" }),
        "launch-agent": Object.freeze({ label: "Launch agent", aria: "Launch an agent for" }),
        "windowize-issue-preview": Object.freeze({
          label: "Windowize",
          aria: "Open the agent terminal in an Agents tab for",
        }),
        "focus-canvas-window": Object.freeze({
          label: "Focus window",
          aria: "Focus the Agents tab for",
        }),
        // SPEC #3885 FR-015: the only place an agent can be stopped from.
        "stop-agent": Object.freeze({
          label: "Stop agent",
          aria: "Stop the agent for",
        }),
        // Issue #3628 (AC-3): release the failure hold without launching.
        "requeue-issue": Object.freeze({
          label: "Return to queue",
          aria: "Return to the queue",
        }),
        // SPEC #3885 Phase 5 (T-035 / FR-023): the detail pane's action band.
        "open-window": Object.freeze({
          label: "Open window",
          aria: "Open the agent window for",
        }),
        "move-to-top": Object.freeze({
          label: "Move to top",
          aria: "Move to the top of the queue",
        }),
      });
      // Actions rendered inside the agent status row rather than the row's
      // action group.
      const ISSUE_ROW_TERMINAL_ACTIONS = new Set([
        "windowize-issue-preview",
        "focus-canvas-window",
      ]);
      // Ids this surface Windowized, so the row keeps its "Shown in Agents" face
      // even before the Work projection reports the agent's window id.
      const windowizedAgentWindowIds = new Set();

      function runIssueRowAction(action, { windowId, state, entry, work, target, inlineWindow, canvasWindow }) {
        if (action === "focus-canvas-window" && canvasWindow) target = canvasWindow;
        switch (action) {
          case "launch-now":
            send({
              kind: "issue_monitor_launch_now",
              issue_number: entry.number,
              linked_issue_kind: entry.is_spec ? "spec" : "issue",
            });
            return;
          case "configure-issue":
            send({
              kind: "issue_monitor_configure_issue",
              issue_number: entry.number,
              linked_issue_kind: entry.is_spec ? "spec" : "issue",
            });
            return;
          case "queue-push":
            send({
              kind: "issue_monitor_queue_push",
              issue_numbers: [entry.number],
            });
            return;
          case "queue-remove":
            send({
              kind: "issue_monitor_queue_remove",
              issue_numbers: [entry.number],
            });
            return;
          case "move-up":
            moveQueuedKnowledgeEntry(windowId, state, entry.number, -1);
            return;
          case "move-down":
            moveQueuedKnowledgeEntry(windowId, state, entry.number, 1);
            return;
          case "move-to-top":
            moveQueuedKnowledgeEntryToTop(windowId, state, entry.number);
            return;
          // The detail pane's single "Open window": Windowize an inline
          // preview, or focus the agent that is already on the canvas.
          case "open-window":
            runIssueRowAction(
              !canvasWindow && inlineWindow ? "windowize-issue-preview" : "focus-canvas-window",
              { windowId, state, entry, work, target, canvasWindow },
            );
            return;
          case "continue-work":
            continueWork?.(work.id, getResumeBounds?.());
            return;
          case "resume-work":
            openWorkspaceResumePicker?.(work.id);
            return;
          case "cleanup-work":
            if (work?.cleanup_candidate) {
              openWorkspaceCleanup?.(work.cleanup_candidate, windowId);
            }
            return;
          case "launch-agent":
            openIssueLaunchWizard(windowId, entry.number);
            return;
          case "windowize-issue-preview":
            if (target?.id) {
              windowizedAgentWindowIds.add(target.id);
              windowizeIssuePreviewWindow?.(target.id);
            }
            return;
          case "focus-canvas-window":
            if (target?.id) {
              focusWindowLocally(target.id);
              sendWindowFocus(target.id);
            }
            return;
          case "stop-agent":
            if (target?.id) {
              send({ kind: "stop_window", id: target.id });
            }
            return;
          // Issue #3628 (AC-3): identity-free by design — the rows this exists
          // for have no launch left to name. The driver refuses any row a live
          // launch still owns, so the button cannot kill a running agent.
          case "requeue-issue":
            send({
              kind: "issue_monitor_requeue",
              issue_number: entry.number,
            });
            return;
          default:
            return;
        }
      }

      function issueRowActionButton(action, context, { menuItem = false } = {}) {
        const view = ISSUE_ROW_ACTION_VIEWS[action] || { label: action, aria: action };
        const button = createNode(
          "button",
          menuItem ? "knowledge-row-menu-item" : "wizard-button is-compact knowledge-row-action",
          view.label,
        );
        button.type = "button";
        button.dataset.action = action;
        button.setAttribute("aria-label", `${view.aria} Issue #${context.entry.number}`);
        if (menuItem) {
          button.setAttribute("role", "menuitem");
        }
        const { work, queue } = context;
        if ((action === "move-up" || action === "move-to-top") && queue) {
          button.disabled = queue.index <= 0;
        } else if (action === "move-down" && queue) {
          button.disabled = queue.index >= queue.length - 1;
        } else if (action === "cleanup-work" && !work?.cleanup_candidate) {
          // The backend owns cleanup eligibility (live agent / live process).
          // The row must never infer it from merged state alone.
          button.disabled = true;
          button.dataset.blockedReason = work?.cleanup_blocked_reason || "";
          button.title = `Cleanup unavailable: ${work?.cleanup_blocked_reason || ""}`;
        }
        button.addEventListener("click", (event) => {
          event.preventDefault();
          event.stopPropagation();
          if (button.disabled) return;
          const menu = menuItem ? button.closest(".knowledge-row-menu") : null;
          if (menu) {
            menu.open = false;
            menu.removeAttribute("open");
          }
          runIssueRowAction(action, context);
        });
        return button;
      }

      function renderIssueRowMenu(overflow, context) {
        const menu = createNode("details", "knowledge-row-menu");
        const trigger = createNode("summary", "icon-button knowledge-row-menu-trigger", "⋯");
        trigger.setAttribute("aria-label", `More actions for Issue #${context.entry.number}`);
        trigger.title = `More actions for Issue #${context.entry.number}`;
        menu.appendChild(trigger);
        const list = createNode("div", "knowledge-row-menu-list");
        list.setAttribute("role", "menu");
        for (const action of overflow) {
          list.appendChild(issueRowActionButton(action, context, { menuItem: true }));
        }
        menu.appendChild(list);
        return menu;
      }

      function issueProjectWindows(windowId) {
        const catalog = windowCatalogModel?.read().catalog;
        const windows = (typeof getWorkspaceWindows === "function" ? getWorkspaceWindows() : [])
          .map(windowData => Object.hasOwn(catalog?.runtimeStates || {}, windowData.id)
            ? { ...windowData, status: catalog.runtimeStates[windowData.id] } : windowData);
        const tabs = catalog?.appState?.tabs;
        if (!tabs) return windows;
        const project = tabs.find(tab => tab.workspace?.windows?.some(window => window.id === windowId));
        const ids = new Set((project?.workspace?.windows || []).map(window => window.id));
        return windows.filter(window => ids.has(window.id));
      }

      function issueRowFaces(windowId, entry, work) {
        const windows = issueProjectWindows(windowId);
        const inline = issuePreviewWindowsForIssue(windows, windowId, entry.number);
        const canvas = issueCanvasAgentWindowsForIssue(windows, work, windowizedAgentWindowIds, entry.number);
        const isLive = candidate => ISSUE_ROW_STOPPABLE_AGENT_STATUSES.has(issuePreviewStatusView(candidate).status);
        const liveCanvas = canvas.find(isLive);
        if (liveCanvas) return { inlineWindow: null, canvasWindow: liveCanvas };
        const inlineWindow = inline.find(isLive) || null;
        if (inlineWindow) return { inlineWindow, canvasWindow: canvas[0] || null };
        if (canvas[0]) return { inlineWindow: null, canvasWindow: canvas[0] };
        return { inlineWindow: inline[0] || null, canvasWindow: null };
      }

      function renderIssueRow(windowId, state, entry) {
        const row = createNode("div", "knowledge-row");
        row.dataset.issueNumber = String(entry.number);
        row.setAttribute("role", "listitem");
        const select = createNode("button", "knowledge-row-select");
        select.type = "button";
        if (state.selectedNumber === entry.number) {
          row.classList.add("selected");
          select.setAttribute("aria-current", "true");
        }

        // SPEC-3671 FR-012: the Work row joined from the already-broadcast projection;
        // SPEC #3885 T-004: everything the row shows derives from one state model.
        const work = issueWorkRowForEntry(getActiveWorkProjection?.(), entry);
        const attention = work ? workAttentionFor?.(work) || null : null;
        const faces = issueRowFaces(windowId, entry, work);
        const queued = canonicalQueuedKnowledgeEntries(state);
        const queueIndex = queued.findIndex((queuedEntry) => queuedEntry.number === entry.number);
        const queue = queueIndex >= 0 ? { index: queueIndex, length: queued.length } : null;
        const model = issueRowStateModel({
          entry,
          work,
          attention,
          inlineWindow: faces.inlineWindow,
          canvasWindow: faces.canvasWindow,
          queue,
        });
        const context = {
          windowId,
          state,
          entry,
          work,
          queue,
          target: faces.inlineWindow || faces.canvasWindow,
          ...faces,
        };

        const main = createNode("div", "knowledge-row-main");
        const titleWrap = createNode("div", "");
        titleWrap.appendChild(
          createNode("div", "knowledge-row-title", entry.title || `Issue #${entry.number}`),
        );
        titleWrap.appendChild(
          createNode("div", "knowledge-row-number", `#${entry.number}`),
        );
        main.appendChild(titleWrap);
        const badge = createNode("span", "knowledge-row-badge", model.primary.label);
        badge.dataset.tone = model.primary.tone;
        badge.dataset.stateKey = model.primary.key;
        explainIssueControl(badge, issueStateExplanation(model.primary, entry), `row-state-${entry.number}`, false);
        main.appendChild(badge);
        select.appendChild(main);

        const tierChips = issueTierChips(entry.number);
        const secondaryItems = [...model.secondary, ...tierChips];
        if (secondaryItems.length > 0) {
          const secondary = createNode("div", "knowledge-row-secondary");
          for (const item of secondaryItems) {
            const node = createNode("span", `knowledge-row-secondary-item${item.kind === "tier" ? " knowledge-monitor-chip" : ""}`, item.label);
            node.dataset.kind = item.kind;
            node.dataset.key = item.key;
            if (item.title) {
              node.title = item.title;
            }
            if (item.key === "queue") {
              explainIssueControl(node, `Position ${entry.queue_position} in the launch queue. ${issueQueueSourceExplanation(entry)}`, `row-queue-${entry.number}`, false);
            } else if (item.kind === "tier") {
              explainIssueControl(node, item.explanation, `${item.key}-${entry.number}`, false);
            }
            secondary.appendChild(node);
          }
          select.appendChild(secondary);
        }
        explainIssueControl(select,
          [issueStateExplanation(model.primary, entry), Number.isFinite(entry.queue_position) ? issueQueueSourceExplanation(entry) : "",
            ...tierChips.map(item => item.explanation)].filter(Boolean).join("\n"),
          `issue-${entry.number}`);

        row.addEventListener("click", (event) => {
          if (event.target?.closest?.(".knowledge-row-actions")) return;
          // Issue #3884: neither is the agent status row (its Windowize button).
          if (event.target?.closest?.(".issue-agent-status")) return;
          requestKnowledgeDetail(windowId, state.kind, entry.number);
        });
        row.appendChild(select);

        const actions = createNode("div", "knowledge-row-actions");
        actions.setAttribute("role", "group");
        actions.setAttribute("aria-label", `Issue #${entry.number} actions`);
        for (const action of model.actions) {
          if (ISSUE_ROW_TERMINAL_ACTIONS.has(action)) continue;
          actions.appendChild(issueRowActionButton(action, context));
        }
        if (model.overflow.length > 0) {
          actions.appendChild(renderIssueRowMenu(model.overflow, context));
        }
        if (actions.childElementCount > 0) {
          row.appendChild(actions);
        }
        // Issue #3884 AC-6: the agent's read-only status row (or its "Shown on
        // canvas" face), shown whether or not the row is selected, outside the
        // select button.
        const agentStatus = renderIssueAgentStatusRow(windowId, state, entry, faces);
        if (agentStatus) {
          row.appendChild(agentStatus);
        }
        return row;
      }

      function renderIssueKnowledgeBridge(windowId, element, state) {
        const focus = issueExplanationFocus(windowId);
        const list = element.querySelector(".knowledge-list");
        const detailPane = element.querySelector(".knowledge-detail-pane");
        const refreshButton = element.querySelector("[data-action='refresh-knowledge']");
        const searchInput = element.querySelector(".knowledge-search");
        if (!list || !detailPane || !refreshButton || !searchInput) {
          return;
        }
        // A queue transition changes the owning column. Preserve the logical
        // control across both lists, including a destination outside the viewport.
        const focused = list.contains(document.activeElement) ? document.activeElement : null;
        const focusedNumber = Number(focused?.closest("[data-issue-number]")?.dataset.issueNumber);
        const focusedAction = focused?.dataset.action;
        const focusedSummary = focused?.matches("summary");

        refreshButton.disabled =
          !state.refreshEnabled || (state.loading && !knowledgeEntriesAreEmpty(state));
        searchInput.placeholder = knowledgeSearchPlaceholder(state.kind);
        for (const button of element.querySelectorAll("[data-issue-filter]")) {
          const selected = button.dataset.issueFilter === state.issueStateFilter;
          button.classList.toggle("is-active", selected);
          button.setAttribute("aria-pressed", selected ? "true" : "false");
        }
        // SPEC #3885 FR-014 / AC-14: list is the default face; split is the one
        // that takes input. The mode is an attribute on the root so the
        // stylesheet, not a second render path, lays the two out.
        const splitMode = state.viewMode === "split";
        const root = element.querySelector(".issue-bridge-root");
        if (root) {
          root.dataset.viewMode = splitMode ? "split" : "list";
          syncIssuePreviewVisibility(element, state);
        }
        for (const button of element.querySelectorAll("[data-issue-view]")) {
          const selected = button.dataset.issueView === (splitMode ? "split" : "list");
          button.classList.toggle("is-active", selected);
          button.setAttribute("aria-pressed", selected ? "true" : "false");
        }

        renderKnowledgeStatusOnly(windowId, state);

        // Issue menus are recreated; retain disclosure state by Issue identity.
        const openIssueMenus = new Set(
          Array.from(list.querySelectorAll(".knowledge-row-menu[open]"),
            menu => menu.closest(".knowledge-row, .issue-split-pair")?.dataset.issueNumber),
        );
        // Keep the native disclosure connected while cache projections refresh;
        // replacing it between pointerdown and pointerup loses the user's click.
        const other = list.querySelector(".issue-other-group");
        for (const child of Array.from(list.childNodes)) {
          if (child === other || (!splitMode && (child.classList?.contains("issue-queue-board") || child.classList?.contains("issue-queue-feedback")))) continue;
          for (const host of child.querySelectorAll?.(".issue-queue-items") || []) keyedKnowledgeLists.get(host)?.dispose();
          child.remove();
        }
        const visibleEntries = filteredIssueEntries(state);
        renderIssueMonitorControls(element, windowId);
        if (state.error) {
          list.insertBefore(createIssueNoticeBadge("Issue load failed", issueWindowErrorKey(windowId), state.error), other);
        } else if (state.loading && state.loadSlow && knowledgeEntriesAreEmpty(state)) {
          const progress = createNode("div", "knowledge-empty", knowledgeStatusView(state, true).text);
          progress.setAttribute("role", "status");
          list.insertBefore(progress, other);
        }
        if (splitMode) {
          const pairs = visibleEntries
            .map((entry) => renderIssueSplitPair(windowId, state, entry))
            .filter(Boolean);
          if (pairs.length === 0) {
            list.insertBefore(
              createNode("div", "knowledge-empty", "No running agents to show side by side"), other,
            );
          } else {
            for (const pair of pairs) {
              list.insertBefore(pair, other);
            }
          }
        } else {
          renderIssueQueueBoard(windowId, state, list, visibleEntries);
        }
        for (const menu of list.querySelectorAll(".knowledge-row-menu")) {
          if (openIssueMenus.has(menu.closest(".knowledge-row, .issue-split-pair")?.dataset.issueNumber)) {
            menu.setAttribute("open", "");
          }
        }
        renderOtherWork(list, windowId, { laneFilter: state.issueLaneFilter || "all" });
        renderIssueDetailPane(windowId, state, detailPane, { agentPreview: !splitMode });
        if (Number.isFinite(focusedNumber) && !focused.isConnected) {
          let row = list.querySelector(`.knowledge-row[data-issue-number="${focusedNumber}"]`);
          if (!row) {
            for (const host of list.querySelectorAll(".issue-queue-items")) {
              if (keyedKnowledgeLists.get(host)?.reveal(focusedNumber)) break;
            }
            row = list.querySelector(`.knowledge-row[data-issue-number="${focusedNumber}"]`);
          }
          const target = focusedAction
            ? [...(row?.querySelectorAll("[data-action]") || [])].find(node => node.dataset.action === focusedAction)
            : focusedSummary ? row?.querySelector("summary") : null;
          (target || row?.querySelector(".knowledge-row-select"))?.focus({ preventScroll: true });
        }
        publishIssueControlExplanations(windowId, focus);
      }

      function renderKnowledgeBridge(windowId) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const state = ensureKnowledgeBridgeState(
          windowId,
          knowledgeKindForPreset(workspaceWindowById(windowId)?.preset),
        );
        // An error changes the list presentation outside the data revision.
        // The next successful completion must rebuild it to remove that state.
        if (state.error) {
          state.entriesRevision = null;
          state.entriesContent = null;
        }
        if (state.kind === "issue") {
          issueWindowViewModel.update(() => ({ windowId }));
          return;
        }
        knowledgeBoardViewModel.update(() => ({ windowId }));
      }

      function renderKnowledgeBoardView(windowId, element, state) {
        const board = element.querySelector(".kanban-board");
        const detailPane = element.querySelector(".knowledge-detail-pane");
        const status = element.querySelector(".knowledge-status");
        const refreshButton = element.querySelector("[data-action='refresh-knowledge']");
        const searchInput = element.querySelector(".knowledge-search");
        const hideDoneToggle = element.querySelector("[data-action='kanban-hide-done']");
        if (!board || !detailPane || !status || !refreshButton || !searchInput) {
          return;
        }
        const focusedCard = board.contains(document.activeElement)
          ? document.activeElement.closest(".kanban-card[data-issue-number]") : null;
        const focusedNumber = Number(focusedCard?.dataset.issueNumber);

        refreshButton.disabled =
          !state.refreshEnabled || (state.loading && !knowledgeEntriesAreEmpty(state));
        searchInput.placeholder = knowledgeSearchPlaceholder(state.kind);
        if (hideDoneToggle) {
          hideDoneToggle.checked = state.hideDone === true;
        }
        board.dataset.hideDone = state.hideDone === true ? "true" : "false";

        renderKnowledgeStatusOnly(windowId, state);

        // SPEC-2017 — Kanban grouping. Each entry routes to a single
        // column: closed Issues land in "done" regardless of phase
        // label so the Done column unifies state="closed" with the
        // phase/done open Issues; otherwise we trust entry.phase, with
        // null falling back to "backlog" so plain Issues and unlabeled
        // SPECs are never lost. Unknown phase labels stay in their
        // backend-extracted column but flag has_unknown_phase so the
        // card can warn the user about malformed metadata.
        const visibleEntries = state.query.trim()
          ? state.entries
          : filteredKnowledgeEntries(state);
        const columnsByPhase = new Map();
        for (const column of board.querySelectorAll(".kanban-column[data-phase]")) {
          columnsByPhase.set(column.dataset.phase, column);
          if (column.dataset.kanbanWired !== "true") {
            wireKanbanColumnDropTarget(windowId, column);
            column.dataset.kanbanWired = "true";
          }
        }
        for (const [phase, column] of columnsByPhase) {
          const items = visibleEntries.filter(entry => {
            const key = effectiveKnowledgePhase(entry);
            return key === phase || (phase === "backlog" && !columnsByPhase.has(key));
          });
          const countLabel = column.querySelector("[data-role='count']");
          if (countLabel && countLabel.textContent !== String(items.length)) {
            countLabel.textContent = String(items.length);
          }
          const body = column.querySelector("[data-role='body']");
          if (body) reconcileKnowledgeList(body, items, {
            scrollRoot: body, virtualize: visibleEntries.length > 200,
            version: entry => JSON.stringify([entry, state.pendingPhaseUpdates?.has(entry.number)]),
            update: (card, entry) => {
              card.classList.toggle("is-selected", state.selectedNumber === entry.number);
              if (state.selectedNumber === entry.number) {
                if (card.getAttribute("aria-current") !== "true") card.setAttribute("aria-current", "true");
              } else if (card.hasAttribute("aria-current")) card.removeAttribute("aria-current");
            },
            render: entry => renderKanbanCard(windowId, state, entry),
          });
          if (body && body.childElementCount === 0) {
            const empty = createNode(
              "div",
              "kanban-column-empty",
              kanbanEmptyMessage(state, phase),
            );
            body.appendChild(empty);
          }
        }

        renderKnowledgeDetailPane(windowId, state, detailPane);
        if (focusedCard && !focusedCard.isConnected) {
          let card = board.querySelector(`.kanban-card[data-issue-number="${focusedNumber}"]`);
          if (!card) {
            for (const body of board.querySelectorAll(".kanban-column-body")) {
              if (keyedKnowledgeLists.get(body)?.reveal(focusedNumber)) break;
            }
            card = board.querySelector(`.kanban-card[data-issue-number="${focusedNumber}"]`);
          }
          card?.focus({ preventScroll: true });
        }
      }

      function renderAllKnowledgeBridgeWindows() {
        for (const windowId of knowledgeBridgeStateMap.keys()) {
          renderKnowledgeBridge(windowId);
        }
      }
      // SPEC-3064 Phase 3 (E6d): Knowledge window mount moved verbatim from
      // app.js mountWindowBody (surface === "knowledge" branch).
      function mountKnowledgeWindow(windowData, body) {
          const knowledgeKind = knowledgeKindForPreset(windowData.preset);
          if (knowledgeKind !== "issue" && knowledgeKind !== "spec") return;
          // SPEC-2017 — Knowledge Bridge surface is a 6-column Kanban Board:
          // Backlog / Draft / Planning / Implementation / Review / Done.
          // The columns are hard-coded so the source carries every
          // canonical data-phase literal (asserted by kanban-structure
          // tests) and so the renderer can simply locate columns via
          // .kanban-column[data-phase="..."]. The right-hand detail
          // pane survives Phase 1 unchanged; SPEC-2017 Phase 3 replaces
          // it with the SPEC-2356 Drawer pattern.
          body.innerHTML = `
            <div class="knowledge-root kanban-root">
              <div class="workspace-toolbar kanban-toolbar is-stacked">
                <div class="workspace-toolbar-main">
                  <div class="knowledge-heading">${knowledgeHeading(knowledgeKind)}</div>
                  <input class="knowledge-search" type="search" placeholder="${knowledgeSearchPlaceholder(knowledgeKind)}" />
                  <label class="kanban-hide-done-toggle" for="kanban-hide-done-${windowData.id}">
                    <input
                      type="checkbox"
                      id="kanban-hide-done-${windowData.id}"
                      class="kanban-hide-done"
                      data-action="kanban-hide-done"
                    />
                    <span>Hide done</span>
                  </label>
                </div>
                <div class="workspace-toolbar-actions">
                  <button class="icon-button" data-action="refresh-knowledge" aria-label="Refresh cached knowledge">↻</button>
                </div>
              </div>
              <div class="knowledge-status"></div>
              <div class="knowledge-split workspace-split kanban-shell">
                <div class="knowledge-list-pane kanban-list-pane">
                  <div class="kanban-board" role="list" aria-label="Knowledge Bridge Kanban Board">
                    <div class="kanban-column" data-phase="backlog" aria-label="Backlog column">
                      <div class="kanban-column-header">
                        <span class="kanban-column-name">Backlog</span>
                        <span class="kanban-column-count" data-role="count">0</span>
                      </div>
                      <div class="kanban-column-body" data-role="body"></div>
                    </div>
                    <div class="kanban-column" data-phase="draft" aria-label="Draft column">
                      <div class="kanban-column-header">
                        <span class="kanban-column-name">Draft</span>
                        <span class="kanban-column-count" data-role="count">0</span>
                      </div>
                      <div class="kanban-column-body" data-role="body"></div>
                    </div>
                    <div class="kanban-column" data-phase="planning" aria-label="Planning column">
                      <div class="kanban-column-header">
                        <span class="kanban-column-name">Planning</span>
                        <span class="kanban-column-count" data-role="count">0</span>
                      </div>
                      <div class="kanban-column-body" data-role="body"></div>
                    </div>
                    <div class="kanban-column" data-phase="implementation" aria-label="Implementation column">
                      <div class="kanban-column-header">
                        <span class="kanban-column-name">Implementation</span>
                        <span class="kanban-column-count" data-role="count">0</span>
                      </div>
                      <div class="kanban-column-body" data-role="body"></div>
                    </div>
                    <div class="kanban-column" data-phase="review" aria-label="Review column">
                      <div class="kanban-column-header">
                        <span class="kanban-column-name">Review</span>
                        <span class="kanban-column-count" data-role="count">0</span>
                      </div>
                      <div class="kanban-column-body" data-role="body"></div>
                    </div>
                    <div class="kanban-column" data-phase="done" aria-label="Done column">
                      <div class="kanban-column-header">
                        <span class="kanban-column-name">Done</span>
                        <span class="kanban-column-count" data-role="count">0</span>
                      </div>
                      <div class="kanban-column-body" data-role="body"></div>
                    </div>
                  </div>
                </div>
                <div class="knowledge-detail-pane"></div>
              </div>
            </div>
          `;
          if (knowledgeKind === "issue") {
            body.innerHTML = `
              <div class="knowledge-root issue-bridge-root">
                <div class="workspace-toolbar kanban-toolbar is-stacked">
                  <div class="workspace-toolbar-main">
                    <div class="knowledge-heading">${knowledgeHeading(knowledgeKind)}</div>
                    <input class="knowledge-search" type="search" placeholder="${knowledgeSearchPlaceholder(knowledgeKind)}" />
                    <div class="knowledge-state-filter knowledge-view-mode" role="group" aria-label="Issue view mode">
                      <button type="button" data-issue-view="list">Kanban</button>
                      <button type="button" data-issue-view="split">Split</button>
                    </div>

                  </div>
                  <div class="workspace-toolbar-actions">
                    <button type="button" class="wizard-button is-compact" data-action="toggle-issue-preview" aria-expanded="false" hidden>Close preview</button>
                    <button type="button" class="wizard-button is-compact" data-action="issue-new" aria-haspopup="dialog">＋ New</button>
                    <button class="wizard-button is-compact" data-action="refresh-knowledge" aria-label="Refresh cached work items" title="Refresh cached work items">↻ Refresh</button>
                  </div>
                </div>
                <section class="knowledge-monitor-bar" aria-label="Issues summary">
                  <div class="knowledge-monitor-status" role="group" aria-label="Monitor status">
                  <button type="button" class="knowledge-row-badge knowledge-monitor-pill" data-tone="idle" aria-live="polite" aria-disabled="true">Stopped</button>
                  <span class="knowledge-monitor-metric" data-metric="active">Active 0/1</span>
                  <span class="knowledge-monitor-metric" data-metric="queue">Queue 0</span>
                  </div><div class="knowledge-monitor-controls" role="group" aria-label="Monitor controls">
                  <button type="button" class="knowledge-monitor-switch" role="switch" aria-checked="false" aria-label="Autonomous mode" data-action="monitor-autonomous" data-enabled="false">
                    <span class="knowledge-monitor-switch__label">Autonomous</span>
                    <span class="knowledge-monitor-switch__track" aria-hidden="true"><span class="knowledge-monitor-switch__knob"></span></span>
                    <span class="knowledge-monitor-switch__state">Off</span>
                  </button>
                  <button type="button" class="wizard-button is-compact primary" data-action="monitor-toggle">Start</button>
                  <button type="button" class="wizard-button is-compact" data-action="monitor-settings" aria-label="Issues settings">⚙</button>
                  </div>
                  <button type="button" class="knowledge-row-badge knowledge-notification-badge knowledge-monitor-capacity-badge" aria-haspopup="dialog" hidden></button>
                </section>
                <div class="knowledge-split workspace-split issue-list-shell">
                  <div class="knowledge-list-pane">
                    <div class="knowledge-list" role="list" aria-label="Cached work items"></div>
                  </div>
                  <div class="knowledge-detail-pane modal-shell issue-preview-shell" role="dialog" aria-modal="false" aria-label="Issue preview" tabindex="-1"></div>
                </div>
              </div>
            `;
          }
          body.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            sendWindowFocus(windowData.id);
          });
          const state = ensureKnowledgeBridgeState(
            windowData.id,
            knowledgeKind,
          );
          if (!state.monitorSubscriptions) {
            // Register views only at mount; receive-side state creation is data-only.
            // Initial rendering follows registration so a failed mount can retry.
            let mounted = false;
            state.monitorSubscriptions = [
              issueMonitorModel.subscribe(model => model, () => {
                if (mounted && state.kind === "issue") renderIssueMonitorControls(windowMap.get(windowData.id), windowData.id);
              }),
              issueMonitorModel.subscribeFrame(model => model, () => {
                if (mounted) renderKnowledgeBridge(windowData.id);
              }),
            ];
            if (windowCatalogModel && state.kind === "issue") {
              state.monitorSubscriptions.push(windowCatalogModel.subscribeFrame(model =>
                JSON.stringify(model.catalog.appState.tabs.flatMap(tab => tab.workspace?.windows || [])
                  .map(windowData => [windowData.id, windowData.linked_issue_number,
                    windowData.placement?.kind === "issue_preview" ? windowData.placement.issue_number : null,
                    Object.hasOwn(model.catalog.runtimeStates, windowData.id)
                      ? model.catalog.runtimeStates[windowData.id] : windowData.status])), () => {
                if (mounted) renderKnowledgeBridge(windowData.id);
              }));
            }
            mounted = true;
          }
          const laneFilter = body.querySelector("[data-issue-lane-filter]");
          if (laneFilter) {
            for (const option of laneFilter.options) option.selected = option.value === state.issueLaneFilter;
            laneFilter.addEventListener("change", () => {
              state.issueLaneFilter = laneFilter.value;
              renderKnowledgeBridge(windowData.id);
            });
          }
          const pendingIndexTarget = pendingIndexOpenTargetsByPreset.get(windowData.preset);
          if (
            pendingIndexTarget
            && pendingIndexTarget.knowledgeKind === knowledgeKind
          ) {
            state.selectedNumber = pendingIndexTarget.number;
            if (knowledgeKind === "issue") state.previewHidden = false;
            pendingIndexOpenTargetsByPreset.delete(windowData.preset);
          }
          if (knowledgeKind === "issue") {
            bindIssueControlExplanations(windowData.id, body.querySelector(".issue-bridge-root"));
            for (const [selector, text] of [
              ['[data-action="monitor-autonomous"]', "Allow eligible issues to run without waiting for human approval."],
              ['[data-action="monitor-toggle"]', "Start or pause monitoring for queued issues."],
              ['[data-issue-view="list"]', "Show issues by their execution queue state with a read-only output preview."],
              ['[data-issue-view="split"]', "Pair issues with interactive terminal views."],
            ]) explainIssueControl(body.querySelector(selector), text);
            publishIssueControlExplanations(windowData.id);
          }
          const search = body.querySelector(".knowledge-search");
          search.value = state.query;
          search.addEventListener("input", () => {
            state.query = search.value;
            scheduleKnowledgeSearch(
              windowData.id,
              knowledgeKind,
            );
          });
          body
            .querySelector("[data-action='refresh-knowledge']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              requestKnowledgeBridge(
                windowData.id,
                knowledgeKind,
                true,
              );
              renderKnowledgeBridge(
                windowData.id,
              );
            });
          for (const filterButton of body.querySelectorAll("[data-issue-filter]")) {
            filterButton.addEventListener("click", (event) => {
              event.stopPropagation();
              state.issueStateFilter = filterButton.dataset.issueFilter || "open";
              renderKnowledgeBridge(
                windowData.id,
              );
            });
          }
          // SPEC #3885 T-018: switching the view mode re-renders the same state;
          // the terminal runtimes are reparented by the render, so the PTY, the
          // scrollback and the selection are never rebuilt.
          for (const viewButton of body.querySelectorAll("[data-issue-view]")) {
            viewButton.addEventListener("click", (event) => {
              event.stopPropagation();
              state.viewMode = viewButton.dataset.issueView === "split" ? "split" : "list";
              renderKnowledgeBridge(
                windowData.id,
              );
            });
          }
          if (knowledgeKind === "issue") {
            body.querySelector('[data-action="toggle-issue-preview"]')?.addEventListener("click", () => {
              closeIssuePreview(windowData.id, state);
            });
            body.querySelector(".knowledge-detail-pane")?.addEventListener("keydown", event => {
              if (event.key === "Escape" && !state.previewHidden && state.viewMode !== "split") {
                event.stopPropagation();
                closeIssuePreview(windowData.id, state);
              }
            });
            wireIssueMonitorControls(body, windowData.id);
          }
          // SPEC-2017 — Hide done toggle persists via localStorage so
          // reloads honour the user preference. The hidden state hides
          // the Done column entirely (CSS-driven via data-hide-done on
          // the board) and updates state in place without reloading.
          const hideDoneToggle = body.querySelector("[data-action='kanban-hide-done']");
          if (hideDoneToggle) {
            hideDoneToggle.checked = state.hideDone === true;
            hideDoneToggle.addEventListener("change", (event) => {
              event.stopPropagation();
              state.hideDone = hideDoneToggle.checked === true;
              writeKanbanHideDonePreference(
                state.hideDone,
              );
              renderKnowledgeBridge(
                windowData.id,
              );
            });
          }
          if (!state.loading && (!state.detail || knowledgeEntriesAreEmpty(state))) {
            requestKnowledgeBridge(
              windowData.id,
              knowledgeKind,
              false,
            );
          }
          ensureKnowledgeAutoRefresh(windowData.id, knowledgeKind);
          renderKnowledgeBridge(
            windowData.id,
          );
          return;
      }

      // SPEC-3064 Phase 3 (E6d): receive() bodies for knowledge_* events
      // moved verbatim from app.js; the case arms in app.js delegate here.
      function applyKnowledgeReceiveEvent(event) {
        switch (event.kind) {
          case "knowledge_revision": {
            applyKnowledgeCacheRevision(event);
            break;
          }
          case "terminal_preview": {
            terminalPreviewText.set(event.id, event.text);
            for (const element of windowMap.values()) {
              for (const output of element.querySelectorAll(".issue-card-output")) {
                if (output.dataset.windowId === event.id) {
                  output.querySelector("pre").textContent = event.text;
                }
              }
            }
            break;
          }
          case "knowledge_entries": {
            const state = knowledgeBridgeStateMap.get(event.id);
            if (
              !state ||
              normalizeKnowledgeKind(event.knowledge_kind) !==
                normalizeKnowledgeKind(state.kind)
            ) {
              break;
            }
            if (event.request_id && !state.ownedLoadRequestIds.has(event.request_id)) {
              break;
            }
            // Issue #3297: a response that lost the race against the 5s
            // recovery timer carries a superseded request_id, but while the
            // window still has no data it is strictly better than the empty
            // view — apply it; a newer in-flight response overwrites it.
            if (
              event.request_id &&
              event.request_id !== state.loadRequestId &&
              !knowledgeEntriesAreEmpty(state)
            ) {
              break;
            }
            const queuedQuery = state.query.trim();
            const previousNumber = state.selectedNumber;
            const incomingEntries = event.entries || [];
            const previousSelectedEntry = state.baseEntries.find(entry => entry.number === previousNumber);
            const incomingSelectedEntry = incomingEntries.find(entry => entry.number === previousNumber);
            const relatedSnapshot = entry => JSON.stringify([
              entry.related_work_refs || [], entry.related_work_count || 0,
              entry.related_session_count || 0,
            ]);
            const refreshSelectedDetail = state.selectionGeneration > 0 &&
              previousSelectedEntry && incomingSelectedEntry &&
              relatedSnapshot(previousSelectedEntry) !== relatedSnapshot(incomingSelectedEntry);
            const entriesContent = JSON.stringify(incomingEntries);
            const unchangedEntries = !state.error && !state.renderError &&
              ((typeof event.revision === "string" && event.revision.length > 0 &&
                event.revision === state.entriesRevision) || entriesContent === state.entriesContent);
            // Legacy senders without revisions also preserve identical rows.
            state.entriesContent = entriesContent;
            state.entriesRevision = event.revision || null;
            const keepSelectedNumber =
              state.selectedNumber &&
              incomingEntries.some((entry) => entry.number === state.selectedNumber);
            state.baseEntries = incomingEntries;
            state.cacheRevision = event.cache_revision ?? null;
            if (state.autoRefreshChanged && state.cacheRevision !== null &&
                state.cacheRevision === state.autoRefreshRevision) {
              state.autoRefreshChanged = false;
              state.autoRefreshDirty = false;
            }
            state.baseEmptyMessage = event.empty_message || "";
            if (!queuedQuery) {
              state.entries = state.baseEntries.slice();
              state.emptyMessage = state.baseEmptyMessage;
              state.searching = false;
            }
            // FR-101: an initial-load / list completion may refresh rows
            // but must never move an explicit selection.
            if (
              state.selectionGeneration > 0 ||
              (event.request_id === state.loadRequestId &&
                state.loadSelectionGeneration !== state.selectionGeneration)
            ) {
              // keep state.selectedNumber untouched
            } else {
              state.selectedNumber = keepSelectedNumber
                ? state.selectedNumber
                : event.selected_number ?? null;
            }
            state.refreshEnabled = Boolean(event.refresh_enabled);
            state.refreshedAt = Date.now();
            state.error = "";
            // A cache-first list can precede the related-work snapshot used by
            // an explicit selection. Refresh its detail without selecting anew.
            if (refreshSelectedDetail) {
              knowledgeCompletionViewModel.update(() => ({
                windowId: event.id, detailRefreshNumber: previousNumber,
              }));
            }
            const renderCompletion = () => {
              knowledgeCompletionViewModel.update(() => ({
                windowId: event.id, unchangedEntries, previousNumber,
              }));
            };
            if (finishKnowledgeLoad(state, event.id, event.knowledge_kind)) {
              renderCompletion();
              break;
            }
            if (queuedQuery) {
              if (unchangedEntries) {
                if (!state.inFlightSearchIntent && state.pendingSearchTimer === null) {
                  queueKnowledgeSearch(event.id, state,
                    updateKnowledgeSearchIntent(state, event.knowledge_kind, queuedQuery));
                }
                renderCompletion();
                break;
              }
              scheduleKnowledgeSearch(
                event.id,
                event.knowledge_kind,
              );
              break;
            }
            renderCompletion();
            break;
          }
          case "knowledge_search_results": {
            const state = knowledgeBridgeStateMap.get(event.id);
            if (!state) {
              break;
            }
            const activeIntent = state.inFlightSearchIntent;
            if (!activeIntent || event.request_id !== activeIntent.requestId) {
              break;
            }
            state.inFlightSearchIntent = null;
            state.searchInFlight = false;
            state.inFlightSearchRequestId = 0;
            const responseMatchesIntent =
              normalizeKnowledgeKind(event.knowledge_kind) === activeIntent.kind &&
              String(event.query || "").trim() === activeIntent.query &&
              knowledgeSearchIntentIsCurrent(state, activeIntent);
            if (!responseMatchesIntent) {
              dispatchLatestKnowledgeSearchIntent(event.id, state);
              break;
            }
            state.queuedSearchIntent = null;
            state.queuedSearchQuery = "";

            state.entries = event.entries || [];
            const selectionIsCurrent =
              activeIntent.selectionGeneration === state.selectionGeneration;
            if (selectionIsCurrent && state.selectionGeneration === 0) {
              state.selectedNumber = event.selected_number ?? null;
            }
            state.emptyMessage = event.empty_message || "";
            state.refreshEnabled = Boolean(event.refresh_enabled);
            state.error = "";
            state.searching = false;
            const directive = event.semantic_retry;
            const transientDirective =
              isSilentSemanticKind(activeIntent.kind) &&
              isKnowledgeSemanticRetryDirective(directive);
            if (transientDirective) {
              state.semanticRetryTyped = true;
              scheduleKnowledgeSemanticRetry(
                event.id,
                activeIntent.kind,
                state,
              );
            } else if (isSilentSemanticKind(activeIntent.kind)) {
              invalidateKnowledgeSemanticRetry(state);
            }
            if (selectionIsCurrent && state.selectionGeneration === 0) {
              state.detail = null;
            }
            renderKnowledgeBridge(event.id);
            break;
          }
          case "knowledge_detail": {
            const state = knowledgeBridgeStateMap.get(event.id);
            if (
              !state ||
              normalizeKnowledgeKind(event.knowledge_kind) !==
                normalizeKnowledgeKind(state.kind)
            ) {
              break;
            }
            if (!knowledgeDetailRequestMatches(state, event)) {
              break;
            }
            const previousNumber = state.selectedNumber;
            const matchesLoadRequest =
              !event.request_id || event.request_id === state.loadRequestId;
            state.detail = event.detail ? { ...event.detail,
              sections: (event.detail.sections || []).map(section => ({ ...section,
                content: { type: "markdown", body: section.body || "", body_html: section.body_html || "" } })) } : null;
            state.selectedNumber = event.detail?.number ?? state.selectedNumber ?? null;
            if (matchesLoadRequest) {
              finishKnowledgeLoad(state, event.id, event.knowledge_kind);
            }
            state.detailLoading = false;
            renderKnowledgeSelection(event.id, state, previousNumber);
            // SPEC-2017 US-9 — refresh the Drawer body when the detail
            // is for the entry the Drawer is currently showing. This
            // also handles the swap-on-different-card case (T-034):
            // requestKnowledgeDetail was just dispatched for the new
            // number, so the new detail will arrive here and overwrite
            // the body without re-mounting the Drawer.
            const drawer = document.getElementById("kanban-drawer");
            if (
              drawer &&
              drawer.dataset.open === "true" &&
              kanbanDrawerActiveContext &&
              kanbanDrawerActiveContext.windowId === event.id
            ) {
              kanbanDrawerActiveContext = {
                ...kanbanDrawerActiveContext,
                number: event.detail?.number ?? kanbanDrawerActiveContext.number,
              };
              knowledgeBoardViewModel.update(() => ({ windowId: event.id, drawer: true }));
            }
            break;
          }
          // SPEC-3064 Phase 3 (E6b): branch cleanup state and rendering
          // live in the branches cleanup surface.
          case "branch_cleanup_result":
          case "branch_cleanup_progress":
          case "branch_error":
            applyBranchCleanupReceiveEvent(event);
            break;
          case "knowledge_bridge_phase_updated": {
            // SPEC-2017 US-8 — phase write-back response. On Ok we
            // overwrite the optimistic card with fresh_entry and clear
            // the pending marker so the spinner stops; on Error we
            // rollback from dndSnapshot and surface a toast.
            const state = knowledgeBridgeStateMap.get(event.id);
            if (!state) {
              break;
            }
            if (state.pendingPhaseUpdates) {
              state.pendingPhaseUpdates.delete(event.issue_number);
            }
            if (event.result?.kind === "ok") {
              const fresh = event.result.fresh_entry;
              if (fresh) {
                replaceKnowledgeEntry(state.entries, fresh);
                replaceKnowledgeEntry(state.baseEntries, fresh);
              }
              state.dndSnapshot = null;
            } else {
              const message =
                event.result?.message || "Failed to update phase. Reverting.";
              if (
                state.dndSnapshot &&
                state.dndSnapshot.issueNumber === event.issue_number &&
                Array.isArray(state.entries)
              ) {
                const index = state.entries.findIndex(
                  (entry) => entry.number === event.issue_number,
                );
                if (index >= 0 && state.dndSnapshot.entry) {
                  // Restore the card data captured at dragstart so the
                  // labels / phase / state mirror the pre-drop reality.
                  state.entries[index] = state.dndSnapshot.entry;
                }
                state.dndSnapshot = null;
              }
              state.error = message;
            }
            renderKnowledgeBridge(event.id);
            break;
          }
          case "knowledge_error": {
            const state = knowledgeBridgeStateMap.get(event.id);
            if (!state) {
              break;
            }
            const isSearchError =
              typeof event.request_id === "number" && typeof event.query === "string";
            if (isSearchError) {
              const activeIntent = state.inFlightSearchIntent;
              if (!activeIntent || event.request_id !== activeIntent.requestId) {
                break;
              }
              state.inFlightSearchIntent = null;
              state.searchInFlight = false;
              state.inFlightSearchRequestId = 0;
              const responseMatchesIntent =
                normalizeKnowledgeKind(event.knowledge_kind) === activeIntent.kind &&
                event.query.trim() === activeIntent.query &&
                knowledgeSearchIntentIsCurrent(state, activeIntent);
              if (!responseMatchesIntent) {
                dispatchLatestKnowledgeSearchIntent(event.id, state);
                break;
              }
              state.queuedSearchIntent = null;
              state.queuedSearchQuery = "";
              state.searching = false;
              invalidateKnowledgeSemanticRetry(state);
              if (
                isSilentSemanticKind(activeIntent.kind) &&
                event.error_domain !== "non_semantic"
              ) {
                // Legacy/untyped semantic failures are deliberately silent
                // and never start the typed indefinite retry ladder.
                renderKnowledgeStatusOnly(event.id, state);
              } else {
                state.error = event.message;
                renderKnowledgeBridge(event.id);
              }
              break;
            }

            if (
              normalizeKnowledgeKind(event.knowledge_kind) !==
                normalizeKnowledgeKind(state.kind)
            ) {
              break;
            }
            const matchesLoadRequest = event.request_id === state.loadRequestId;
            const matchesDetailRequest =
              event.request_id === state.detailRequestId &&
              state.detailRequestSelectionGeneration === state.selectionGeneration &&
              state.detailRequestNumber === state.selectedNumber;
            const matchesInitialIdless =
              !event.request_id && state.selectionGeneration === 0;
            if (!matchesLoadRequest && !matchesDetailRequest && !matchesInitialIdless) {
              break;
            }
            if (
              matchesLoadRequest &&
              state.loadSelectionGeneration !== state.selectionGeneration
            ) {
              finishKnowledgeLoad(state, event.id, event.knowledge_kind);
              break;
            }
            const startedQueuedRefresh = matchesLoadRequest
              ? finishKnowledgeLoad(state, event.id, event.knowledge_kind)
              : false;
            if (matchesLoadRequest) {
              state.error = startedQueuedRefresh ? "" : event.message;
            } else {
              state.error = event.message;
            }
            state.searching = false;
            state.detailLoading = false;
            if (matchesLoadRequest) {
              renderKnowledgeBridge(event.id);
            } else {
              renderKnowledgeStatusOnly(event.id, state);
              renderKnowledgeDetailOnly(event.id, state);
            }
            break;
          }
          default:
            break;
        }
      }

      // SPEC #3885 FR-011: the Windowized agent lives on the canvas, but its header
      // still shows the Issue. This is the one place that answers "what do we know
      // about Issue #N right now" so the canvas never re-derives Issue state.
      function issueContextForNumber(issueNumber) {
        const number = Number(issueNumber);
        if (!Number.isFinite(number)) return null;
        let entry = null;
        for (const state of knowledgeBridgeStateMap.values()) {
          const lists = [state?.entries, state?.baseEntries];
          for (const list of lists) {
            if (!Array.isArray(list)) continue;
            const found = list.find((candidate) => Number(candidate?.number) === number);
            if (found) {
              entry = found;
              break;
            }
          }
          if (entry) break;
        }
        const work = entry ? issueWorkRowForEntry(getActiveWorkProjection?.(), entry) : null;
        return { entry, work, attention: work ? workAttentionFor?.(work) || null : null };
      }

      issueMonitorModel.subscribe(model => model.status, syncIssueMonitorErrorReport);
      knowledgeEventRefreshModel.subscribe(model => model, ({ kind, forceReload }) => {
        if (kind === "projection") refreshKnowledgeWorkSnapshot(forceReload);
        if (kind === "monitor") {
          for (const state of knowledgeBridgeStateMap.values()) {
            if (normalizeKnowledgeKind(state.kind) === "issue") state.eventRefreshDirty = true;
          }
          scheduleKnowledgeEventRefresh();
        }
        if (kind === "resume") resumeVisibleKnowledgeEventRefresh();
      });
      knowledgeAutoRefreshModel.subscribe(model => model.revision, flushVisibleKnowledgeWindows);
      knowledgeBoardViewModel.subscribe(model => model, ({ windowId, detailOnly, drawer }) => {
        if (drawer) {
          renderKanbanDrawerBody();
          return;
        }
        const element = windowMap.get(windowId);
        const state = knowledgeBridgeStateMap.get(windowId);
        if (!element || !state) return;
        if (detailOnly) renderKnowledgeDetailView(windowId, state);
        else renderKnowledgeBoardView(windowId, element, state);
      });
      // List completion publishes data in receive; the synchronous view owns
      // both the full rebuild and revision-preserving selection updates.
      knowledgeCompletionViewModel.subscribe(model => model, ({ windowId, unchangedEntries, previousNumber, detailRefreshNumber }) => {
        const state = knowledgeBridgeStateMap.get(windowId);
        if (!state) return;
        if (detailRefreshNumber) {
          dispatchKnowledgeDetailRequest(windowId, state.kind, detailRefreshNumber, { explicit: false });
          return;
        }
        if (!unchangedEntries) {
          renderKnowledgeBridge(windowId);
          return;
        }
        const refresh = windowMap.get(windowId)?.querySelector("[data-action='refresh-knowledge']");
        if (refresh) {
          refresh.disabled = !state.refreshEnabled ||
            (state.loading && !knowledgeEntriesAreEmpty(state));
        }
        renderKnowledgeSelection(windowId, state, previousNumber);
      });
      issueWindowViewModel.subscribe(model => model, ({ windowId, detailOnly, focusPreview }) => {
        const element = windowMap.get(windowId);
        const state = knowledgeBridgeStateMap.get(windowId);
        if (!element || !state) return;
        if ((state.error || "") !== (state.reportedLoadDiagnostic || "")) {
          state.reportedLoadDiagnostic = state.error || "";
          if (state.error) {
            send({ kind: "report_knowledge_error", id: windowId, stage: "load", message: state.error });
          }
        }
        try {
          if (detailOnly) {
            const focus = issueExplanationFocus(windowId);
            syncIssuePreviewVisibility(element, state);
            renderIssueDetailPane(windowId, state, element.querySelector(".knowledge-detail-pane"), { agentPreview: state.viewMode !== "split" });
            if (state.error) {
              const detail = element.querySelector(".knowledge-detail-pane");
              if (!detail?.querySelector(".knowledge-notification-badge")) {
                detail?.prepend(createIssueNoticeBadge("Issue load failed", issueWindowErrorKey(windowId), state.error));
              }
            }
            publishIssueControlExplanations(windowId, focus);
            if (focusPreview) focusIssuePreview(windowId, element);
          } else {
            renderIssueKnowledgeBridge(windowId, element, state);
          }
          if (state.renderError) {
            state.renderError = "";
            resolveSurfaceError(`issue-window:${windowId}:render`);
          }
        } catch (error) {
          const message = String(error?.message || error);
          const list = element.querySelector(detailOnly ? ".knowledge-detail-pane" : ".knowledge-list");
          if (list) {
            for (const host of list.querySelectorAll(".issue-queue-items")) keyedKnowledgeLists.get(host)?.dispose();
            list.replaceChildren(createIssueNoticeBadge("Issue rendering failed", `issue-window:${windowId}:render`, message));
          }
          if (state.renderError !== message) {
            state.renderError = message;
            send({ kind: "report_knowledge_error", id: windowId, stage: "render", message });
            reportSurfaceError({ key: `issue-window:${windowId}:render`, title: "Issue window", message });
          }
          throw error;
        }
      });

      return {
        issueMonitorModel,
        knowledgeBridgeStateMap,
        issueContextForNumber,
        ensureKnowledgeBridgeState,
        clearKnowledgeBridgeState,
        requestKnowledgeBridge,
        resyncKnowledge,
        scheduleKnowledgeRelatedWorkRefresh,
        resumeKnowledgeEventRefresh,
        scheduleKnowledgeSearch,
        requestKnowledgeDetail,
        knowledgeDetailRequestMatches,
        renderKnowledgeBridge,
        renderAllKnowledgeBridgeWindows,
        writeKanbanHideDonePreference,
        openKanbanDrawer,
        closeKanbanDrawer,
        renderKanbanDrawerBody,
        mountKnowledgeWindow,
        applyKnowledgeReceiveEvent,
        applyKnowledgeCacheRevision,
        applyIssueMonitorStatus,
        applyIssueMonitorInbox,
        applyIssueMonitorQueueResult,
        scheduleIssueMonitorProjectionRefresh,
        handleKnowledgeTransportChange,
        refreshVisibleKnowledgeWindows,
      };
}
