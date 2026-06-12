// SPEC-3038 US-3 — Close Guard: every window close (titlebar × and tab ×)
// runs through this confirmation modal regardless of agent state
// (user-confirmed decision, 2026-06-10). Mirrors the
// close-project-tab-confirm-modal.js conventions: pure renderer, shared
// `.modal-backdrop` + `.modal-shell` classes, focus-trap + focus restore,
// backdrop click / Esc cancel, Cancel as the default focus.
//
// state shape:
//   { open: bool, windowId: string|null, windowTitle: string,
//     agentLabel: string, runtimeLabel: string, running: bool }

import { createFocusTrap } from "./focus-trap.js";

const focusReturnMap = new WeakMap();
const focusTrapMap = new WeakMap();
const cancelHandlerMap = new WeakMap();

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

function attachCancelHandlers(modalEl, onCancel) {
  detachCancelHandlers(modalEl);
  const overlay = (event) => {
    // Only the backdrop itself cancels; clicks inside the dialog shell pass.
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

export function renderWindowCloseConfirmModal({
  modalEl,
  dialogEl,
  state,
  createNode,
  onCancel,
  onConfirm,
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

  const header = createNode("header", "window-close-confirm__header");
  const title = createNode("h2", "window-close-confirm__title", "Close window?");
  header.appendChild(title);
  if (state.windowTitle) {
    const subtitle = createNode(
      "p",
      "window-close-confirm__subtitle",
      state.windowTitle,
    );
    header.appendChild(subtitle);
  }
  dialogEl.appendChild(header);

  // Identity row — agent / surface label plus the live runtime state so the
  // user knows exactly what they are about to stop.
  if (state.agentLabel || state.runtimeLabel) {
    const meta = createNode("p", "window-close-confirm__meta");
    if (state.agentLabel) {
      meta.appendChild(
        createNode("span", "window-close-confirm__agent", state.agentLabel),
      );
    }
    if (state.runtimeLabel) {
      const stateChip = createNode(
        "span",
        "window-close-confirm__state",
        state.runtimeLabel,
      );
      if (state.running) {
        stateChip.classList.add("window-close-confirm__state--running");
      }
      meta.appendChild(stateChip);
    }
    dialogEl.appendChild(meta);
  }

  if (state.running) {
    dialogEl.appendChild(
      createNode(
        "p",
        "window-close-confirm__warning",
        "The running agent will be stopped and its session ends.",
      ),
    );
  }

  const footer = createNode("footer", "window-close-confirm__footer modal-footer");

  const cancelButton = createNode(
    "button",
    "text-button window-close-confirm__cancel",
    "Cancel",
  );
  cancelButton.type = "button";
  cancelButton.dataset.role = "window-close-cancel";
  cancelButton.addEventListener("click", () => onCancel());
  footer.appendChild(cancelButton);

  const confirmButton = createNode(
    "button",
    "wizard-button primary destructive window-close-confirm__confirm",
    "Close window",
  );
  confirmButton.type = "button";
  confirmButton.dataset.role = "window-close-confirm";
  confirmButton.addEventListener("click", () => onConfirm());
  footer.appendChild(confirmButton);

  dialogEl.appendChild(footer);

  // Default focus: Cancel (safe choice for a destructive flow).
  try {
    cancelButton.focus({ preventScroll: true });
  } catch {
    cancelButton.focus();
  }

  attachCancelHandlers(modalEl, onCancel);
}
