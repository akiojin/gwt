// SPEC-1934 US-6: Migration confirmation / progress modal renderer.
// Mirrors the structure used by branch-cleanup-modal.js: a pure function over
// the dependency bag so app.js can wire DOM refs, state, and callbacks while
// the renderer stays unit-testable.

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
  onSkip,
  onQuit,
}) {
  if (!state || !state.migrationModal || !state.migrationModal.open) {
    modalEl.classList.remove("open");
    // SPEC-2356 — flip aria-hidden alongside the .open class so screen
    // readers stop announcing the dialog when it slides closed.
    modalEl.setAttribute("aria-hidden", "true");
    while (dialogEl.firstChild) {
      dialogEl.removeChild(dialogEl.firstChild);
    }
    return;
  }

  modalEl.classList.add("open");
  modalEl.removeAttribute("aria-hidden");
  while (dialogEl.firstChild) {
    dialogEl.removeChild(dialogEl.firstChild);
  }

  const m = state.migrationModal;

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
    dialogEl.appendChild(phaseLabel);
    const progress = createNode("progress", "migration-modal-progress");
    progress.setAttribute("max", "100");
    progress.setAttribute(
      "value",
      String(Number.isFinite(m.percent) ? m.percent : 0),
    );
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
    close.addEventListener("click", onSkip);
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

  const footer = createNode("div", "modal-footer");
  const quit = createNode("button", "wizard-button", "Quit");
  quit.type = "button";
  quit.addEventListener("click", onQuit);
  footer.appendChild(quit);

  const skip = createNode("button", "wizard-button", "Skip");
  skip.type = "button";
  skip.addEventListener("click", onSkip);
  footer.appendChild(skip);

  const migrate = createNode("button", "wizard-button primary", "Migrate");
  migrate.type = "button";
  migrate.disabled = Boolean(m.hasLocked);
  migrate.addEventListener("click", onMigrate);
  footer.appendChild(migrate);

  dialogEl.appendChild(footer);
}
