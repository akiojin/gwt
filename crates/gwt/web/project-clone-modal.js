import { createFocusTrap } from "./focus-trap.js";

const focusReturnMap = new WeakMap();
const focusTrapMap = new WeakMap();

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
  onSearchQueryChange,
  onSearch,
  onRepositorySelect,
  onClone,
}) {
  const ownerDoc = modalEl.ownerDocument;
  const current = state || { open: false };
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
      onSearchQueryChange,
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
      onUrlChange,
    );
    body.appendChild(wrapper);
  }

  const destination = createNode("div", "clone-project-destination");
  const selectedPath = current.parentPath || "No destination selected";
  const parentButton = button(
    ownerDoc,
    "Choose Folder...",
    "wizard-button",
    onParentSelect,
    Boolean(current.cloning),
  );
  parentButton.id = "clone-project-parent-button";
  destination.append(
    createNode("span", "clone-project-field-label", "Destination parent"),
    createNode("code", "clone-project-path", selectedPath),
    parentButton,
  );
  body.appendChild(destination);

  if (current.progress) {
    body.appendChild(createNode("div", "clone-project-progress", current.progress));
  }
  if (current.error) {
    body.appendChild(createNode("div", "clone-project-error", current.error));
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
