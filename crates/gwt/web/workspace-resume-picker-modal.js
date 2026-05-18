// SPEC-2359 US-42 — Workspace Resume Picker modal.
//
// Rendered when the user clicks the Resume button on a Workspace card.
// Lists previously-assigned agents (filtered to resumable candidates by
// the backend) so the user can pick which agent to restart in-place.
// Spawning bypasses the Launch Wizard so Resume preserves the previous
// conversation handle without re-prompting for runtime / model / etc.

import { createFocusTrap } from "./focus-trap.js";

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

  dialogEl.appendChild(createNode("h2", "workspace-resume-picker-title", "Resume Workspace"));
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
        "No resumable agents for this Workspace.",
      ),
    );
  } else {
    const list = createNode("div", "workspace-resume-picker-list");
    for (const agent of agents) {
      const row = createNode("button", "workspace-resume-picker-row wizard-button");
      row.type = "button";
      row.dataset.sessionId = agent.session_id;
      const heading = createNode("div", "workspace-resume-picker-row-heading");
      heading.appendChild(
        createNode("span", "workspace-resume-picker-row-name", agent.display_name || agent.agent_id),
      );
      if (agent.resume_kind === "metadata_only") {
        heading.appendChild(
          createNode("span", "workspace-resume-picker-row-tag", "Fresh start"),
        );
      } else {
        heading.appendChild(
          createNode("span", "workspace-resume-picker-row-tag", "Conversation"),
        );
      }
      row.appendChild(heading);
      const meta = createNode("div", "workspace-resume-picker-row-meta");
      if (agent.branch) meta.appendChild(createNode("span", "", agent.branch));
      if (agent.worktree_path) meta.appendChild(createNode("span", "", agent.worktree_path));
      if (agent.last_activity_at) {
        meta.appendChild(createNode("span", "", agent.last_activity_at));
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
