// Recovery Center — attention-first recovery candidate picker.
//
// The backend owns eligibility and recovery execution. This module only
// renders recovery evidence, emits semantic action requests, and keeps one
// action pending until an explicit backend result arrives. Startup uses the
// attention-only view; a user-opened center can review every candidate.

import { createFocusTrap } from "./focus-trap.js";

const focusReturnMap = new WeakMap();
const focusTrapMap = new WeakMap();
const dismissHandlerMap = new WeakMap();

export const RECOVERY_CENTER_ACTIONS = Object.freeze([
  Object.freeze({ action: "focus", label: "Focus" }),
  Object.freeze({ action: "confirm_resume", label: "Confirm & Resume" }),
  Object.freeze({ action: "continue_checkpoint", label: "Continue Checkpoint" }),
  Object.freeze({ action: "start_fresh", label: "Start Fresh" }),
  Object.freeze({ action: "open_board", label: "Open Board" }),
  Object.freeze({ action: "details", label: "Details" }),
  Object.freeze({ action: "discard", label: "Discard", destructive: true }),
]);

function defaultCreateNode(documentRef) {
  return (tag, className, text) => {
    const node = documentRef.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
}

function asText(value, fallback = "") {
  if (value === null || value === undefined) return fallback;
  const text = String(value).trim();
  return text || fallback;
}

function candidateActionHandle(candidate) {
  return asText(candidate?.action_handle ?? candidate?.actionHandle);
}

function requiresAttention(candidate) {
  const explicit = candidate?.attention_required ?? candidate?.requires_attention;
  return explicit === undefined ? true : Boolean(explicit);
}

function kindLabel(kind) {
  return asText(kind).toLowerCase() === "intake" ? "Intake" : "Execution";
}

function coverageLabel(coverage) {
  if (typeof coverage === "string" || typeof coverage === "number") {
    return asText(coverage, "Unknown");
  }
  if (coverage && typeof coverage === "object") {
    const captured = Number(coverage.captured);
    const total = Number(coverage.total);
    if (Number.isFinite(captured) && Number.isFinite(total)) {
      return `${captured} of ${total} captured`;
    }
    const percent = Number(coverage.percent);
    if (Number.isFinite(percent)) return `${percent}% captured`;
  }
  return "Unknown";
}

function captureHealth(candidate) {
  const raw = asText(candidate?.capture_health ?? candidate?.captureHealth, "unknown")
    .toLowerCase();
  const known = ["healthy", "degraded", "unavailable"];
  const status = known.includes(raw) ? raw : "unknown";
  return {
    status,
    label: `Capture ${status}`,
  };
}

function boardPendingLabel(value) {
  if (typeof value === "number") {
    if (value <= 0) return "Board synced";
    return `${value} Board update${value === 1 ? "" : "s"} pending`;
  }
  if (value === true) return "Board update pending";
  if (value === false || value === null || value === undefined) return "Board synced";
  return asText(value, "Board status unknown");
}

function exactEvidence(candidate) {
  const raw = candidate?.exact_available ?? candidate?.exactAvailable;
  const available = raw === true || raw === "available" || raw === "exact";
  const unavailable = raw === false || raw === "unavailable";
  const ambiguity = candidate?.exact_ambiguous ?? candidate?.exactAmbiguous;
  return {
    status: available ? "available" : unavailable ? "unavailable" : "unknown",
    label: available ? "Exact available" : unavailable ? "Exact unavailable" : "Exact unknown",
    ambiguity:
      typeof ambiguity === "string"
        ? asText(ambiguity)
        : ambiguity
          ? "Exact match is ambiguous"
          : "",
  };
}

function providerChoices(candidate) {
  const raw = candidate?.provider_choices ?? candidate?.providerChoices;
  if (!Array.isArray(raw)) return [];
  return raw
    .map((candidateRoot) => ({
      rootId: asText(candidateRoot?.choice_handle ?? candidateRoot?.choiceHandle),
      label: asText(candidateRoot?.label, "Provider candidate"),
      evidenceCount: Number(candidateRoot?.evidence_count ?? candidateRoot?.evidenceCount ?? 0),
    }))
    .filter((candidateRoot) => candidateRoot.rootId);
}

function needsProviderChoiceSelection(candidate) {
  return providerChoices(candidate).length > 0;
}

function clearChildren(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
}

function releaseModal(modalEl, dialogEl) {
  const handlers = dismissHandlerMap.get(modalEl);
  if (handlers) {
    modalEl.removeEventListener("click", handlers.backdrop);
    handlers.document?.removeEventListener("keydown", handlers.escape);
    dismissHandlerMap.delete(modalEl);
  }
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
  clearChildren(dialogEl);
}

function attachDismissHandlers(modalEl, ownerDocument, onClose, onEscape) {
  const backdrop = (event) => {
    if (event.target === modalEl && typeof onClose === "function") onClose();
  };
  const escape = (event) => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    if (typeof onEscape === "function") onEscape();
    else if (typeof onClose === "function") onClose();
  };
  modalEl.addEventListener("click", backdrop);
  ownerDocument?.addEventListener("keydown", escape);
  dismissHandlerMap.set(modalEl, { backdrop, escape, document: ownerDocument });
}

function appendEvidence(createNode, container, label, value) {
  const item = createNode("div", "recovery-center-evidence");
  item.appendChild(createNode("span", "recovery-center-evidence__label", label));
  item.appendChild(createNode("span", "recovery-center-evidence__value", value));
  container.appendChild(item);
}

function appendCandidateRow({
  createNode,
  list,
  candidate,
  selected,
  pending,
  onSelect,
}) {
  const id = candidateActionHandle(candidate);
  const row = createNode(
    "button",
    `recovery-center-row${selected ? " is-selected" : ""}${pending ? " is-pending" : ""}`,
  );
  row.type = "button";
  row.dataset.actionHandle = id;
  row.setAttribute("role", "option");
  row.setAttribute("aria-selected", selected ? "true" : "false");
  row.disabled = Boolean(pending);

  const heading = createNode("div", "recovery-center-row__heading");
  const identity = createNode("div", "recovery-center-row__identity");
  identity.appendChild(
    createNode(
      "span",
      "recovery-center-row__title",
      asText(candidate.purpose_preview ?? candidate.purposePreview, "Recovery candidate"),
    ),
  );
  identity.appendChild(
    createNode("span", "recovery-center-chip recovery-center-chip--kind", kindLabel(candidate.kind)),
  );
  heading.appendChild(identity);
  heading.appendChild(
    createNode(
      "span",
      "recovery-center-row__provider",
      asText(candidate.provider, "Unknown provider"),
    ),
  );
  row.appendChild(heading);

  const evidence = createNode("div", "recovery-center-row__evidence");
  appendEvidence(
    createNode,
    evidence,
    "Worktree",
    asText(candidate.worktree_name ?? candidate.worktreeName, "Unavailable"),
  );
  appendEvidence(
    createNode,
    evidence,
    "Last checkpoint",
    asText(candidate.last_checkpoint_at ?? candidate.lastCheckpointAt, "None"),
  );
  appendEvidence(createNode, evidence, "Coverage", coverageLabel(candidate.coverage));
  row.appendChild(evidence);

  const signals = createNode("div", "recovery-center-row__signals");
  const health = captureHealth(candidate);
  signals.appendChild(
    createNode(
      "span",
      `recovery-center-chip recovery-center-chip--${health.status}`,
      health.label,
    ),
  );
  signals.appendChild(
    createNode(
      "span",
      "recovery-center-chip recovery-center-chip--board",
      boardPendingLabel(candidate.board_pending ?? candidate.boardPending),
    ),
  );
  const boardDeliveryFailed = Boolean(
    candidate.board_delivery_failed ?? candidate.boardDeliveryFailed,
  );
  if (boardDeliveryFailed) {
    const deliveryChip = createNode(
      "span",
      "recovery-center-chip recovery-center-chip--board-error",
      "Board delivery needs attention",
    );
    deliveryChip.title = "A Board update could not be delivered.";
    signals.appendChild(deliveryChip);
  }
  const exact = exactEvidence(candidate);
  signals.appendChild(
    createNode(
      "span",
      `recovery-center-chip recovery-center-chip--exact-${exact.status}`,
      exact.label,
    ),
  );
  if (exact.ambiguity) {
    signals.appendChild(
      createNode(
        "span",
        "recovery-center-chip recovery-center-chip--ambiguous",
        exact.ambiguity,
      ),
    );
  }
  row.appendChild(signals);

  row.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (!pending && typeof onSelect === "function") onSelect(id);
  });
  list.appendChild(row);
}

function appendSelectedDetails(createNode, container, candidate) {
  const details = candidate?.details;
  if (!details || typeof details !== "object" || Array.isArray(details)) return;
  const entries = Object.entries(details).filter(([, value]) => value !== null && value !== undefined);
  if (entries.length === 0) return;
  const detailList = createNode("dl", "recovery-center-details");
  for (const [label, value] of entries) {
    detailList.appendChild(createNode("dt", "recovery-center-details__label", asText(label)));
    detailList.appendChild(createNode("dd", "recovery-center-details__value", asText(value)));
  }
  container.appendChild(detailList);
}

function appendProviderChoiceSelection({
  createNode,
  container,
  candidate,
  selectedProviderChoiceHandle,
  pending,
  onSelect,
}) {
  if (!needsProviderChoiceSelection(candidate)) return;
  const roots = providerChoices(candidate);
  const fieldset = createNode("fieldset", "recovery-center-provider-roots");
  fieldset.disabled = Boolean(pending);
  fieldset.appendChild(
    createNode("legend", "recovery-center-provider-roots__title", "Choose provider session"),
  );
  fieldset.appendChild(
    createNode(
      "p",
      "recovery-center-provider-roots__help",
      "Select the exact provider session supported by the recovery evidence.",
    ),
  );
  roots.forEach((root, index) => {
    const label = createNode("label", "recovery-center-provider-root");
    const input = createNode("input", "recovery-center-provider-root__input");
    input.type = "radio";
    input.name = "recovery-provider-choice";
    input.value = root.rootId;
    input.checked = root.rootId === selectedProviderChoiceHandle;
    input.disabled = Boolean(pending);
    input.addEventListener("click", (event) => {
      event.stopPropagation();
      if (!pending && typeof onSelect === "function") onSelect(root.rootId);
    });
    label.appendChild(input);
    const copy = createNode("span", "recovery-center-provider-root__copy");
    copy.appendChild(
      createNode("span", "recovery-center-provider-root__id", root.label),
    );
    copy.appendChild(
      createNode(
        "span",
        "recovery-center-provider-root__evidence",
        root.evidenceCount > 0
          ? `${root.evidenceCount} recorded evidence signal${root.evidenceCount === 1 ? "" : "s"}`
          : "Recorded recovery candidate",
      ),
    );
    label.appendChild(copy);
    fieldset.appendChild(label);
  });
  container.appendChild(fieldset);
}

function appendActions({
  createNode,
  container,
  state,
  selected,
  selectedProviderChoiceHandle,
  onAction,
}) {
  const actions = createNode("div", "recovery-center-actions");
  actions.setAttribute("aria-label", "Recovery actions");
  const advertisedActions = selected?.available_actions ?? selected?.availableActions;
  const eligibleActions = Array.isArray(advertisedActions)
    ? new Set(advertisedActions.map((action) => asText(action)))
    : null;
  for (const definition of RECOVERY_CENTER_ACTIONS) {
    const button = createNode(
      "button",
      `wizard-button recovery-center-action${definition.destructive ? " destructive" : ""}`,
      definition.label,
    );
    button.type = "button";
    button.dataset.recoveryAction = definition.action;
    button.disabled =
      Boolean(state.pending) ||
      !selected ||
      (definition.action === "confirm_resume" &&
        needsProviderChoiceSelection(selected) &&
        !selectedProviderChoiceHandle) ||
      (eligibleActions !== null && !eligibleActions.has(definition.action));
    if (state.pending?.action === definition.action) button.classList.add("is-pending");
    button.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      if (!button.disabled && typeof onAction === "function") onAction(definition.action);
    });
    actions.appendChild(button);
  }
  container.appendChild(actions);
}

function appendDiscardConfirmation({ createNode, container, pending, onCancel, onConfirm }) {
  const panel = createNode("section", "recovery-center-discard-confirm");
  panel.setAttribute("role", "alertdialog");
  panel.setAttribute("aria-label", "Confirm discard recovery candidate");
  panel.appendChild(
    createNode("h3", "recovery-center-discard-confirm__title", "Discard this recovery candidate?"),
  );
  panel.appendChild(
    createNode(
      "p",
      "recovery-center-discard-confirm__copy",
      "This removes the recovery record. User work is never deleted by this screen.",
    ),
  );
  const actions = createNode("div", "recovery-center-discard-confirm__actions");
  const cancel = createNode("button", "wizard-button", "Cancel");
  cancel.type = "button";
  cancel.disabled = Boolean(pending);
  cancel.addEventListener("click", onCancel);
  actions.appendChild(cancel);
  const confirm = createNode("button", "wizard-button destructive", "Confirm Discard");
  confirm.type = "button";
  confirm.dataset.role = "confirm-discard";
  confirm.disabled = Boolean(pending);
  confirm.addEventListener("click", onConfirm);
  actions.appendChild(confirm);
  panel.appendChild(actions);
  container.appendChild(panel);
  if (!pending && typeof cancel.focus === "function") {
    try {
      cancel.focus({ preventScroll: true });
    } catch {
      cancel.focus();
    }
  }
}

export function renderRecoveryCenter({
  modalEl,
  dialogEl,
  state,
  createNode,
  onClose,
  onEscape,
  onSelect,
  onProviderChoiceSelect,
  onAction,
  onCancelDiscard,
  onConfirmDiscard,
}) {
  if (!modalEl || !dialogEl) return;
  if (!state?.open) {
    const wasOpen = modalEl.classList.contains("open");
    modalEl.classList.remove("open");
    modalEl.setAttribute("aria-hidden", "true");
    dialogEl.setAttribute("aria-busy", "false");
    if (wasOpen) releaseModal(modalEl, dialogEl);
    else clearChildren(dialogEl);
    return;
  }

  const ownerDocument = modalEl.ownerDocument || globalThis.document || null;
  const node = createNode || defaultCreateNode(ownerDocument);
  const wasOpen = modalEl.classList.contains("open");
  if (!wasOpen) {
    if (ownerDocument) {
      focusReturnMap.set(modalEl, ownerDocument.activeElement);
      focusTrapMap.set(modalEl, createFocusTrap(dialogEl, { document: ownerDocument }));
    }
    attachDismissHandlers(modalEl, ownerDocument, onClose, onEscape);
  }

  modalEl.classList.add("open");
  modalEl.removeAttribute("aria-hidden");
  dialogEl.setAttribute("aria-busy", state.pending ? "true" : "false");
  clearChildren(dialogEl);

  const available = Array.isArray(state.candidates) ? state.candidates : [];
  const total = Array.isArray(state.sourceCandidates)
    ? state.sourceCandidates.length
    : available.length;
  const header = node("header", "recovery-center-header");
  const titles = node("div", "recovery-center-header__titles");
  const title = node("h2", "recovery-center-title", "Recovery Center");
  title.id = "recovery-center-title";
  titles.appendChild(title);
  titles.appendChild(
    node(
      "p",
      "recovery-center-subtitle",
      state.attentionOnly
        ? `${available.length} need attention · ${total} recoverable total. Exact restores continue automatically.`
        : `${total} recoverable session${total === 1 ? "" : "s"}. Review recoverable sessions and choose how to continue.`,
    ),
  );
  header.appendChild(titles);
  const close = node("button", "text-button recovery-center-close", "Close");
  close.type = "button";
  close.dataset.role = "recovery-center-close";
  close.addEventListener("click", onClose);
  header.appendChild(close);
  dialogEl.appendChild(header);

  if (available.length === 0) {
    const empty = node("div", "recovery-center-empty");
    empty.appendChild(
      node(
        "strong",
        "recovery-center-empty__title",
        state.attentionOnly ? "No sessions need attention" : "No recoverable sessions",
      ),
    );
    empty.appendChild(
      node(
        "span",
        "recovery-center-empty__copy",
        state.attentionOnly
          ? "Exact recoveries run automatically and appear in the normal workspace history."
          : "Recovery records will appear here when a session can be continued.",
      ),
    );
    dialogEl.appendChild(empty);
  } else {
    const layout = node("div", "recovery-center-layout");
    const list = node("div", "recovery-center-list");
    list.setAttribute("role", "listbox");
    list.setAttribute(
      "aria-label",
      state.attentionOnly ? "Sessions requiring attention" : "Recoverable sessions",
    );
    for (const candidate of available) {
      const id = candidateActionHandle(candidate);
      appendCandidateRow({
        createNode: node,
        list,
        candidate,
        selected: id === state.selectedId,
        pending: Boolean(state.pending),
        onSelect,
      });
    }
    layout.appendChild(list);

    const selected = available.find(
      (candidate) => candidateActionHandle(candidate) === state.selectedId,
    );
    const decision = node("section", "recovery-center-decision");
    decision.appendChild(
      node(
        "h3",
        "recovery-center-decision__title",
        selected
          ? asText(selected.purpose_preview ?? selected.purposePreview, "Recovery candidate")
          : "Select a session",
      ),
    );
    if (selected) appendSelectedDetails(node, decision, selected);
    const selectedProviderChoiceHandle = selected
      ? state.providerChoiceSelections.get(candidateActionHandle(selected)) || ""
      : "";
    if (selected) {
      appendProviderChoiceSelection({
        createNode: node,
        container: decision,
        candidate: selected,
        selectedProviderChoiceHandle,
        pending: state.pending,
        onSelect: onProviderChoiceSelect,
      });
    }
    appendActions({
      createNode: node,
      container: decision,
      state,
      selected,
      selectedProviderChoiceHandle,
      onAction,
    });
    if (state.pending) {
      const pending = node(
        "div",
        "recovery-center-pending",
        "Working… waiting for the recovery service.",
      );
      pending.setAttribute("role", "status");
      pending.setAttribute("aria-live", "polite");
      decision.appendChild(pending);
    }
    if (state.confirmDiscardId === state.selectedId && selected) {
      appendDiscardConfirmation({
        createNode: node,
        container: decision,
        pending: state.pending,
        onCancel: onCancelDiscard,
        onConfirm: onConfirmDiscard,
      });
    }
    if (state.error) {
      const error = node("div", "recovery-center-error", state.error);
      error.setAttribute("role", "alert");
      decision.appendChild(error);
    }
    layout.appendChild(decision);
    dialogEl.appendChild(layout);
  }

  if (!wasOpen && typeof close.focus === "function") {
    try {
      close.focus({ preventScroll: true });
    } catch {
      close.focus();
    }
  }
}

/**
 * Creates the stateful Recovery Center bridge used by app.js.
 *
 * `open(candidates)` is the user-requested history view and keeps every valid
 * candidate. `openAttention(candidates)` is the startup path and keeps only
 * candidates whose `attention_required` flag is true. `sendAction` receives
 * one `{ actionHandle, action }` request and the controller remains pending
 * until `handleActionResult` receives the matching result.
 */
export function createRecoveryCenterController({
  modalEl,
  dialogEl,
  createNode,
  sendAction,
}) {
  const state = {
    open: false,
    sourceCandidates: [],
    candidates: [],
    attentionOnly: false,
    selectedId: null,
    pending: null,
    confirmDiscardId: null,
    error: "",
    providerChoiceSelections: new Map(),
    focusAfterRender: null,
  };

  function focusElement(element) {
    if (!element || typeof element.focus !== "function") return;
    try {
      element.focus({ preventScroll: true });
    } catch {
      element.focus();
    }
  }

  function restoreFocusAfterRender() {
    const target = state.focusAfterRender;
    state.focusAfterRender = null;
    if (!target || !state.open || !dialogEl) return;
    if (target.kind === "candidate") {
      const row = Array.from(dialogEl.querySelectorAll("[data-action-handle]")).find(
        (element) => element.dataset.actionHandle === target.actionHandle,
      );
      focusElement(row);
      return;
    }
    if (target.kind === "provider_choice") {
      if (target.actionHandle !== state.selectedId) return;
      const input = Array.from(
        dialogEl.querySelectorAll('[name="recovery-provider-choice"]'),
      ).find((element) => element.value === target.providerChoiceHandle);
      focusElement(input);
    }
  }

  function normalizeCandidates(values, attentionOnly = false) {
    if (!Array.isArray(values)) return [];
    return values.filter(
      (candidate) =>
        candidateActionHandle(candidate) && (!attentionOnly || requiresAttention(candidate)),
    );
  }

  function replaceCandidates(values) {
    state.sourceCandidates = normalizeCandidates(values);
    state.candidates = normalizeCandidates(state.sourceCandidates, state.attentionOnly);
  }

  function ensureSelection() {
    if (
      state.candidates.some(
        (candidate) => candidateActionHandle(candidate) === state.selectedId,
      )
    ) return;
    state.selectedId =
      state.candidates.length > 0 ? candidateActionHandle(state.candidates[0]) : null;
    state.confirmDiscardId = null;
  }

  function render() {
    renderRecoveryCenter({
      modalEl,
      dialogEl,
      state,
      createNode,
      onClose: close,
      onEscape: escape,
      onSelect: select,
      onProviderChoiceSelect: selectProviderChoice,
      onAction: requestAction,
      onCancelDiscard: cancelDiscard,
      onConfirmDiscard: confirmDiscard,
    });
    restoreFocusAfterRender();
  }

  function open(values = state.sourceCandidates, options = {}) {
    state.open = true;
    state.attentionOnly = Boolean(options.attentionOnly);
    replaceCandidates(values);
    state.error = "";
    ensureSelection();
    render();
  }

  function openAttention(values = state.sourceCandidates) {
    open(values, { attentionOnly: true });
  }

  function close() {
    state.open = false;
    state.confirmDiscardId = null;
    state.error = "";
    render();
  }

  function escape() {
    if (state.confirmDiscardId) {
      cancelDiscard();
      return;
    }
    close();
  }

  function select(id) {
    if (state.pending) return;
    if (!state.candidates.some((candidate) => candidateActionHandle(candidate) === id)) return;
    state.selectedId = id;
    state.confirmDiscardId = null;
    state.error = "";
    state.focusAfterRender = { kind: "candidate", actionHandle: id };
    render();
  }

  function selectProviderChoice(choiceHandle) {
    if (state.pending || !state.selectedId) return;
    const selected = state.candidates.find(
      (candidate) => candidateActionHandle(candidate) === state.selectedId,
    );
    if (!providerChoices(selected).some((candidate) => candidate.rootId === choiceHandle)) return;
    state.providerChoiceSelections.set(state.selectedId, choiceHandle);
    state.error = "";
    state.focusAfterRender = {
      kind: "provider_choice",
      actionHandle: state.selectedId,
      providerChoiceHandle: choiceHandle,
    };
    render();
  }

  function requestAction(action) {
    if (state.pending || !state.selectedId) return false;
    if (!RECOVERY_CENTER_ACTIONS.some((definition) => definition.action === action)) return false;
    if (action === "discard") {
      state.confirmDiscardId = state.selectedId;
      state.error = "";
      render();
      return true;
    }
    return send(action);
  }

  function send(action) {
    if (state.pending || !state.selectedId) return false;
    const request = { actionHandle: state.selectedId, action };
    if (action === "confirm_resume") {
      const providerChoiceHandle = state.providerChoiceSelections.get(state.selectedId);
      if (providerChoiceHandle) request.providerChoiceHandle = providerChoiceHandle;
    }
    state.pending = request;
    state.confirmDiscardId = null;
    state.error = "";
    render();
    try {
      if (typeof sendAction !== "function") throw new Error("Recovery service is unavailable.");
      sendAction(request);
      return true;
    } catch (error) {
      state.pending = null;
      state.error = error instanceof Error ? error.message : "Recovery request failed.";
      render();
      return false;
    }
  }

  function cancelDiscard() {
    if (state.pending) return;
    state.confirmDiscardId = null;
    render();
  }

  function confirmDiscard() {
    if (state.confirmDiscardId !== state.selectedId) return false;
    return send("discard");
  }

  function setCandidates(values) {
    replaceCandidates(values);
    if (
      state.pending
      && !state.sourceCandidates.some(
        (candidate) => candidateActionHandle(candidate) === state.pending.actionHandle,
      )
    ) {
      state.pending = null;
    }
    ensureSelection();
    if (state.open) render();
  }

  function handleActionResult(result) {
    if (!state.pending) return false;
    const resultId = asText(
      result?.actionHandle ?? result?.action_handle,
    );
    const action = asText(result?.action);
    if (resultId !== state.pending.actionHandle || action !== state.pending.action) return false;
    const completed = state.pending;
    state.pending = null;
    state.error = result?.ok === false
      ? asText(result.message, "Recovery request failed.")
      : "";
    if (Array.isArray(result?.candidates)) {
      replaceCandidates(result.candidates);
    } else if (result?.resolved === true) {
      state.sourceCandidates = state.sourceCandidates.filter(
        (candidate) => candidateActionHandle(candidate) !== completed.actionHandle,
      );
      state.candidates = normalizeCandidates(state.sourceCandidates, state.attentionOnly);
    }
    ensureSelection();
    if (result?.close === true && result?.ok !== false) state.open = false;
    render();
    return true;
  }

  return {
    open,
    openAttention,
    close,
    dismiss: close,
    isOpen: () => state.open,
    setCandidates,
    handleActionResult,
    render,
  };
}
