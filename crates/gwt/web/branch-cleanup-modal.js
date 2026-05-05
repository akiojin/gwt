// SPEC-2008 FR-035 follow-up: Branch Cleanup modal renderer extracted from
// app.js so it can be exercised by a DOM smoke test (see
// crates/gwt/web/__tests__/branch-cleanup.smoke.test.mjs). The renderer is a
// pure function over the dependency bag; app.js wires the DOM refs, state
// lookup, helpers, and callbacks.

export function renderBranchCleanupModal({
  modalEl,
  dialogEl,
  windowId,
  state,
  selectedEntries,
  createNode,
  resultSummary,
  mergeTargetText,
  riskLabels,
  onCancel,
  onSubmit,
  onDeleteRemoteToggle,
}) {
  if (!windowId || !state || !state.cleanupModal.open) {
    modalEl.classList.remove("open");
    // SPEC-2356 — flip aria-hidden alongside the .open class so screen
    // readers stop announcing the dialog when it slides closed.
    modalEl.setAttribute("aria-hidden", "true");
    while (dialogEl.firstChild) {
      dialogEl.removeChild(dialogEl.firstChild);
    }
    return;
  }

  const supportsRemoteDelete = selectedEntries.some((entry) =>
    Boolean(entry.cleanup.upstream),
  );
  modalEl.classList.add("open");
  modalEl.removeAttribute("aria-hidden");
  while (dialogEl.firstChild) {
    dialogEl.removeChild(dialogEl.firstChild);
  }

  if (state.cleanupModal.stage === "running") {
    dialogEl.appendChild(createNode("h2", "", "Cleaning up branches"));
    const count = Math.max(selectedEntries.length, 1);
    dialogEl.appendChild(
      createNode(
        "div",
        "branch-cleanup-running",
        `Running cleanup for ${count} branch${count === 1 ? "" : "es"}.`,
      ),
    );
    return;
  }

  if (state.cleanupModal.stage === "result") {
    dialogEl.appendChild(createNode("h2", "", "Cleanup result"));
    dialogEl.appendChild(
      createNode(
        "div",
        "branch-cleanup-results-summary",
        resultSummary(state.cleanupModal.results),
      ),
    );
    const resultList = createNode("div", "branch-cleanup-list");
    for (const result of state.cleanupModal.results || []) {
      const item = createNode("div", "branch-cleanup-item");
      const header = createNode("div", "branch-cleanup-item-header");
      header.appendChild(
        createNode("div", "branch-cleanup-item-title", result.branch),
      );
      const statusClass =
        result.status === "partial"
          ? "risky"
          : result.status === "failed"
            ? "blocked"
            : "safe";
      header.appendChild(
        createNode(
          "span",
          `branch-cleanup-badge ${statusClass}`,
          result.status,
        ),
      );
      item.appendChild(header);
      item.appendChild(
        createNode("div", "branch-cleanup-item-copy", result.message),
      );
      if (
        result.execution_branch &&
        result.execution_branch !== result.branch
      ) {
        item.appendChild(
          createNode(
            "div",
            "branch-cleanup-item-copy",
            `Executed as ${result.execution_branch}`,
          ),
        );
      }
      resultList.appendChild(item);
    }
    dialogEl.appendChild(resultList);
    const footer = createNode("div", "modal-footer");
    const close = createNode("button", "wizard-button primary", "Close");
    close.type = "button";
    close.addEventListener("click", onCancel);
    footer.appendChild(close);
    dialogEl.appendChild(footer);
    return;
  }

  dialogEl.appendChild(createNode("h2", "", "Clean up branches"));
  dialogEl.appendChild(
    createNode(
      "div",
      "branch-cleanup-results-summary",
      `Delete ${selectedEntries.length} selected branch${selectedEntries.length === 1 ? "" : "es"}.`,
    ),
  );
  const list = createNode("div", "branch-cleanup-list");
  for (const entry of selectedEntries) {
    const item = createNode("div", "branch-cleanup-item");
    const header = createNode("div", "branch-cleanup-item-header");
    header.appendChild(createNode("div", "branch-cleanup-item-title", entry.name));
    header.appendChild(
      createNode(
        "span",
        `branch-cleanup-badge ${entry.cleanup.availability}`,
        entry.cleanup.availability,
      ),
    );
    item.appendChild(header);
    const target = mergeTargetText(entry.cleanup.merge_target) || "not merged";
    item.appendChild(createNode("div", "branch-cleanup-item-copy", target));
    if (
      entry.cleanup.execution_branch &&
      entry.cleanup.execution_branch !== entry.name
    ) {
      item.appendChild(
        createNode(
          "div",
          "branch-cleanup-item-copy",
          `Executed as ${entry.cleanup.execution_branch}`,
        ),
      );
    }
    const risks = riskLabels(entry.cleanup.risks);
    if (risks.length > 0) {
      item.appendChild(
        createNode("div", "branch-cleanup-item-copy", risks.join(", ")),
      );
    }
    list.appendChild(item);
  }
  dialogEl.appendChild(list);
  if (supportsRemoteDelete) {
    const toggleRow = createNode("label", "branch-cleanup-toggle-row");
    const checkbox = createNode("input", "");
    checkbox.type = "checkbox";
    checkbox.checked = Boolean(state.cleanupModal.deleteRemote);
    checkbox.addEventListener("change", () => {
      onDeleteRemoteToggle(checkbox.checked);
    });
    toggleRow.appendChild(checkbox);
    toggleRow.appendChild(
      createNode("span", "", "Also delete matching remote branches"),
    );
    dialogEl.appendChild(toggleRow);
  }
  const footer = createNode("div", "modal-footer");
  const cancel = createNode("button", "wizard-button", "Cancel");
  cancel.type = "button";
  cancel.addEventListener("click", onCancel);
  footer.appendChild(cancel);
  const submit = createNode("button", "wizard-button primary", "Run cleanup");
  submit.type = "button";
  submit.addEventListener("click", onSubmit);
  footer.appendChild(submit);
  dialogEl.appendChild(footer);
}
