// SPEC-3064 Phase 3 (E6d) — Knowledge Bridge (Work Item / PR Kanban)
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
// - createNode / createKnowledgeMarkdownBody: shared DOM helpers owned by
//   app.js (the markdown body renderer is shared with the Board surface).
// - windowMap / workspaceWindowById / getWorkspaceWindows: workspace window lookups.
// - pendingIndexOpenTargetsByPreset: index-open handoff targets by preset.
// - knowledgeKindForPreset(preset): issue/pr kind mapping.
// - focusWindowLocally(windowId) / sendWindowFocus(windowId): focus paths.
// - focusOrSpawnPreset(preset): focus-or-spawn used by drawer actions.
// - openIssueLaunchWizard(windowId, issueNumber): launch wizard entry.
// - visibleBounds(): current canvas bounds for resume placement.
// - launchPending: shared Resume/Launch pending controller.
import { createFocusTrap } from "/focus-trap.js";
import { createLaunchOperationId } from "./launch-pending-controller.js";

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

export function issuePreviewStatusView(windowData) {
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
const ISSUE_ROW_LAUNCH_NOW_STATES = new Set(["queued", "launch_failed", "agent_failed"]);
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

function issueEntryHasLabel(entry, name) {
  const labels = Array.isArray(entry?.labels) ? entry.labels : [];
  return labels.some((label) => String(label || "").trim().toLowerCase() === name);
}

function issueRowPrimaryView({ entry, attention, inlineWindow, canvasWindow }) {
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

function issueRowSecondaryItems({ entry, work, attention, primary }) {
  const items = [];
  if (issueEntryStateKey(entry) === "closed" && primary.key !== "issue:closed") {
    items.push({ kind: "chip", key: "closed", label: "Closed" });
  }
  const exclusion = String(entry?.exclusion_reason || "").trim();
  const attentionReason =
    attention?.lane === "needs_attention" ? String(attention.reason || "").trim() : "";
  const reason = exclusion || attentionReason;
  if (reason) {
    items.push({ kind: "reason", key: "reason", label: reason });
  }
  if (Number.isFinite(entry?.queue_position)) {
    items.push({ kind: "chip", key: "queue", label: `Queue ${entry.queue_position}` });
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
  if (inlineWindow) {
    return { order: ["windowize-issue-preview", "configure-issue", ...workActions], limit: 1 };
  }
  if (canvasWindow) {
    return { order: ["focus-canvas-window", "configure-issue", ...workActions], limit: 1 };
  }
  const monitor = monitorStateView(entry?.monitor_state);
  switch (monitor?.state) {
    case "queued":
      return {
        order: ["launch-now", "configure-issue", "move-up", "move-down", ...workActions],
      };
    case "launch_failed":
    case "agent_failed":
      return {
        order: ["launch-now", "continue-work", "resume-work", "configure-issue", "cleanup-work"],
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
  return { order: issueEntryStateKey(entry) === "open" ? ["launch-agent"] : [] };
}

function issueRowActionAvailable(action, { entry, work, queue }) {
  const monitor = monitorStateView(entry?.monitor_state);
  switch (action) {
    case "launch-now":
      return ISSUE_ROW_LAUNCH_NOW_STATES.has(monitor?.state);
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

// SPEC #3885 T-005 / FR-003a: the canvas face of a Windowized agent. The link back
// to the Issue comes from the Work projection's agent rows (window id or session
// id) or from the ids this surface itself Windowized; only windows that are on
// the canvas count, so a preview that returned to the row is never doubled.
export function issueCanvasAgentWindowsForIssue(windows, work, rememberedIds) {
  const list = Array.isArray(windows) ? windows : [];
  const agents = Array.isArray(work?.agents) ? work.agents : [];
  const windowIds = new Set(agents.map((agent) => agent?.window_id).filter(Boolean));
  const sessionIds = new Set(agents.map((agent) => agent?.session_id).filter(Boolean));
  const remembered =
    rememberedIds instanceof Set ? rememberedIds : new Set(rememberedIds || []);
  return list.filter((windowData) => {
    if (!windowData?.id) return false;
    const kind = windowData.placement?.kind || "canvas";
    if (kind !== "canvas") return false;
    if (remembered.has(windowData.id) || windowIds.has(windowData.id)) return true;
    return Boolean(windowData.session_id) && sessionIds.has(windowData.session_id);
  });
}

export function createKnowledgeKanbanSurface({
  send,
  // Semantic search must never use the reconnect queue. This dependency
  // performs one atomic OPEN check + socket.send and reports whether the
  // frame was written; a false result is owned by the retry lifecycle.
  sendKnowledgeSemanticSearchNow = () => false,
  createNode,
  createKnowledgeMarkdownBody,
  windowMap,
  workspaceWindowById,
  getWorkspaceWindows,
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
  workAttentionFor,
  formatWorkLifecycleLabel,
  continueWork,
  openWorkspaceResumePicker,
  openWorkspaceCleanup,
  getResumeBounds,
}) {
      const knowledgeBridgeStateMap = new Map();
      const KNOWLEDGE_AUTO_REFRESH_INTERVAL_MS = 60000;
      let nextKnowledgeLoadRequestId = 1;
      let nextKnowledgeSearchRequestId = 1;
      let relatedWorkRefreshTimer = null;
      let monitorProjectionRefreshTimer = null;
      let issueMonitorStatus = {
        enabled: false,
        state: "disabled",
        queue_len: 0,
        active_count: 0,
        max_active_agents: 1,
        total_candidates: 0,
        autonomous_mode: false,
        quota_hold: null,
      };

      function issueMonitorStateText(state) {
        switch (String(state || "")) {
          case "disabled":
            return "Stopped";
          case "auth_required":
            return "Auth required";
          case "settings_required":
            return "Settings required";
          case "quota_hold":
            return "Quota hold";
          default: {
            const value = String(state || (issueMonitorStatus.enabled ? "idle" : "disabled"));
            return value.charAt(0).toUpperCase() + value.slice(1);
          }
        }
      }

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
        if (
          quotaHold &&
          ["quota_hold", "active", "launching", "settings_required", "idle"].includes(state)
        ) {
          return "quota_hold";
        }
        return state === "quota_hold" ? "idle" : state;
      }

      function renderIssueMonitorControls(element) {
        const panel = element?.querySelector(".knowledge-monitor-panel");
        if (!panel) return;
        const maxActive = Math.max(
          1,
          Number.parseInt(String(issueMonitorStatus.max_active_agents || 1), 10) || 1,
        );
        const summary = panel.querySelector(".knowledge-monitor-summary");
        if (summary) {
          const quotaHold = normalizedIssueMonitorQuotaHold(issueMonitorStatus);
          const state = effectiveIssueMonitorState(issueMonitorStatus, quotaHold);
          const parts = [
            issueMonitorStateText(state),
            `Queue ${issueMonitorStatus.queue_len || 0}`,
            `Active ${issueMonitorStatus.active_count || 0}/${maxActive}`,
          ];
          if (state === "quota_hold") {
            parts.push(`Provider ${quotaHold.provider}`);
            parts.push(`Reset ${quotaHold.reset_at}`);
          }
          if (issueMonitorStatus.total_candidates) {
            parts.push(`Total ${issueMonitorStatus.total_candidates}`);
          }
          summary.textContent = parts.join(" | ");
        }
        const settings = panel.querySelector(".knowledge-monitor-settings-copy");
        if (settings) {
          const source = issueMonitorSettingsSourceLabel(
            issueMonitorStatus.launch_profile_source,
          );
          const profile =
            issueMonitorStatus.launch_profile_summary || "configure before auto start";
          settings.textContent = `Agent settings ${source}: ${profile}`;
        }
        const maxActiveInput = panel.querySelector(".knowledge-monitor-max-active input");
        if (maxActiveInput && document.activeElement !== maxActiveInput) {
          maxActiveInput.value = String(maxActive);
        }
        const toggle = panel.querySelector('[data-action="monitor-toggle"]');
        if (toggle) {
          const enabled = Boolean(issueMonitorStatus.enabled);
          toggle.textContent = enabled ? "Stop" : "Start";
          toggle.dataset.enabled = enabled ? "true" : "false";
          toggle.classList.toggle("primary", !enabled);
        }
        const autonomous = panel.querySelector('[data-action="monitor-autonomous"]');
        if (autonomous) {
          const enabled = Boolean(issueMonitorStatus.autonomous_mode);
          autonomous.textContent = enabled ? "Autonomous: ON" : "Autonomous: OFF";
          autonomous.dataset.enabled = enabled ? "true" : "false";
          autonomous.classList.toggle("primary", enabled);
        }
        const error = panel.querySelector(".knowledge-monitor-error");
        if (error) {
          error.textContent = issueMonitorStatus.last_error || "";
          error.hidden = !issueMonitorStatus.last_error;
        }
      }

      function renderAllIssueMonitorControls() {
        for (const [windowId, state] of knowledgeBridgeStateMap) {
          if (normalizeKnowledgeKind(state.kind) !== "issue") continue;
          renderIssueMonitorControls(windowMap.get(windowId));
        }
      }

      function applyIssueMonitorStatus(nextStatus) {
        issueMonitorStatus = {
          ...issueMonitorStatus,
          ...(nextStatus || {}),
          quota_hold: normalizedIssueMonitorQuotaHold(nextStatus),
        };
        renderAllIssueMonitorControls();
      }

      function scheduleIssueMonitorProjectionRefresh() {
        if (monitorProjectionRefreshTimer !== null) {
          clearTimeout(monitorProjectionRefreshTimer);
        }
        monitorProjectionRefreshTimer = setTimeout(() => {
          monitorProjectionRefreshTimer = null;
          for (const [windowId, state] of knowledgeBridgeStateMap) {
            if (
              normalizeKnowledgeKind(state.kind) === "issue" &&
              workspaceWindowById(windowId)
            ) {
              requestKnowledgeBridge(windowId, "issue", false);
            }
          }
        }, 75);
      }

      function wireIssueMonitorControls(body) {
        const panel = body.querySelector(".knowledge-monitor-panel");
        if (!panel) return;
        panel.addEventListener("mousedown", (event) => event.stopPropagation());
        panel
          .querySelector('[data-action="monitor-settings"]')
          ?.addEventListener("click", () => {
            send({ kind: "issue_monitor_configure_profile" });
          });
        const maxActiveInput = panel.querySelector(".knowledge-monitor-max-active input");
        maxActiveInput?.addEventListener("change", () => {
          const value = Math.max(
            1,
            Number.parseInt(String(maxActiveInput.value || "1"), 10) || 1,
          );
          maxActiveInput.value = String(value);
          send({
            kind: "set_issue_monitor_max_active_agents",
            max_active_agents: value,
          });
        });
        panel
          .querySelector('[data-action="monitor-toggle"]')
          ?.addEventListener("click", () => {
            send({
              kind: "set_issue_monitor_enabled",
              enabled: !Boolean(issueMonitorStatus.enabled),
            });
          });
        panel
          .querySelector('[data-action="monitor-autonomous"]')
          ?.addEventListener("click", () => {
            send({
              kind: "set_issue_monitor_autonomous_mode",
              enabled: !Boolean(issueMonitorStatus.autonomous_mode),
            });
          });
        const quickTitle = panel.querySelector(".knowledge-monitor-quick-title");
        const submitQuickIssue = (launch) => {
          const title = String(quickTitle?.value || "").trim();
          if (!title) return;
          send({ kind: "quick_register_issue", title, launch });
          quickTitle.value = "";
        };
        quickTitle?.addEventListener("keydown", (event) => {
          if (event.key !== "Enter") return;
          event.preventDefault();
          submitQuickIssue(false);
        });
        panel
          .querySelector('[data-action="quick-register-launch"]')
          ?.addEventListener("click", () => submitQuickIssue(true));
        renderIssueMonitorControls(body);
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
            createKnowledgeMarkdownBody(section, "kanban-drawer-section-body"),
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
            selectedNumber: null,
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
            loadRecoveryRetryCount: 0,
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
            dndSnapshot: null,
            pendingPhaseUpdates: new Map(),
            autoRefreshTimer: null,
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
          if (!state.refreshEnabled || knowledgeAutoRefreshIsBusy(state)) {
            return;
          }
          requestKnowledgeBridge(windowId, knowledgeKind, false);
        }, KNOWLEDGE_AUTO_REFRESH_INTERVAL_MS);
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
        const state = knowledgeBridgeStateMap.get(windowId);
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
          state.loadRecoveryRetryCount = 0;
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
        if (
          knowledgeBridgeStateMap.size === 0 &&
          monitorProjectionRefreshTimer !== null
        ) {
          clearTimeout(monitorProjectionRefreshTimer);
          monitorProjectionRefreshTimer = null;
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
            state.loadRequestId !== requestId ||
            !knowledgeEntriesAreEmpty(state)
          ) {
            return;
          }
          if (state.loadRecoveryRetryCount < 1) {
            state.loadRecoveryRetryCount += 1;
            state.loading = false;
            state.refreshing = false;
            // Issue #3297: the retry must stay a cache read. Escalating to
            // refresh=true ran a full remote sync that takes minutes and
            // always outlived the next 5s timer, turning one slow load into
            // a guaranteed "Timed out loading cache-backed data".
            requestKnowledgeBridge(windowId, knowledgeKind, false);
            renderKnowledgeBridge(windowId);
            return;
          }
          state.loading = false;
          state.refreshing = false;
          state.error = "Timed out loading cache-backed data";
          renderKnowledgeBridge(windowId);
        }, 5000);
      }

      function finishKnowledgeLoad(state, windowId, knowledgeKind) {
        clearKnowledgeLoadRecoveryTimer(state);
        state.loading = false;
        state.refreshing = false;
        state.loadRecoveryRetryCount = 0;
        const queuedRefresh = state.queuedLoadRefresh;
        state.queuedLoadRefresh = false;
        if (queuedRefresh && workspaceWindowById(windowId)) {
          requestKnowledgeBridge(windowId, knowledgeKind, true);
          return true;
        }
        return false;
      }

      function requestKnowledgeBridge(windowId, knowledgeKind, refresh = false) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        if (state.loading) {
          if (refresh && knowledgeEntriesAreEmpty(state)) {
            clearKnowledgeLoadRecoveryTimer(state);
            state.loading = false;
            state.refreshing = false;
          } else {
            state.queuedLoadRefresh = state.queuedLoadRefresh || Boolean(refresh);
            return;
          }
        }
        if (state.pendingSearchTimer !== null) {
          clearTimeout(state.pendingSearchTimer);
          state.pendingSearchTimer = null;
        }
        const requestId = nextKnowledgeLoadRequestId++;
        state.loadRequestId = requestId;
        if (normalizeKnowledgeKind(state.kind) === "pr") {
          // PR selection still completes through the legacy full-view path.
          // A newer PR load supersedes that selection owner just as it did
          // before Issue/SPEC detail requests gained independent ownership.
          state.detailRequestId = 0;
        }
        state.ownedLoadRequestIds.add(requestId);
        while (state.ownedLoadRequestIds.size > 4) {
          state.ownedLoadRequestIds.delete(
            state.ownedLoadRequestIds.values().next().value,
          );
        }
        state.loadSelectionGeneration = state.selectionGeneration;
        state.loadSelectedNumber = state.selectedNumber;
        state.loading = true;
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

      function scheduleKnowledgeRelatedWorkRefresh() {
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
            const knowledgeKind = state.kind || knowledgeKindForPreset(windowData.preset);
            if (!knowledgeKind) {
              continue;
            }
            requestKnowledgeBridge(windowId, knowledgeKind, false);
          }
        }, 150);
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
        if (normalizeKnowledgeKind(state.kind) === "pr") {
          if (!event.request_id) {
            return event.detail?.number === state.selectedNumber;
          }
          return (
            event.request_id === state.loadRequestId ||
            event.request_id === state.detailRequestId
          );
        }
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
        // PR intentionally retains its pre-SPEC-3170 behavior.
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
        if (state.loading && state.baseEntries.length === 0) {
          state.searching = true;
          renderKnowledgeBridge(windowId);
          return;
        }
        if (
          isSilentSemanticKind(intent.kind) &&
          state.semanticRetryActive &&
          state.semanticRetryTimer !== null &&
          !state.inFlightSearchIntent
        ) {
          state.searching = false;
          renderKnowledgeBridge(windowId);
          return;
        }
        if (state.inFlightSearchIntent) {
          state.queuedSearchIntent = intent;
          state.queuedSearchQuery = query;
          state.searching = true;
          renderKnowledgeBridge(windowId);
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
        renderKnowledgeBridge(windowId);
      }

      function dispatchKnowledgeDetailRequest(
        windowId,
        knowledgeKind,
        number,
        { explicit = false } = {},
      ) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        const previousNumber = state.selectedNumber;
        const prBaseline = normalizeKnowledgeKind(state.kind) === "pr";
        if (explicit) {
          state.selectedNumber = number;
          if (!prBaseline) {
            // Issue/SPEC selection is a local transition before any I/O.
            state.selectionGeneration = (state.selectionGeneration || 0) + 1;
            state.error = "";
            const findRow = (rows) =>
              Array.isArray(rows)
                ? rows.find((entry) => entry && entry.number === number)
                : null;
            const row = findRow(state.entries) || findRow(state.baseEntries) || null;
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
          }
        } else if (number !== state.selectedNumber) {
          return false;
        }
        state.detailLoading = true;
        const requestId = nextKnowledgeLoadRequestId++;
        state.detailRequestId = requestId;
        if (!prBaseline) {
          state.detailRequestSelectionGeneration = state.selectionGeneration;
          state.detailRequestNumber = number;
        }
        const effectiveKind = knowledgeKind || state.kind;
        if (prBaseline) {
          renderKnowledgeBridge(windowId);
        } else if (explicit) {
          renderKnowledgeSelection(windowId, state, previousNumber);
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
          case "pr":
            return "PR bridge";
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
          case "pr":
            return "Search unavailable";
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
        const works = Array.isArray(detail?.related_works)
          ? detail.related_works
          : [];
        if (works.length === 0) {
          return null;
        }

        const section = createNode("section", "knowledge-section knowledge-related-works");
        section.appendChild(createNode("div", "knowledge-section-title", "Related Work"));
        const list = createNode("div", "knowledge-related-work-list");
        const liveSessionWindowsByConversation =
          buildKnowledgeRelatedLiveSessionWindowsByConversation(works);
        for (const work of works) {
          const card = createNode("article", "knowledge-related-work");
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

      function issueEntryState(entry) {
        return String(entry?.state || "open").toLowerCase() === "closed"
          ? "closed"
          : "open";
      }

      function issueEntryMatchesStateFilter(entry, filter) {
        if (filter === "all") return true;
        return issueEntryState(entry) === filter;
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
        return (Array.isArray(state.entries) ? state.entries : []).filter((entry) =>
          issueEntryMatchesStateFilter(entry, state.issueStateFilter || "open"),
        );
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
        const previews = issuePreviewWindowsForIssue(
          typeof getWorkspaceWindows === "function" ? getWorkspaceWindows() : [],
          windowId,
          state.selectedNumber,
        );
        if (previews.length === 0) {
          return null;
        }
        const target = previews[0];
        const section = createNode("section", "issue-preview");
        section.dataset.windowId = target.id;
        section.dataset.issueNumber = String(state.selectedNumber);

        const header = createNode("div", "issue-preview-header");
        const titleWrap = createNode("div", "issue-preview-title-wrap");
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
        header.appendChild(badge);

        const windowize = createNode("button", "wizard-button", "Windowize");
        windowize.type = "button";
        windowize.dataset.action = "windowize-issue-preview";
        windowize.setAttribute("aria-label", "Windowize agent preview");
        windowize.addEventListener("click", (event) => {
          event.preventDefault();
          event.stopPropagation();
          windowizeIssuePreviewWindow?.(target.id);
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
      // shows a "Shown on canvas" face (FR-012) that offers focus, never a second
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
            onCanvas ? `${meta} · Shown on canvas` : meta,
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
              "Shown on canvas. Input goes to the canvas window.",
            ),
          );
          row.appendChild(issueRowActionButton("focus-canvas-window", context));
          return row;
        }

        const statusView = issuePreviewStatusView(target);
        const since = windowRuntimeStateSince?.(target.id);
        const elapsed = createNode(
          "span",
          "issue-agent-status-elapsed",
          Number.isFinite(since) ? formatAgentElapsed(Date.now() - since) : "",
        );
        elapsed.title = elapsed.textContent ? `${statusView.label} for ${elapsed.textContent}` : "";
        row.appendChild(elapsed);

        const output = createNode(
          "div",
          "issue-agent-status-output",
          String(windowActivityDetail?.(target) || "").trim(),
        );
        output.title = output.textContent;
        row.appendChild(output);

        row.appendChild(issueRowActionButton("windowize-issue-preview", context));
        return row;
      }

      function renderKnowledgeDetailPane(windowId, state, detailPane) {
        detailPane.innerHTML = "";
        const preview = renderIssueAgentPreview(windowId, state);
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
          card.appendChild(createKnowledgeMarkdownBody(section));
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

      function renderKnowledgeDetailOnly(windowId, state) {
        const element = windowMap.get(windowId);
        const detailPane = element?.querySelector(".knowledge-detail-pane");
        if (!detailPane) {
          return;
        }
        renderKnowledgeDetailPane(windowId, state, detailPane);
      }

      function renderKnowledgeSelection(windowId, state, previousNumber) {
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
        renderKnowledgeDetailOnly(windowId, state);
      }

      function renderKnowledgeStatusOnly(windowId, state) {
        const element = windowMap.get(windowId);
        const status = element?.querySelector(".knowledge-status");
        if (!status) {
          return;
        }
        const issueSurface = isSilentSemanticKind(state.kind);
        status.className = "knowledge-status";
        status.textContent = "";
        if (state.error) {
          status.classList.add("visible", "error");
          status.textContent = state.error;
        } else if (!issueSurface && state.searching) {
          status.classList.add("visible", "info");
          status.textContent = "Searching semantic index";
        } else if (state.loading && state.entries.length > 0) {
          status.classList.add("visible", "info");
          status.textContent = state.refreshing
            ? issueSurface
              ? "Refreshing cached work items"
              : "Refreshing cached knowledge"
            : issueSurface
              ? "Loading cache-backed work items"
              : "Loading cache-backed data";
        } else if (state.loading && state.entries.length === 0) {
          status.classList.add("visible", "info");
          status.textContent = issueSurface
            ? "Loading cache-backed work items"
            : "Loading cache-backed data";
        } else if (state.entries.length === 0 && !state.searching) {
          status.classList.add("visible", "info");
          status.textContent = state.emptyMessage || (issueSurface
            ? "No cached work items"
            : "No cached items");
        }
      }

      function canonicalQueuedKnowledgeEntries(state) {
        const source = Array.isArray(state.baseEntries) && state.baseEntries.length > 0
          ? state.baseEntries
          : state.entries;
        return (Array.isArray(source) ? source : [])
          .filter(
            (entry) =>
              entry?.monitor_state === "queued" &&
              Number.isFinite(entry.queue_position),
          )
          .slice()
          .sort(
            (left, right) =>
              Number(left.queue_position) - Number(right.queue_position) ||
              Number(left.number) - Number(right.number),
          );
      }

      function updateQueuedKnowledgePositions(entries, positions) {
        if (!Array.isArray(entries)) return;
        for (let index = 0; index < entries.length; index += 1) {
          const entry = entries[index];
          const queuePosition = positions.get(entry?.number);
          if (queuePosition === undefined || entry.queue_position === queuePosition) {
            continue;
          }
          entries[index] = { ...entry, queue_position: queuePosition };
        }
      }

      function moveQueuedKnowledgeEntry(windowId, state, issueNumber, direction) {
        const queued = canonicalQueuedKnowledgeEntries(state);
        const index = queued.findIndex((entry) => entry.number === issueNumber);
        const targetIndex = index + direction;
        if (index < 0 || targetIndex < 0 || targetIndex >= queued.length) return;
        [queued[index], queued[targetIndex]] = [queued[targetIndex], queued[index]];
        const positions = new Map(
          queued.map((entry, queueIndex) => [entry.number, queueIndex + 1]),
        );
        updateQueuedKnowledgePositions(state.baseEntries, positions);
        updateQueuedKnowledgePositions(state.entries, positions);
        send({
          kind: "reorder_issue_monitor_issues",
          issue_numbers: queued.map((entry) => entry.number),
        });
        renderKnowledgeBridge(windowId);
      }

      // SPEC #3885 T-004 (FR-006): every Issue action the row can offer, keyed by
      // the `data-action` the tests and the Playwright specs address.
      const ISSUE_ROW_ACTION_VIEWS = Object.freeze({
        "launch-now": Object.freeze({ label: "Launch now", aria: "Launch now" }),
        "configure-issue": Object.freeze({
          label: "Settings",
          aria: "Project Agent settings for",
        }),
        "move-up": Object.freeze({ label: "↑ Move up", aria: "Move up" }),
        "move-down": Object.freeze({ label: "↓ Move down", aria: "Move down" }),
        "continue-work": Object.freeze({ label: "Continue work", aria: "Continue work on" }),
        "resume-work": Object.freeze({ label: "Resume", aria: "Resume work on" }),
        "cleanup-work": Object.freeze({ label: "Clean Up", aria: "Clean up work for" }),
        "launch-agent": Object.freeze({ label: "Launch agent", aria: "Launch an agent for" }),
        "windowize-issue-preview": Object.freeze({
          label: "Windowize",
          aria: "Open the agent terminal as a canvas window for",
        }),
        "focus-canvas-window": Object.freeze({
          label: "Focus window",
          aria: "Focus the agent's canvas window for",
        }),
      });
      // Actions rendered inside the agent status row rather than the row's
      // action group.
      const ISSUE_ROW_TERMINAL_ACTIONS = new Set([
        "windowize-issue-preview",
        "focus-canvas-window",
      ]);
      // Ids this surface Windowized, so the row keeps its "Shown on canvas" face
      // even before the Work projection reports the agent's window id.
      const windowizedAgentWindowIds = new Set();

      function runIssueRowAction(action, { windowId, state, entry, work, target }) {
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
          case "move-up":
            moveQueuedKnowledgeEntry(windowId, state, entry.number, -1);
            return;
          case "move-down":
            moveQueuedKnowledgeEntry(windowId, state, entry.number, 1);
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
        if (action === "move-up" && queue) {
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

      function issueRowFaces(windowId, entry, work) {
        const windows = typeof getWorkspaceWindows === "function" ? getWorkspaceWindows() : [];
        const inlineWindow = issuePreviewWindowsForIssue(windows, windowId, entry.number)[0] || null;
        const canvasWindow = inlineWindow
          ? null
          : issueCanvasAgentWindowsForIssue(windows, work, windowizedAgentWindowIds)[0] || null;
        return { inlineWindow, canvasWindow };
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
        main.appendChild(badge);
        select.appendChild(main);

        if (model.secondary.length > 0) {
          const secondary = createNode("div", "knowledge-row-secondary");
          for (const item of model.secondary) {
            const node = createNode("span", "knowledge-row-secondary-item", item.label);
            node.dataset.kind = item.kind;
            node.dataset.key = item.key;
            if (item.title) {
              node.title = item.title;
            }
            secondary.appendChild(node);
          }
          select.appendChild(secondary);
        }

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
        const list = element.querySelector(".knowledge-list");
        const detailPane = element.querySelector(".knowledge-detail-pane");
        const status = element.querySelector(".knowledge-status");
        const refreshButton = element.querySelector("[data-action='refresh-knowledge']");
        const searchInput = element.querySelector(".knowledge-search");
        if (!list || !detailPane || !status || !refreshButton || !searchInput) {
          return;
        }

        refreshButton.disabled =
          !state.refreshEnabled || (state.loading && !knowledgeEntriesAreEmpty(state));
        searchInput.placeholder = knowledgeSearchPlaceholder(state.kind);
        for (const button of element.querySelectorAll("[data-issue-filter]")) {
          const selected = button.dataset.issueFilter === state.issueStateFilter;
          button.classList.toggle("is-active", selected);
          button.setAttribute("aria-pressed", selected ? "true" : "false");
        }

        renderKnowledgeStatusOnly(windowId, state);

        list.innerHTML = "";
        const visibleEntries = filteredIssueEntries(state);
        if (visibleEntries.length === 0) {
          const filterLabel = state.issueStateFilter === "all"
            ? ""
            : `${state.issueStateFilter || "open"} `;
          list.appendChild(
            createNode("div", "knowledge-empty", `No ${filterLabel}work items`),
          );
        } else {
          for (const entry of visibleEntries) {
            list.appendChild(renderIssueRow(windowId, state, entry));
          }
        }
        renderKnowledgeDetailPane(windowId, state, detailPane);
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
        if (state.kind === "issue") {
          renderIssueKnowledgeBridge(windowId, element, state);
          return;
        }
        const board = element.querySelector(".kanban-board");
        const detailPane = element.querySelector(".knowledge-detail-pane");
        const status = element.querySelector(".knowledge-status");
        const refreshButton = element.querySelector("[data-action='refresh-knowledge']");
        const searchInput = element.querySelector(".knowledge-search");
        const hideDoneToggle = element.querySelector("[data-action='kanban-hide-done']");
        if (!board || !detailPane || !status || !refreshButton || !searchInput) {
          return;
        }

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
          const body = column.querySelector("[data-role='body']");
          if (body) {
            body.innerHTML = "";
          }
          columnsByPhase.set(column.dataset.phase, column);
          if (column.dataset.kanbanWired !== "true") {
            wireKanbanColumnDropTarget(windowId, column);
            column.dataset.kanbanWired = "true";
          }
        }
        const counts = new Map();
        for (const entry of visibleEntries) {
          const phaseKey = effectiveKnowledgePhase(entry);
          const column = columnsByPhase.get(phaseKey) || columnsByPhase.get("backlog");
          if (!column) continue;
          const body = column.querySelector("[data-role='body']");
          if (!body) continue;
          const card = renderKanbanCard(windowId, state, entry);
          body.appendChild(card);
          counts.set(phaseKey, (counts.get(phaseKey) || 0) + 1);
        }
        for (const [phase, column] of columnsByPhase) {
          const countLabel = column.querySelector("[data-role='count']");
          if (countLabel) {
            countLabel.textContent = String(counts.get(phase) || 0);
          }
          const body = column.querySelector("[data-role='body']");
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
                    <div class="knowledge-state-filter" role="group" aria-label="Issue state filter">
                      <button type="button" data-issue-filter="open">Open</button>
                      <button type="button" data-issue-filter="closed">Closed</button>
                      <button type="button" data-issue-filter="all">All</button>
                    </div>
                  </div>
                  <div class="workspace-toolbar-actions">
                    <button class="icon-button" data-action="refresh-knowledge" aria-label="Refresh cached work items">↻</button>
                  </div>
                </div>
                <section class="knowledge-monitor-panel" aria-label="Issue execution monitor">
                  <div class="knowledge-monitor-overview">
                    <div class="knowledge-monitor-summary" aria-live="polite">Stopped | Queue 0 | Active 0/1</div>
                    <div class="knowledge-monitor-settings-copy">Agent settings Missing saved profile: configure before auto start</div>
                  </div>
                  <div class="knowledge-monitor-controls">
                    <button type="button" class="wizard-button" data-action="monitor-settings">Agent settings</button>
                    <label class="knowledge-monitor-max-active">
                      <span>Max active</span>
                      <input type="number" min="1" step="1" value="1" />
                    </label>
                    <button type="button" class="wizard-button primary" data-action="monitor-toggle">Start</button>
                    <button type="button" class="wizard-button" data-action="monitor-autonomous">Autonomous: OFF</button>
                  </div>
                  <div class="knowledge-monitor-quick">
                    <input class="knowledge-monitor-quick-title" type="text" placeholder="Quick issue title…" aria-label="Quick issue title" />
                    <button type="button" class="wizard-button" data-action="quick-register-launch">⚡ Register &amp; Launch</button>
                  </div>
                  <div class="knowledge-monitor-error" role="alert" hidden></div>
                </section>
                <div class="knowledge-status"></div>
                <div class="knowledge-split workspace-split issue-list-shell">
                  <div class="knowledge-list-pane">
                    <div class="knowledge-list" role="list" aria-label="Cached work items"></div>
                  </div>
                  <div class="knowledge-detail-pane"></div>
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
          const pendingIndexTarget = pendingIndexOpenTargetsByPreset.get(windowData.preset);
          if (
            pendingIndexTarget
            && pendingIndexTarget.knowledgeKind === knowledgeKind
          ) {
            state.selectedNumber = pendingIndexTarget.number;
            pendingIndexOpenTargetsByPreset.delete(windowData.preset);
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
          if (knowledgeKind === "issue") {
            wireIssueMonitorControls(body);
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
          case "knowledge_entries": {
            const state = knowledgeBridgeStateMap.get(event.id);
            if (
              !state ||
              normalizeKnowledgeKind(event.knowledge_kind) !==
                normalizeKnowledgeKind(state.kind)
            ) {
              break;
            }
            const prSelectionCompletion = Boolean(
              normalizeKnowledgeKind(state.kind) === "pr" &&
              event.request_id &&
              event.request_id === state.detailRequestId,
            );
            if (
              event.request_id &&
              !state.ownedLoadRequestIds.has(event.request_id) &&
              !prSelectionCompletion
            ) {
              break;
            }
            // Issue #3297: a response that lost the race against the 5s
            // recovery timer carries a superseded request_id, but while the
            // window still has no data it is strictly better than the empty
            // view — apply it; a newer in-flight response overwrites it.
            if (
              event.request_id &&
              event.request_id !== state.loadRequestId &&
              !knowledgeEntriesAreEmpty(state) &&
              !prSelectionCompletion
            ) {
              break;
            }
            const queuedQuery = state.query.trim();
            const incomingEntries = event.entries || [];
            const keepSelectedNumber =
              state.selectedNumber &&
              incomingEntries.some((entry) => entry.number === state.selectedNumber);
            state.baseEntries = incomingEntries;
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
            state.error = "";
            if (finishKnowledgeLoad(state, event.id, event.knowledge_kind)) {
              renderKnowledgeBridge(event.id);
              break;
            }
            if (queuedQuery) {
              scheduleKnowledgeSearch(
                event.id,
                event.knowledge_kind,
              );
              break;
            }
            renderKnowledgeBridge(event.id);
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
            if (selectionIsCurrent && state.selectedNumber) {
              dispatchKnowledgeDetailRequest(
                event.id,
                activeIntent.kind,
                state.selectedNumber,
                { explicit: false },
              );
            } else if (selectionIsCurrent && state.selectionGeneration === 0) {
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
            state.detail = event.detail;
            state.selectedNumber = event.detail?.number ?? state.selectedNumber ?? null;
            if (matchesLoadRequest) {
              finishKnowledgeLoad(state, event.id, event.knowledge_kind);
            }
            state.detailLoading = false;
            if (normalizeKnowledgeKind(state.kind) === "pr") {
              renderKnowledgeBridge(event.id);
            } else {
              renderKnowledgeSelection(event.id, state, previousNumber);
            }
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
              renderKanbanDrawerBody();
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
            const prSelectionError =
              normalizeKnowledgeKind(state.kind) === "pr" &&
              event.request_id === state.detailRequestId;
            const matchesDetailRequest =
              event.request_id === state.detailRequestId &&
              (prSelectionError ||
                (state.detailRequestSelectionGeneration === state.selectionGeneration &&
                  state.detailRequestNumber === state.selectedNumber));
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
            if (matchesLoadRequest || prSelectionError) {
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

      return {
        knowledgeBridgeStateMap,
        ensureKnowledgeBridgeState,
        clearKnowledgeBridgeState,
        requestKnowledgeBridge,
        scheduleKnowledgeRelatedWorkRefresh,
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
        applyIssueMonitorStatus,
        scheduleIssueMonitorProjectionRefresh,
        handleKnowledgeTransportChange,
      };
}
