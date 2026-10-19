import { createFocusTrap } from "./focus-trap.js";
import { createUiStateStore } from "./ui-state-store.js";

const focusReturnMap = new WeakMap();
const focusTrapMap = new WeakMap();

// One native picker may be pending on each client, across both entry points.
export function createFolderPickerController({ send, onChange, onSelected }) {
  const model = createUiStateStore({ pending: null, feedback: null, selected: null });
  // Request acknowledgement changes authority without rebuilding focused forms.
  const unsubscribeChanges = model.subscribe(current => current.feedback, feedback => {
    if (feedback) onChange(model.read().pending, feedback.message);
  });
  const unsubscribeSelected = model.subscribe(current => current.selected, event => {
    if (event) onSelected(event);
  });

  function applyFolderPickerReceiveEvent(event) {
    if (!event.kind?.startsWith("picker_")) return false;
    model.update(current => {
      const pending = current.pending;
      if (event.kind === "picker_busy") {
        return { ...current, pending: pending?.request_id === null ? null : pending,
          feedback: { message: event.message ?? "" } };
      }
      if (!pending || event.purpose !== pending.purpose) return current;
      if (event.kind === "picker_started") {
        return pending.request_id === null
          ? { ...current, pending: { ...pending, request_id: event.request_id } } : current;
      }
      if (pending.request_id === null || event.request_id !== pending.request_id) return current;
      if (!["picker_selected", "picker_cancelled", "picker_error"].includes(event.kind)) return current;
      return { ...current, pending: null,
        feedback: { message: event.kind === "picker_error" ? event.message ?? "" : "" },
        selected: event.kind === "picker_selected" ? event : current.selected };
    });
    return true;
  }

  return {
    begin(purpose) {
      if (model.read().pending) {
        model.update(current => ({ ...current,
          feedback: { message: "A folder picker is already open. Finish or cancel it first." },
        }));
        return false;
      }
      model.update(current => ({ ...current, pending: { purpose, request_id: null }, feedback: { message: "" } }));
      send({ kind: purpose === "open" ? "open_project_dialog" : "select_clone_project_parent" });
      return true;
    },
    receive: applyFolderPickerReceiveEvent,
    connectionLost() { model.update(current => ({ ...current, pending: null, feedback: { message: "" } })); },
    dispose() { unsubscribeChanges(); unsubscribeSelected(); model.dispose(); },
  };
}

export function createOpenProjectPathForm(ownerDoc, onOpen) {
  const details = ownerDoc.createElement("details");
  const summary = ownerDoc.createElement("summary");
  summary.textContent = "Enter folder path…";
  const { wrapper, input } = labeledInput(ownerDoc, "", "Project folder path", "", "Full folder path", () => {
    submit.disabled = !input.value.trim();
  });
  input.dataset.openProjectPath = "";
  const submit = button(ownerDoc, "Open path", "wizard-button", () => {
    if (input.value.trim()) onOpen(input.value.trim());
  }, true);
  submit.dataset.openProjectSubmit = "";
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && input.value.trim()) { event.preventDefault(); onOpen(input.value.trim()); }
  });
  details.append(summary, wrapper, submit);
  return details;
}

export function createOpenProjectPathDialog(ownerDoc, { onChoose, onOpen }) {
  const model = createUiStateStore({ open: false, awaitingOpen: null, pending: false, message: "" });
  const node = (tag, className, text) => {
    const element = ownerDoc.createElement(tag);
    element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  };
  const modal = node("div", "modal-backdrop");
  modal.id = "open-project-path-modal";
  modal.setAttribute("aria-hidden", "true");
  const dialog = node("div", "modal-shell");
  dialog.setAttribute("role", "dialog");
  dialog.setAttribute("aria-modal", "true");
  dialog.setAttribute("aria-labelledby", "open-project-path-title");
  dialog.tabIndex = -1;
  const header = node("div", "modal-header");
  const title = node("h2", "", "Open Project");
  title.id = "open-project-path-title";
  header.append(title);
  const body = node("div", "modal-body");
  const status = node("p", "project-picker-copy", "");
  status.setAttribute("role", "status");
  status.hidden = true;
  const form = createOpenProjectPathForm(ownerDoc, (path) => {
    const requestId = globalThis.crypto?.randomUUID?.()
      ?? `open-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
    waitForOpen(path, requestId);
    onOpen(path, requestId);
  });
  form.open = true;
  body.append(status, form);
  const footer = node("div", "modal-footer");
  const choose = button(ownerDoc, "Choose Folder…", "wizard-button", onChoose);
  footer.append(button(ownerDoc, "Close", "text-button", hide), choose);
  dialog.append(header, body, footer);
  modal.append(dialog);
  function hide() { model.update(current => ({ ...current, open: false, awaitingOpen: null })); }
  function waitForOpen(path, requestId) {
    const input = form.querySelector('[data-open-project-path]');
    input.value = path;
    form.querySelector('[data-open-project-submit]').disabled = !path.trim();
    model.update(current => ({ ...current, open: true, awaitingOpen: requestId, message: "Opening project…" }));
  }
  function applyOpenProjectPathResultReceiveEvent(event) {
    model.update(current => {
      if (!current.awaitingOpen || event.request_id !== current.awaitingOpen || !["project_opened", "project_open_error"].includes(event.kind)) return current;
      return { ...current, awaitingOpen: null, open: event.kind !== "project_opened",
        message: event.kind === "project_open_error" ? event.message || "Could not open the project." : current.message };
    });
  }
  modal.addEventListener("click", (event) => { if (event.target === modal) hide(); });
  modal.addEventListener("keydown", (event) => {
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); hide(); }
  });
  const unsubscribe = model.subscribe(current => current, current => {
    if (current.open && !modal.classList.contains("open")) openModal(modal, dialog, ownerDoc);
    if (!current.open && modal.classList.contains("open")) closeModal(modal, dialog);
    choose.textContent = current.pending ? "Choosing folder…" : "Choose Folder…";
    dialog.setAttribute("aria-busy", String(current.open && current.pending));
    status.textContent = current.message;
    status.hidden = !current.message;
  });
  return {
    modal,
    hide,
    waitForOpen,
    receive: applyOpenProjectPathResultReceiveEvent,
    dispose() { unsubscribe(); model.dispose(); },
    open() { model.update(current => ({ ...current, open: true })); },
    update(pending, message) {
      model.update(current => ({ ...current, pending: Boolean(pending),
        message: message || (pending ? "Choose a folder in the system dialog, or cancel to return." : "") }));
    },
  };
}

function button(ownerDoc, label, className, onClick, disabled = false) {
  const element = ownerDoc.createElement("button");
  element.type = "button";
  element.className = className;
  element.textContent = label;
  element.disabled = disabled;
  element.addEventListener("click", onClick);
  return element;
}

function labeledInput(ownerDoc, id, label, value, placeholder, onInput) {
  const wrapper = ownerDoc.createElement("label");
  wrapper.className = "clone-project-field";
  const text = ownerDoc.createElement("span");
  text.className = "clone-project-field-label";
  text.textContent = label;
  const input = ownerDoc.createElement("input");
  input.id = id;
  input.type = "text";
  input.value = value || "";
  input.placeholder = placeholder;
  input.spellcheck = false;
  input.addEventListener("input", () => onInput(input.value));
  wrapper.append(text, input);
  return { wrapper, input };
}

function closeModal(modalEl, dialogEl) {
  modalEl.classList.remove("open");
  modalEl.setAttribute("aria-hidden", "true");
  dialogEl.setAttribute("aria-busy", "false");
  const releaseTrap = focusTrapMap.get(modalEl);
  if (typeof releaseTrap === "function") {
    releaseTrap();
  }
  focusTrapMap.delete(modalEl);
  const returnEl = focusReturnMap.get(modalEl);
  if (returnEl && typeof returnEl.focus === "function") {
    try { returnEl.focus({ preventScroll: true }); }
    catch { returnEl.focus(); }
  }
  focusReturnMap.delete(modalEl);
}

function openModal(modalEl, dialogEl, ownerDoc) {
  const wasOpen = modalEl.classList.contains("open");
  if (!wasOpen) {
    focusReturnMap.set(modalEl, ownerDoc.activeElement);
  }
  modalEl.classList.add("open");
  modalEl.removeAttribute("aria-hidden");
  if (!wasOpen) {
    try { dialogEl.focus({ preventScroll: true }); }
    catch { dialogEl.focus(); }
    const release = createFocusTrap(dialogEl, { document: ownerDoc });
    focusTrapMap.set(modalEl, release);
  }
}

function renderRepositoryResults({ body, state, onRepositorySelect, createNode }) {
  const list = createNode("div", "clone-project-results");
  const repositories = Array.isArray(state.repositories) ? state.repositories : [];
  if (state.searching) {
    list.appendChild(createNode("div", "clone-project-empty", "Searching repositories..."));
  } else if (repositories.length === 0) {
    list.appendChild(createNode("div", "clone-project-empty", "No repositories selected"));
  } else {
    for (const repo of repositories) {
      const row = body.ownerDocument.createElement("button");
      row.type = "button";
      row.className = "clone-project-result";
      row.dataset.cloneRepositoryUrl = repo.url || "";
      if (repo.url && repo.url === state.selectedRepositoryUrl) {
        row.classList.add("selected");
      }
      row.addEventListener("click", () => onRepositorySelect(repo.url || ""));

      const title = createNode("span", "clone-project-result-title", repo.full_name || repo.url || "");
      const meta = createNode(
        "span",
        "clone-project-result-meta",
        [
          repo.visibility,
          repo.default_branch ? `default: ${repo.default_branch}` : "",
          repo.updated_at,
        ].filter(Boolean).join(" | "),
      );
      const description = createNode(
        "span",
        "clone-project-result-description",
        repo.description || "No description",
      );
      row.append(title, description, meta);
      list.appendChild(row);
    }
  }
  body.appendChild(list);
}

export function renderProjectCloneModal({
  modalEl,
  dialogEl,
  state,
  createNode,
  onClose,
  onModeChange,
  onUrlChange,
  onParentSelect,
  onParentChange = () => {},
  onSearchQueryChange,
  onSearch,
  onRepositorySelect,
  onClone,
}) {
  const ownerDoc = modalEl.ownerDocument;
  // Input handlers edit a view-local draft, never the immutable model snapshot.
  const current = { ...(state || { open: false }) };
  dialogEl.innerHTML = "";
  if (!current.open) {
    closeModal(modalEl, dialogEl);
    return;
  }

  openModal(modalEl, dialogEl, ownerDoc);
  dialogEl.setAttribute("aria-busy", current.cloning || current.searching ? "true" : "false");

  const header = createNode("div", "modal-header clone-project-header");
  const heading = createNode("div", "");
  const title = createNode("h2", "", "Clone Project");
  title.id = "clone-project-modal-title";
  const subtitle = createNode(
    "p",
    "clone-project-subtitle",
    "Create a new gwt project from a GitHub repository.",
  );
  heading.append(title, subtitle);
  header.append(
    heading,
    button(ownerDoc, "Close", "text-button", onClose, Boolean(current.cloning)),
  );

  const body = createNode("div", "modal-body clone-project-body");
  function updateCloneAvailability() {
    const selectedUrl = current.mode === "search" ? current.selectedRepositoryUrl : current.url;
    const clone = dialogEl.querySelector("#clone-project-start");
    if (clone) clone.disabled = !String(selectedUrl || "").trim() || !String(current.parentPath || "").trim() || Boolean(current.cloning);
  }
  const modeRow = createNode("div", "clone-project-mode-row");
  for (const mode of ["url", "search"]) {
    const modeButton = button(
      ownerDoc,
      mode === "url" ? "URL" : "Search",
      "clone-project-mode-button",
      () => onModeChange(mode),
      Boolean(current.cloning),
    );
    modeButton.dataset.cloneMode = mode;
    modeButton.setAttribute("aria-pressed", current.mode === mode ? "true" : "false");
    modeRow.appendChild(modeButton);
  }
  body.appendChild(modeRow);

  if (current.mode === "search") {
    const { wrapper: searchWrapper, input: searchInput } = labeledInput(
      ownerDoc,
      "clone-project-search-input",
      "Repository search",
      current.query,
      "owner/name or keywords",
      (value) => {
        current.query = value;
        onSearchQueryChange(value);
        const searchButton = searchRow.querySelector("button");
        searchButton.disabled = Boolean(current.searching || current.cloning || !value.trim());
      },
    );
    const searchRow = createNode("div", "clone-project-search-row");
    searchRow.append(
      searchWrapper,
      button(
        ownerDoc,
        current.searching ? "Searching..." : "Search",
        "wizard-button",
        onSearch,
        Boolean(current.searching || current.cloning || !String(current.query || "").trim()),
      ),
    );
    body.appendChild(searchRow);
    renderRepositoryResults({ body, state: current, onRepositorySelect, createNode });
    if (!current.selectedRepositoryUrl && searchInput && repositoriesAvailable(current)) {
      searchInput.setAttribute("aria-describedby", "clone-project-search-hint");
    }
  } else {
    const { wrapper } = labeledInput(
      ownerDoc,
      "clone-project-url-input",
      "Repository URL",
      current.url,
      "https://github.com/owner/repo.git or git@github.com:owner/repo.git",
      (value) => { current.url = value; onUrlChange(value); updateCloneAvailability(); },
    );
    body.appendChild(wrapper);
  }

  const destination = createNode("div", "clone-project-destination");
  const { wrapper: parentField } = labeledInput(ownerDoc, "clone-project-parent-input", "Destination parent", current.parentPath,
    "Full destination folder path", (value) => { current.parentPath = value; onParentChange(value); updateCloneAvailability(); });
  const parentButton = button(
    ownerDoc,
    current.pickerPending ? "Choosing folder…" : "Choose Folder...",
    "wizard-button",
    onParentSelect,
    Boolean(current.cloning),
  );
  parentButton.id = "clone-project-parent-button";
  destination.append(
    parentField,
    parentButton,
  );
  body.appendChild(destination);
  if (current.pickerMessage) body.appendChild(createNode("p", "clone-project-progress", current.pickerMessage));

  if (current.progress) {
    body.appendChild(createNode("div", "clone-project-progress", current.progress));
  }
  if (current.error) {
    body.appendChild(createNode("div", "clone-project-error", current.error));
  }
  if (current.existingProjectUrl) {
    const link = createNode("a", "", "Open existing project");
    link.href = current.existingProjectUrl;
    link.target = "_blank";
    link.rel = "noopener";
    body.appendChild(link);
  }

  const selectedUrl =
    current.mode === "search" ? current.selectedRepositoryUrl : current.url;
  const canClone =
    String(selectedUrl || "").trim() &&
    String(current.parentPath || "").trim() &&
    !current.cloning;
  const footer = createNode("div", "modal-footer clone-project-footer");
  const cloneButton = button(
    ownerDoc,
    current.cloning ? "Cloning..." : "Clone",
    "wizard-button primary",
    onClone,
    !canClone,
  );
  cloneButton.id = "clone-project-start";
  footer.append(
    button(ownerDoc, "Cancel", "text-button", onClose, Boolean(current.cloning)),
    cloneButton,
  );

  dialogEl.append(header, body, footer);
}

function repositoriesAvailable(state) {
  return Array.isArray(state.repositories) && state.repositories.length > 0;
}
