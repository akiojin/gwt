// SPEC-1934 US-6: Migration confirmation / progress modal renderer.
// Mirrors the structure used by branch-cleanup-modal.js: a pure function over
// the dependency bag so app.js can wire DOM refs, state, and callbacks while
// the renderer stays unit-testable.

import { createFocusTrap } from "./focus-trap.js";

// SPEC-2356 — remember the trigger that opened the modal so close can
// restore focus there. WeakMap keyed on modal element matches the
// branch-cleanup-modal pattern.
const focusReturnMap = new WeakMap();
// SPEC-2356 — track the active focus trap release so Tab cycles within
// the modal and detach the listener on close.
const focusTrapMap = new WeakMap();

const PHASE_LABELS = {
  confirm: "Preparing",
  validate: "Validating workspace",
  backup: "Creating backup",
  bareify: "Building bare repository",
  worktrees: "Setting up worktrees",
  submodules: "Restoring submodules",
  tracking: "Restoring upstream tracking",
  cleanup: "Cleaning up",
  done: "Done",
  error: "Error",
  rolled_back: "Rolled back",
};

function describePhase(phase) {
  if (!phase) return PHASE_LABELS.confirm;
  return PHASE_LABELS[phase] || phase;
}

function describeRecovery(recovery) {
  switch (recovery) {
    case "untouched":
      return "No changes were made to your repository.";
    case "rolled_back":
      return "All partial changes were rolled back. The original layout was restored.";
    case "partial":
      return "Some changes could not be rolled back automatically. Inspect the migration backup directory before continuing.";
    default:
      return "";
  }
}

export function renderMigrationModal({
  modalEl,
  dialogEl,
  state,
  createNode,
  onMigrate,
  // SPEC-1934 US-7 / FR-032: the confirmation modal is Accept-only; closing
  // the OS window remains the sole abort path. `onClose` is still used by the
  // error stage so the user can dismiss a failure dialog without sending a
  // skip event to the backend.
  onClose,
}) {
  if (!state || !state.migrationModal || !state.migrationModal.open) {
    const wasOpenBeforeClose = modalEl.classList.contains("open");
    modalEl.classList.remove("open");
    // SPEC-2356 — flip aria-hidden alongside the .open class so screen
    // readers stop announcing the dialog when it slides closed.
    modalEl.setAttribute("aria-hidden", "true");
    while (dialogEl.firstChild) {
      dialogEl.removeChild(dialogEl.firstChild);
    }
    // SPEC-2356 — restore focus to whatever was focused when the modal
    // opened so keyboard users don't lose their place.
    if (wasOpenBeforeClose) {
      // Release the focus trap before restoring focus.
      const releaseTrap = focusTrapMap.get(modalEl);
      focusTrapMap.delete(modalEl);
      if (typeof releaseTrap === "function") releaseTrap();

      const returnTo = focusReturnMap.get(modalEl);
      focusReturnMap.delete(modalEl);
      if (returnTo && typeof returnTo.focus === "function") {
        try { returnTo.focus({ preventScroll: true }); } catch { returnTo.focus(); }
      }
    }
    return;
  }

  // SPEC-2356 — capture fresh-open transition so we can move focus into
  // the dialog only on the initial render (subsequent re-renders during
  // running / done stages keep focus where the user already navigated).
  const wasOpen = modalEl.classList.contains("open");
  if (!wasOpen) {
    const ownerDoc = modalEl.ownerDocument || (typeof document !== "undefined" ? document : null);
    if (ownerDoc) {
      focusReturnMap.set(modalEl, ownerDoc.activeElement);
      // Activate focus trap so Tab cycles within the modal.
      const release = createFocusTrap(dialogEl, { document: ownerDoc });
      focusTrapMap.set(modalEl, release);
    }
  }
  modalEl.classList.add("open");
  modalEl.removeAttribute("aria-hidden");

  const m = state.migrationModal;

  // SPEC-2356 — aria-busy signals to screen readers that the dialog is in
  // a loading state during the async migration. Set true during running,
  // false otherwise.
  if (m.stage === "running") {
    dialogEl.setAttribute("aria-busy", "true");
  } else {
    dialogEl.setAttribute("aria-busy", "false");
  }

  while (dialogEl.firstChild) {
    dialogEl.removeChild(dialogEl.firstChild);
  }

  if (m.stage === "running") {
    dialogEl.appendChild(createNode("h2", "", "Migrating repository"));
    dialogEl.appendChild(
      createNode("div", "migration-modal-subtitle", m.projectRoot || ""),
    );
    const phaseLabel = createNode(
      "div",
      "migration-modal-phase-label",
      describePhase(m.phase),
    );
    // SPEC-2356 — give the phase label a stable id so the <progress>
    // element can reference it via aria-labelledby. Without this, screen
    // readers announce just "progressbar 50%" with no context about
    // which phase is running.
    phaseLabel.id = "migration-modal-phase-label";
    dialogEl.appendChild(phaseLabel);
    const progress = createNode("progress", "migration-modal-progress");
    progress.setAttribute("max", "100");
    progress.setAttribute(
      "value",
      String(Number.isFinite(m.percent) ? m.percent : 0),
    );
    progress.setAttribute("aria-labelledby", "migration-modal-phase-label");
    dialogEl.appendChild(progress);
    dialogEl.appendChild(
      createNode(
        "div",
        "migration-modal-hint",
        "A backup is being kept under .gwt-migration-backup/ until the run succeeds.",
      ),
    );
    return;
  }

  if (m.stage === "error") {
    dialogEl.appendChild(createNode("h2", "", "Migration failed"));
    dialogEl.appendChild(
      createNode(
        "div",
        "migration-modal-subtitle",
        `Phase: ${describePhase(m.phase)}`,
      ),
    );
    dialogEl.appendChild(
      createNode("div", "migration-modal-error-message", m.message || ""),
    );
    const recoveryText = describeRecovery(m.recovery);
    if (recoveryText) {
      dialogEl.appendChild(
        createNode("div", "migration-modal-recovery", recoveryText),
      );
    }
    const footer = createNode("div", "modal-footer");
    const close = createNode("button", "wizard-button primary", "Close");
    close.type = "button";
    // SPEC-1934 US-7 / FR-032: dismiss the error UI without sending a skip
    // event; the tab remains migration_pending so the Accept-only modal can
    // be re-presented on the next Open Project.
    close.addEventListener("click", onClose);
    footer.appendChild(close);
    dialogEl.appendChild(footer);
    return;
  }

  // confirm stage (default)
  dialogEl.appendChild(
    createNode("h2", "", "Migrate to Bare + Worktree layout"),
  );
  dialogEl.appendChild(
    createNode("div", "migration-modal-subtitle", m.projectRoot || ""),
  );
  dialogEl.appendChild(
    createNode(
      "p",
      "migration-modal-body",
      "gwt manages each project as <project>.git/ + <branch>/. This repository is currently a Normal Git checkout. Migrate to keep it consistent with the rest of your workspace.",
    ),
  );

  const detailList = createNode("ul", "migration-modal-details");
  if (m.branch) {
    detailList.appendChild(
      createNode("li", "", `Active branch: ${m.branch}`),
    );
  }
  if (m.hasDirty) {
    detailList.appendChild(
      createNode(
        "li",
        "",
        "Uncommitted changes detected — they will be evacuated and restored.",
      ),
    );
  }
  if (m.hasSubmodules) {
    detailList.appendChild(
      createNode(
        "li",
        "",
        "Submodules detected — `git submodule update --init --recursive` will run after the migration.",
      ),
    );
  }
  if (m.hasLocked) {
    detailList.appendChild(
      createNode(
        "li",
        "blocked",
        "Locked worktree detected — migration cannot proceed until it is unlocked.",
      ),
    );
  }
  if (detailList.childNodes.length > 0) {
    dialogEl.appendChild(detailList);
  }

  dialogEl.appendChild(
    createNode(
      "p",
      "migration-modal-hint",
      "A full snapshot will be saved to .gwt-migration-backup/ before any change. On any failure the original layout is restored automatically.",
    ),
  );

  // SPEC-1934 US-7 / FR-032 / SC-024: the confirm stage offers Accept only.
  // Skip / Quit / Cancel buttons are intentionally not rendered — closing
  // the OS window remains the sole abort path so a migration_pending tab can
  // never proceed to Start Work / Launch / Materialize while the layout is
  // still Normal Git.
  const footer = createNode("div", "modal-footer");
  const migrate = createNode("button", "wizard-button primary", "Migrate");
  migrate.type = "button";
  migrate.disabled = Boolean(m.hasLocked);
  migrate.addEventListener("click", onMigrate);
  footer.appendChild(migrate);

  dialogEl.appendChild(footer);
  // SPEC-2356 — on a fresh open, move focus to the dialog so screen readers
  // announce "Worktree migration dialog" and keyboard users land inside.
  if (!wasOpen && typeof dialogEl.focus === "function") {
    try { dialogEl.focus({ preventScroll: true }); } catch { dialogEl.focus(); }
  }
}
