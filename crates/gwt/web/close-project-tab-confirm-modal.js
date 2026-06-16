// SPEC-2013 FR-012 / 2026-06-16 amendment: project tab の `×` は常に
// 確認 modal を表示する。agent preset (Agent / Claude / Codex もしくは
// agent_id 設定済み) で `WindowState::Running` の window が 1 個以上の
// ときだけ警告 copy + destructive confirm に切り替える。
// modal は既存 modal の `.modal-backdrop` + `.modal-shell`
// パターンを踏襲し、focus-trap + focus 復元 + overlay click + Esc で
// cancel をミラーする。
//
// 形状は branch-cleanup-modal.js / migration-modal.js に揃え、pure
// renderer + WeakMap-based focus 管理という慣習を共有する。
//
// state shape:
//   { open: bool, tabId: string|null, tabTitle: string|null,
//     runningAgents: Array<{ display_name: string, branch: string|null }> }

import { createFocusTrap } from "./focus-trap.js";

// SPEC-2356 — remember the element focused when the modal opens so we can
// restore focus to it on close. Mirrors branch-cleanup-modal.js.
const focusReturnMap = new WeakMap();
const focusTrapMap = new WeakMap();
const cancelHandlerMap = new WeakMap();

const MAX_AGENT_LIST = 3;

function appendAgentList(dialog, runningAgents, createNode) {
  const total = runningAgents.length;
  if (total === 0) {
    return;
  }
  const shown = runningAgents.slice(0, MAX_AGENT_LIST);

  const list = createNode("ul", "close-project-tab-modal__agent-list");
  for (const agent of shown) {
    const item = createNode("li", "close-project-tab-modal__agent-item");
    const name = createNode(
      "span",
      "close-project-tab-modal__agent-name",
      agent.display_name || "agent",
    );
    item.appendChild(name);
    if (agent.branch) {
      const branch = createNode(
        "span",
        "close-project-tab-modal__agent-branch",
        ` (${agent.branch})`,
      );
      item.appendChild(branch);
    }
    list.appendChild(item);
  }
  dialog.appendChild(list);

  if (total > shown.length) {
    const more = createNode(
      "p",
      "close-project-tab-modal__more",
      `and ${total - shown.length} more`,
    );
    dialog.appendChild(more);
  }
}

function detachCancelHandlers(modalEl) {
  const handlers = cancelHandlerMap.get(modalEl);
  if (!handlers) return;
  if (handlers.overlay) {
    modalEl.removeEventListener("click", handlers.overlay);
  }
  if (handlers.escape) {
    const ownerDoc = modalEl.ownerDocument;
    if (ownerDoc) {
      ownerDoc.removeEventListener("keydown", handlers.escape);
    }
  }
  cancelHandlerMap.delete(modalEl);
}

function attachCancelHandlers(modalEl, dialogEl, onCancel) {
  detachCancelHandlers(modalEl);
  const overlay = (event) => {
    // Only treat clicks on the backdrop element itself (not the dialog
    // shell or any descendant) as cancel. composedPath check keeps shell
    // interactions intact.
    if (event.target === modalEl) {
      onCancel();
    }
  };
  const escape = (event) => {
    if (event.key === "Escape") {
      onCancel();
    }
  };
  modalEl.addEventListener("click", overlay);
  const ownerDoc = modalEl.ownerDocument;
  if (ownerDoc) {
    ownerDoc.addEventListener("keydown", escape);
  }
  cancelHandlerMap.set(modalEl, { overlay, escape });
}

// Default copy is the SPEC-2013 project tab close; the agent-window close
// confirm (user verification 2026-06-12) reuses this renderer with its own
// modal element and copy overrides.
const DEFAULT_COPY = Object.freeze({
  title: "Close project tab?",
  summary: (runningAgents) =>
    runningAgents.length > 0
      ? `${runningAgents.length} running agent(s) will be stopped:`
      : "You can reopen it from Recent projects.",
  confirmLabel: (runningAgents) =>
    runningAgents.length > 0 ? "Close anyway" : "Close tab",
  destructive: (runningAgents) => runningAgents.length > 0,
});

export function renderCloseProjectTabConfirmModal({
  modalEl,
  dialogEl,
  state,
  createNode,
  onCancel,
  onConfirm,
  copy = DEFAULT_COPY,
}) {
  if (!modalEl || !dialogEl) {
    return;
  }
  const isOpenRequest = Boolean(state && state.open);

  if (!isOpenRequest) {
    const wasOpenBeforeClose = modalEl.classList.contains("open");
    modalEl.classList.remove("open");
    modalEl.setAttribute("aria-hidden", "true");
    while (dialogEl.firstChild) {
      dialogEl.removeChild(dialogEl.firstChild);
    }
    detachCancelHandlers(modalEl);
    if (wasOpenBeforeClose) {
      const releaseTrap = focusTrapMap.get(modalEl);
      focusTrapMap.delete(modalEl);
      if (typeof releaseTrap === "function") releaseTrap();
      const returnTo = focusReturnMap.get(modalEl);
      focusReturnMap.delete(modalEl);
      if (returnTo && typeof returnTo.focus === "function") {
        try {
          returnTo.focus({ preventScroll: true });
        } catch {
          returnTo.focus();
        }
      }
    }
    return;
  }

  const wasOpen = modalEl.classList.contains("open");
  if (!wasOpen) {
    const ownerDoc =
      modalEl.ownerDocument || (typeof document !== "undefined" ? document : null);
    if (ownerDoc) {
      focusReturnMap.set(modalEl, ownerDoc.activeElement);
      const release = createFocusTrap(dialogEl, { document: ownerDoc });
      focusTrapMap.set(modalEl, release);
    }
  }
  modalEl.classList.add("open");
  modalEl.removeAttribute("aria-hidden");

  while (dialogEl.firstChild) {
    dialogEl.removeChild(dialogEl.firstChild);
  }

  const header = createNode("header", "close-project-tab-modal__header");
  const title = createNode(
    "h2",
    "close-project-tab-modal__title",
    copy.title || DEFAULT_COPY.title,
  );
  header.appendChild(title);
  if (state.tabTitle) {
    const subtitle = createNode(
      "p",
      "close-project-tab-modal__subtitle",
      state.tabTitle,
    );
    header.appendChild(subtitle);
  }
  dialogEl.appendChild(header);

  const runningAgents = Array.isArray(state.runningAgents) ? state.runningAgents : [];
  const summaryText =
    typeof copy.summary === "function"
      ? copy.summary(runningAgents)
      : copy.summary || DEFAULT_COPY.summary(runningAgents);
  const summary = createNode(
    "p",
    "close-project-tab-modal__summary",
    summaryText,
  );
  dialogEl.appendChild(summary);

  appendAgentList(dialogEl, runningAgents, createNode);

  const footer = createNode("footer", "close-project-tab-modal__footer modal-footer");

  const cancelButton = createNode(
    "button",
    "text-button close-project-tab-modal__cancel",
    "Cancel",
  );
  cancelButton.type = "button";
  cancelButton.dataset.role = "close-project-tab-cancel";
  cancelButton.addEventListener("click", () => onCancel());
  footer.appendChild(cancelButton);

  const confirmLabel =
    typeof copy.confirmLabel === "function"
      ? copy.confirmLabel(runningAgents)
      : copy.confirmLabel || DEFAULT_COPY.confirmLabel(runningAgents);
  const destructive =
    typeof copy.destructive === "function"
      ? copy.destructive(runningAgents)
      : copy.destructive ?? DEFAULT_COPY.destructive(runningAgents);
  const confirmButton = createNode(
    "button",
    `wizard-button primary close-project-tab-modal__confirm${
      destructive ? " destructive" : ""
    }`,
    confirmLabel,
  );
  confirmButton.type = "button";
  confirmButton.dataset.role = "close-project-tab-confirm";
  confirmButton.addEventListener("click", () => onConfirm());
  footer.appendChild(confirmButton);

  dialogEl.appendChild(footer);

  // Default focus: Cancel button (safe choice for a destructive flow).
  try {
    cancelButton.focus({ preventScroll: true });
  } catch {
    cancelButton.focus();
  }

  attachCancelHandlers(modalEl, dialogEl, onCancel);
}
