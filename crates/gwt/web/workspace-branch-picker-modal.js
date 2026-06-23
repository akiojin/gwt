// SPEC-2359 US-83 — Workspace "Open a branch…" picker modal.
//
// Rendered from the Workspace toolbar action. Lists the eligible existing
// remote branches (computed + filtered by the backend) so the user can start
// work on one. Picking a branch reuses the existing `open_launch_wizard`
// message, which opens the Launch Wizard in continue-on-branch mode: launching
// materializes a worktree tracking `origin/<branch>` without minting a new
// `work/*` branch. The branch behavior is already unified (the backend decides
// local-vs-origin on launch), so this surface abstracts the remote/local
// distinction — it shows plain branch names ("feature-foo") and a subtle
// "remote" hint instead of `origin/` refs.

import { createFocusTrap } from "./focus-trap.js";

// Launch-facing display: strip the `origin/` prefix so the user thinks in
// branch names, not remote refs. The picked value keeps the raw name (the
// backend `select_existing_branch` normalizes either form).
function displayBranchName(name) {
  return typeof name === "string" ? name.replace(/^origin\//, "") : name;
}

const focusReturnMap = new WeakMap();
const focusTrapMap = new WeakMap();

/**
 * Pure render primitive for the Workspace branch picker. `state` describes
 * whether the modal is open, the backend-supplied `branches` (remote ref
 * names), a loading flag, and an optional error. Smoke tests drive this
 * directly without launching `app.js`.
 */
export function renderWorkspaceBranchPicker({
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

  dialogEl.appendChild(createNode("h2", "workspace-branch-picker-title", "Open a branch"));
  dialogEl.appendChild(
    createNode(
      "p",
      "workspace-branch-picker-subtitle",
      "Continue work on an existing remote branch.",
    ),
  );

  const branches = Array.isArray(state.branches) ? state.branches : [];
  if (state.loading) {
    dialogEl.appendChild(
      createNode("div", "workspace-branch-picker-empty", "Loading branches…"),
    );
  } else if (branches.length === 0) {
    dialogEl.appendChild(
      createNode(
        "div",
        "workspace-branch-picker-empty",
        "No remote branches available to start work on.",
      ),
    );
  } else {
    const list = createNode("div", "workspace-branch-picker-list");
    for (const branch of branches) {
      const row = createNode("button", "workspace-branch-picker-row");
      row.type = "button";
      row.dataset.branchName = branch;
      row.appendChild(
        createNode("span", "workspace-branch-picker-row-name", displayBranchName(branch)),
      );
      row.appendChild(
        createNode("span", "workspace-branch-picker-row-tag", "remote"),
      );
      if (state.pendingBranch) {
        row.disabled = true;
        if (branch === state.pendingBranch) row.classList.add("is-pending");
      }
      row.addEventListener("click", (event) => {
        event.preventDefault();
        event.stopPropagation();
        if (typeof onPick === "function") onPick(branch);
      });
      list.appendChild(row);
    }
    dialogEl.appendChild(list);
  }

  if (state.error) {
    dialogEl.appendChild(
      createNode("div", "workspace-branch-picker-error", state.error),
    );
  }

  const actions = createNode("div", "workspace-branch-picker-actions");
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
 * Factory used by app.js. `open(windowId)` shows the modal in a loading state;
 * the caller sends `request_remote_start_work_branches` and the backend's
 * `remote_start_work_branches` event drives `handleBranchList`. Picking a
 * branch sends `open_launch_wizard` and closes the picker (the wizard takes
 * over the launch).
 */
export function createWorkspaceBranchPickerController({
  modalEl,
  dialogEl,
  createNode,
  send,
}) {
  const state = {
    open: false,
    branches: [],
    windowId: null,
    loading: false,
    error: "",
    pendingBranch: null,
  };

  function close() {
    state.open = false;
    state.branches = [];
    state.windowId = null;
    state.loading = false;
    state.error = "";
    state.pendingBranch = null;
    render();
  }

  function pick(branch) {
    if (!branch || state.pendingBranch) return;
    state.error = "";
    if (typeof send === "function" && state.windowId) {
      // Reuse the existing per-branch launch path. open_launch_wizard opens the
      // wizard in continue-on-branch mode for branch_name; launching then runs
      // create_from_remote (worktree tracking origin/<branch>, no work/*).
      send({
        kind: "open_launch_wizard",
        id: state.windowId,
        branch_name: branch,
      });
    }
    // The wizard takes over; close the picker.
    close();
  }

  function render() {
    renderWorkspaceBranchPicker({
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
    open: (windowId) => {
      state.open = true;
      state.branches = [];
      state.windowId = windowId ?? null;
      state.loading = true;
      state.error = "";
      state.pendingBranch = null;
      render();
    },
    handleBranchList: (event) => {
      if (!state.open) return;
      // Only apply the response addressed to the window that opened the picker.
      if (event?.id && state.windowId && event.id !== state.windowId) return;
      state.branches = Array.isArray(event?.branches) ? event.branches : [];
      state.loading = false;
      render();
    },
    dismiss: close,
    render,
  };
}
