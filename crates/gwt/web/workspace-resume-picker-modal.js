// SPEC-2359 US-42 — Workspace Resume Picker modal.
//
// Rendered when the user clicks the Resume button on a Workspace card.
// Lists previously-assigned agents (filtered to resumable candidates by
// the backend) so the user can pick which agent to restart in-place.
// Spawning bypasses the Launch Wizard so Resume preserves the previous
// conversation handle without re-prompting for runtime / model / etc.

import { createFocusTrap } from "./focus-trap.js";

// Trim a full worktree path down to the last two segments so the picker
// row stays scannable on small modals. e.g.
// `/Users/akiojin/Workbench/gwt/work/20260518-0818` becomes
// `gwt / work/20260518-0818`. Leading-slash paths drop the absolute
// prefix so the user sees "what" not "where".
function shortenWorktreePath(path) {
  if (typeof path !== "string" || !path) return "";
  const trimmed = path.replace(/\\+/g, "/").replace(/\/+$/, "");
  const parts = trimmed.split("/").filter(Boolean);
  if (parts.length <= 2) return trimmed;
  return `${parts[parts.length - 2]} / ${parts[parts.length - 1]}`;
}

// Convert an ISO 8601 timestamp into a compact relative string ("3m ago"
// / "2h ago" / "yesterday") so the row sticks to one line instead of
// showing the full RFC-3339 form. Falls back to the raw value when
// parsing fails so the picker never silently drops a timestamp.
function formatRelativeTime(iso) {
  if (typeof iso !== "string" || !iso) return "";
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return iso;
  const diff = Date.now() - ms;
  if (diff < 0) return new Date(ms).toLocaleString();
  const sec = Math.floor(diff / 1000);
  if (sec < 60) return `${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const days = Math.floor(hr / 24);
  if (days < 7) return `${days}d ago`;
  return new Date(ms).toLocaleDateString();
}

// SPEC-2356 — preserve the focus return target and focus trap release
// between successive `renderWorkspaceResumePicker` calls without leaking
// when the modal element is detached.
const focusReturnMap = new WeakMap();
const focusTrapMap = new WeakMap();

/**
 * Render the Workspace Resume Picker into `modalEl` / `dialogEl`. The
 * caller passes a `state` describing whether the modal is open, the
 * backend-supplied `agents` list, an optional per-entry error, and a set
 * of behavior callbacks. The function is a pure rendering primitive so
 * smoke tests can drive it without launching `app.js`.
 */
export function renderWorkspaceResumePicker({
  modalEl,
  dialogEl,
  state,
  createNode,
  onPick,
  onDismiss,
}) {
  if (!modalEl || !dialogEl) return;
  if (!state || !state.open) {
    const wasOpen = modalEl.classList.contains("open");
    modalEl.classList.remove("open");
    modalEl.setAttribute("aria-hidden", "true");
    while (dialogEl.firstChild) dialogEl.removeChild(dialogEl.firstChild);
    if (wasOpen) {
      const releaseTrap = focusTrapMap.get(modalEl);
      focusTrapMap.delete(modalEl);
      if (typeof releaseTrap === "function") releaseTrap();
      const returnTo = focusReturnMap.get(modalEl);
      focusReturnMap.delete(modalEl);
      if (returnTo && typeof returnTo.focus === "function") {
        try { returnTo.focus({ preventScroll: true }); }
        catch { returnTo.focus(); }
      }
    }
    return;
  }

  const wasOpen = modalEl.classList.contains("open");
  if (!wasOpen) {
    const ownerDoc = modalEl.ownerDocument || (typeof document !== "undefined" ? document : null);
    if (ownerDoc) {
      focusReturnMap.set(modalEl, ownerDoc.activeElement);
      const release = createFocusTrap(dialogEl, { document: ownerDoc });
      focusTrapMap.set(modalEl, release);
    }
  }
  modalEl.classList.add("open");
  modalEl.removeAttribute("aria-hidden");
  while (dialogEl.firstChild) dialogEl.removeChild(dialogEl.firstChild);

  dialogEl.appendChild(createNode("h2", "workspace-resume-picker-title", "Resume Work"));
  dialogEl.appendChild(
    createNode(
      "p",
      "workspace-resume-picker-subtitle",
      "Pick which previously-assigned agent to restart.",
    ),
  );

  const agents = Array.isArray(state.agents) ? state.agents : [];
  if (agents.length === 0) {
    dialogEl.appendChild(
      createNode(
        "div",
        "workspace-resume-picker-empty",
        "No resumable agents for this Work.",
      ),
    );
  } else {
    const list = createNode("div", "workspace-resume-picker-list");
    for (const agent of agents) {
      const row = createNode("button", "workspace-resume-picker-row");
      row.type = "button";
      row.dataset.sessionId = agent.session_id;
      const heading = createNode("div", "workspace-resume-picker-row-heading");
      heading.appendChild(
        createNode(
          "span",
          "workspace-resume-picker-row-name",
          agent.display_name || agent.agent_id,
        ),
      );
      const lifecycleTag =
        agent.lifecycle_status === "running"
          ? { className: "is-running", label: "Running" }
          : agent.lifecycle_status === "interrupted"
            ? { className: "is-interrupted", label: "Interrupted" }
            : agent.lifecycle_status === "active"
              ? { className: "is-active", label: "Active" }
              : null;
      const tagClass = lifecycleTag
        ? `workspace-resume-picker-row-tag ${lifecycleTag.className}`
        : agent.resume_kind === "metadata_only"
          ? "workspace-resume-picker-row-tag is-fresh"
          : "workspace-resume-picker-row-tag is-conversation";
      heading.appendChild(
        createNode(
          "span",
          tagClass,
          lifecycleTag?.label
            || (agent.resume_kind === "metadata_only" ? "Fresh start" : "Conversation"),
        ),
      );
      row.appendChild(heading);

      // Pretty-print metadata: branch / shortened worktree / readable
      // timestamp on their own rows. Long paths are trimmed to the last
      // two path segments so the row stays scannable.
      const meta = createNode("div", "workspace-resume-picker-row-meta");
      if (agent.branch) {
        const branchRow = createNode("div", "workspace-resume-picker-row-meta-line");
        branchRow.appendChild(createNode("span", "workspace-resume-picker-row-meta-label", "Branch"));
        branchRow.appendChild(createNode("span", "workspace-resume-picker-row-meta-value", agent.branch));
        meta.appendChild(branchRow);
      }
      if (agent.worktree_path) {
        const shortPath = shortenWorktreePath(agent.worktree_path);
        const pathRow = createNode("div", "workspace-resume-picker-row-meta-line");
        pathRow.appendChild(createNode("span", "workspace-resume-picker-row-meta-label", "Worktree"));
        pathRow.appendChild(createNode("span", "workspace-resume-picker-row-meta-value", shortPath));
        meta.appendChild(pathRow);
      }
      if (agent.last_activity_at) {
        const whenRow = createNode("div", "workspace-resume-picker-row-meta-line");
        whenRow.appendChild(createNode("span", "workspace-resume-picker-row-meta-label", "Last"));
        whenRow.appendChild(
          createNode(
            "span",
            "workspace-resume-picker-row-meta-value",
            formatRelativeTime(agent.last_activity_at),
          ),
        );
        meta.appendChild(whenRow);
      }
      if (meta.childElementCount > 0) row.appendChild(meta);
      row.addEventListener("click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        if (typeof onPick === "function") onPick(agent);
      });
      list.appendChild(row);
    }
    dialogEl.appendChild(list);
  }

  if (state.error) {
    dialogEl.appendChild(
      createNode("div", "workspace-resume-picker-error", state.error),
    );
  }

  const actions = createNode("div", "workspace-resume-picker-actions");
  const cancel = createNode("button", "wizard-button", "Cancel");
  cancel.type = "button";
  cancel.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (typeof onDismiss === "function") onDismiss();
  });
  actions.appendChild(cancel);
  dialogEl.appendChild(actions);
}

/**
 * Factory used by app.js to create the in-memory state and wire DOM-
 * agnostic helpers. Returns `{ open, render, handleAgentsList,
 * handleError, isOpen }` for the dispatcher to call from the WebSocket
 * loop. Keeping state local to the factory avoids leaking through the
 * app-level `state` object and keeps the unit test surface narrow.
 */
export function createWorkspaceResumePickerController({
  modalEl,
  dialogEl,
  createNode,
  send,
  getResumeBounds,
}) {
  const state = {
    open: false,
    agents: [],
    workspaceId: null,
    error: "",
  };

  function close() {
    state.open = false;
    state.agents = [];
    state.workspaceId = null;
    state.error = "";
    render();
  }

  function pick(agent) {
    if (!agent || !agent.session_id) return;
    state.error = "";
    const bounds = typeof getResumeBounds === "function" ? getResumeBounds() : null;
    if (!bounds) {
      state.error = "Cannot determine viewport bounds.";
      render();
      return;
    }
    if (typeof send === "function") {
      send({
        kind: "resume_workspace_agent",
        session_id: agent.session_id,
        bounds,
      });
    }
    // Optimistically close; if backend reports failure we re-open via
    // handleError.
    close();
  }

  function render() {
    renderWorkspaceResumePicker({
      modalEl,
      dialogEl,
      state,
      createNode,
      onPick: pick,
      onDismiss: close,
    });
  }

  return {
    isOpen: () => state.open,
    open: (workspaceId) => {
      state.open = true;
      state.agents = [];
      state.workspaceId = workspaceId ?? null;
      state.error = "";
      render();
    },
    handleAgentsList: (event) => {
      if (!state.open) {
        // Backend can broadcast list updates while the picker is closed if
        // the user already dismissed; ignore those rather than re-opening
        // the modal unexpectedly.
        return;
      }
      state.agents = Array.isArray(event?.agents) ? event.agents : [];
      render();
    },
    handleError: (event) => {
      state.open = true;
      state.error = event?.message || "Failed to resume the selected agent.";
      render();
    },
    dismiss: close,
    render,
  };
}
