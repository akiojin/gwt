// SPEC #2780 — Release Notes window.
//
// Renders the bundled CHANGELOG payload returned by the backend as a
// floating window: left sidebar lists every released version, right pane
// renders the selected version's notes. Markdown rendering is a tiny
// hand-rolled subset (### heading, - list, **bold**) matching exactly what
// the gwt-core parser emits.
//
// All DOM is constructed with createElement / textContent. innerHTML is
// not used so untrusted strings inside the bundled changelog (e.g. agent
// commit messages) cannot produce HTML injection even though they cannot
// reach this surface from network input today.
//
// Wired into app.js: `#app-version` click and the `#update-modal` "View
// release notes" link both send `open_release_notes` over WebSocket. The
// backend replies with `release_notes_payload` (or `release_notes_error`)
// and `app.js` forwards it here.

const CHANGELOG_URL =
  "https://github.com/akiojin/gwt/blob/main/CHANGELOG.md";

export function createReleaseNotesWindow({
  document,
  send = () => {},
  generateId = () => `release-notes-${Date.now()}`,
} = {}) {
  if (!document) {
    throw new Error("createReleaseNotesWindow requires a document");
  }

  const state = {
    root: null,
    sidebar: null,
    content: null,
    focusReturn: null,
    keydownHandler: null,
    entries: [],
    selectedVersion: null,
    pendingFocusVersion: null,
    lastRequestId: null,
    currentVersion: null,
    confirmModal: null,
  };

  // SPEC #2780 v2 Amendment (FR-015): minimal `major.minor.patch` comparison.
  // Returns -1 / 0 / 1; returns null when either value is unparseable so the
  // caller can fall back to a safe "disabled" state.
  function compareVersions(a, b) {
    if (typeof a !== "string" || typeof b !== "string") {
      return null;
    }
    const parse = (v) => {
      const parts = v.trim().replace(/^v/, "").split(".");
      if (parts.length < 3) {
        return null;
      }
      const nums = parts.slice(0, 3).map((p) => Number.parseInt(p, 10));
      return nums.some((n) => Number.isNaN(n)) ? null : nums;
    };
    const pa = parse(a);
    const pb = parse(b);
    if (!pa || !pb) {
      return null;
    }
    for (let i = 0; i < 3; i++) {
      if (pa[i] < pb[i]) {
        return -1;
      }
      if (pa[i] > pb[i]) {
        return 1;
      }
    }
    return 0;
  }

  function clearChildren(node) {
    while (node.firstChild) {
      node.removeChild(node.firstChild);
    }
  }

  function appendInlineMarkdown(target, text) {
    // Splits on **bold** markers and appends alternating text / <strong>
    // children. The bundled subset uses no other inline syntax.
    const pattern = /\*\*([^*]+)\*\*/g;
    let lastIndex = 0;
    for (const match of text.matchAll(pattern)) {
      if (match.index > lastIndex) {
        target.appendChild(
          document.createTextNode(text.slice(lastIndex, match.index)),
        );
      }
      const strong = document.createElement("strong");
      strong.textContent = match[1];
      target.appendChild(strong);
      lastIndex = match.index + match[0].length;
    }
    if (lastIndex < text.length) {
      target.appendChild(document.createTextNode(text.slice(lastIndex)));
    }
  }

  function buildEntryNode(entry) {
    const fragment = document.createDocumentFragment();
    if (!entry) {
      return fragment;
    }

    const header = document.createElement("header");
    header.className = "release-notes-version-header";

    const titleGroup = document.createElement("div");
    titleGroup.className = "release-notes-version-title";
    const h2 = document.createElement("h2");
    h2.textContent = `v${entry.version}`;
    titleGroup.appendChild(h2);
    if (entry.date) {
      const dateEl = document.createElement("span");
      dateEl.className = "release-notes-date";
      dateEl.textContent = entry.date;
      titleGroup.appendChild(dateEl);
    }
    header.appendChild(titleGroup);

    const actionBtn = buildUpdateActionButton(entry.version);
    if (actionBtn) {
      header.appendChild(actionBtn);
    }

    fragment.appendChild(header);

    const sectionsWrap = document.createElement("div");
    sectionsWrap.className = "release-notes-sections";
    for (const section of entry.sections) {
      const sectionEl = document.createElement("section");
      sectionEl.className = "release-notes-section";
      if (section.heading) {
        const h3 = document.createElement("h3");
        h3.textContent = section.heading;
        sectionEl.appendChild(h3);
      }
      if (section.items && section.items.length > 0) {
        const ul = document.createElement("ul");
        for (const item of section.items) {
          const li = document.createElement("li");
          appendInlineMarkdown(li, item);
          ul.appendChild(li);
        }
        sectionEl.appendChild(ul);
      }
      sectionsWrap.appendChild(sectionEl);
    }
    fragment.appendChild(sectionsWrap);
    return fragment;
  }

  // SPEC #2780 v2 Amendment (FR-015): build the sticky action button rendered
  // at the top-right of the content pane header. Returns `null` when the
  // backend payload did not provide `current_version` (older clients), so the
  // existing read-only Release Notes layout still renders.
  function buildUpdateActionButton(version) {
    if (!state.currentVersion) {
      return null;
    }
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "release-notes-update-action";

    const cmp = compareVersions(version, state.currentVersion);
    if (cmp === 0) {
      btn.classList.add("is-current");
      btn.disabled = true;
      btn.textContent = "Current version";
      btn.setAttribute("aria-disabled", "true");
    } else if (cmp === 1) {
      btn.classList.add("is-update");
      btn.textContent = `Update to v${version}`;
      btn.addEventListener("click", () => requestApply(version, false));
    } else if (cmp === -1) {
      btn.classList.add("is-downgrade");
      btn.textContent = `Downgrade to v${version}`;
      btn.addEventListener("click", () => requestApply(version, true));
    } else {
      // Unparseable version — render disabled so we never silently apply a
      // malformed tag to the update pipeline.
      btn.classList.add("is-current");
      btn.disabled = true;
      btn.textContent = "Unavailable";
      btn.setAttribute("aria-disabled", "true");
    }
    return btn;
  }

  function requestApply(version, requiresConfirm) {
    if (requiresConfirm) {
      showDowngradeConfirm(version);
      return;
    }
    sendApply(version);
    close();
  }

  function sendApply(version) {
    send({ kind: "apply_update_to_version", version });
  }

  function showDowngradeConfirm(version) {
    if (state.confirmModal) {
      return;
    }
    const overlay = document.createElement("div");
    overlay.className = "release-notes-downgrade-confirm";
    overlay.setAttribute("role", "dialog");
    overlay.setAttribute("aria-modal", "true");
    overlay.setAttribute(
      "aria-label",
      `Confirm downgrade to v${version}`,
    );

    const dialog = document.createElement("div");
    dialog.className = "release-notes-downgrade-confirm__dialog";

    const heading = document.createElement("h2");
    heading.className = "release-notes-downgrade-confirm__title";
    heading.textContent = `Downgrade to v${version}?`;
    dialog.appendChild(heading);

    const body = document.createElement("p");
    body.className = "release-notes-downgrade-confirm__body";
    body.textContent =
      "Downgrading installs an older build. Data formats are not guaranteed to be backward-compatible across versions.";
    dialog.appendChild(body);

    const actions = document.createElement("div");
    actions.className = "release-notes-downgrade-confirm__actions";

    const cancelBtn = document.createElement("button");
    cancelBtn.type = "button";
    cancelBtn.className = "release-notes-downgrade-confirm__cancel";
    cancelBtn.textContent = "Cancel";
    cancelBtn.addEventListener("click", () => dismissDowngradeConfirm());
    actions.appendChild(cancelBtn);

    const confirmBtn = document.createElement("button");
    confirmBtn.type = "button";
    confirmBtn.className = "release-notes-downgrade-confirm__confirm";
    confirmBtn.textContent = "Confirm downgrade";
    confirmBtn.addEventListener("click", () => {
      dismissDowngradeConfirm();
      sendApply(version);
      close();
    });
    actions.appendChild(confirmBtn);

    dialog.appendChild(actions);
    overlay.appendChild(dialog);
    if (state.root) {
      state.root.appendChild(overlay);
    } else {
      document.body.appendChild(overlay);
    }
    state.confirmModal = overlay;
  }

  function dismissDowngradeConfirm() {
    if (state.confirmModal && state.confirmModal.parentElement) {
      state.confirmModal.parentElement.removeChild(state.confirmModal);
    }
    state.confirmModal = null;
  }

  function buildEmptyStateNode(message) {
    const wrap = document.createElement("div");
    wrap.className = "release-notes-empty";
    wrap.setAttribute("role", "alert");
    const p1 = document.createElement("p");
    p1.textContent = message;
    wrap.appendChild(p1);
    const p2 = document.createElement("p");
    p2.appendChild(document.createTextNode("See the full changelog at "));
    const link = document.createElement("a");
    link.href = CHANGELOG_URL;
    link.target = "_blank";
    link.rel = "noreferrer noopener";
    link.textContent = CHANGELOG_URL;
    p2.appendChild(link);
    p2.appendChild(document.createTextNode("."));
    wrap.appendChild(p2);
    return wrap;
  }

  function ensureWindow() {
    if (state.root) {
      return state.root;
    }
    const root = document.createElement("div");
    root.className = "op-global-window release-notes-window";
    root.id = "release-notes-window";
    root.dataset.surface = "release-notes";
    root.setAttribute("role", "dialog");
    root.setAttribute("aria-modal", "false");
    root.setAttribute("aria-label", "Release notes");
    root.setAttribute("tabindex", "-1");

    const header = document.createElement("header");
    header.className = "op-global-window__titlebar release-notes-window-header";
    const title = document.createElement("h1");
    title.className = "op-global-window__title";
    title.textContent = "Release notes";
    header.appendChild(title);
    const actions = document.createElement("div");
    actions.className = "op-global-window__actions";
    const closeBtn = document.createElement("button");
    closeBtn.type = "button";
    closeBtn.className = "icon-button release-notes-close";
    closeBtn.setAttribute("aria-label", "Close release notes");
    closeBtn.textContent = "×";
    closeBtn.addEventListener("click", () => close());
    actions.appendChild(closeBtn);
    header.appendChild(actions);
    root.appendChild(header);

    const body = document.createElement("div");
    body.className = "op-global-window__body release-notes-body";

    const sidebar = document.createElement("aside");
    sidebar.className = "release-notes-sidebar";
    sidebar.setAttribute("role", "listbox");
    sidebar.setAttribute("aria-label", "Versions");
    body.appendChild(sidebar);

    const content = document.createElement("main");
    content.className = "release-notes-content";
    content.setAttribute("tabindex", "-1");
    body.appendChild(content);

    root.appendChild(body);

    state.root = root;
    state.sidebar = sidebar;
    state.content = content;
    state.keydownHandler = (event) => {
      if (event.key === "Escape") {
        event.preventDefault?.();
        close();
      }
    };
    document.addEventListener("keydown", state.keydownHandler);
    document.body.appendChild(root);
    try {
      root.focus({ preventScroll: true });
    } catch {
      root.focus();
    }
    return root;
  }

  function renderSidebar() {
    if (!state.sidebar) {
      return;
    }
    clearChildren(state.sidebar);
    if (!state.entries || state.entries.length === 0) {
      return;
    }
    for (const entry of state.entries) {
      const selected = entry.version === state.selectedVersion;
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = `release-notes-sidebar-item${selected ? " is-selected" : ""}`;
      btn.setAttribute("role", "option");
      btn.setAttribute("aria-selected", selected ? "true" : "false");
      btn.dataset.version = entry.version;
      const versionSpan = document.createElement("span");
      versionSpan.className = "release-notes-sidebar-version";
      versionSpan.textContent = `v${entry.version}`;
      btn.appendChild(versionSpan);
      if (entry.date) {
        const dateSpan = document.createElement("span");
        dateSpan.className = "release-notes-sidebar-date";
        dateSpan.textContent = entry.date;
        btn.appendChild(dateSpan);
      }
      btn.addEventListener("click", () => {
        select(entry.version);
      });
      state.sidebar.appendChild(btn);
    }
  }

  function renderContent() {
    if (!state.content) {
      return;
    }
    clearChildren(state.content);
    if (!state.entries || state.entries.length === 0) {
      state.content.appendChild(
        buildEmptyStateNode("Release notes could not be loaded."),
      );
      return;
    }
    const entry =
      state.entries.find((e) => e.version === state.selectedVersion) ||
      state.entries[0];
    state.content.appendChild(buildEntryNode(entry));
  }

  function select(version) {
    if (!version) {
      return;
    }
    const exists = state.entries.some((e) => e.version === version);
    state.selectedVersion = exists
      ? version
      : state.entries[0]
        ? state.entries[0].version
        : null;
    renderSidebar();
    renderContent();
  }

  function open(focusVersion = null) {
    state.pendingFocusVersion = focusVersion;
    state.focusReturn = document.activeElement;
    const id = generateId();
    state.lastRequestId = id;
    send({
      kind: "open_release_notes",
      id,
      focus_version: focusVersion || null,
    });
  }

  function handlePayload(payload) {
    if (!payload || !Array.isArray(payload.entries)) {
      return;
    }
    state.entries = payload.entries;
    // SPEC #2780 v2 Amendment (FR-013): payload now carries the running
    // version so the Update action button can pick the right label.
    state.currentVersion =
      typeof payload.current_version === "string"
        ? payload.current_version
        : null;
    const requested =
      payload.focus_version ||
      state.pendingFocusVersion ||
      (payload.entries[0] && payload.entries[0].version) ||
      null;
    state.selectedVersion = requested;
    ensureWindow();
    renderSidebar();
    renderContent();
    state.pendingFocusVersion = null;
  }

  function handleError(message) {
    state.entries = [];
    state.selectedVersion = null;
    ensureWindow();
    renderSidebar();
    if (state.content) {
      clearChildren(state.content);
      state.content.appendChild(
        buildEmptyStateNode(message || "Release notes could not be loaded."),
      );
    }
  }

  function close() {
    dismissDowngradeConfirm();
    if (state.keydownHandler) {
      document.removeEventListener("keydown", state.keydownHandler);
    }
    if (state.root && state.root.parentElement) {
      state.root.parentElement.removeChild(state.root);
    }
    const focusReturn = state.focusReturn;
    state.root = null;
    state.sidebar = null;
    state.content = null;
    state.keydownHandler = null;
    state.focusReturn = null;
    if (focusReturn && typeof focusReturn.focus === "function") {
      try {
        focusReturn.focus({ preventScroll: true });
      } catch {
        focusReturn.focus();
      }
    }
  }

  function isOpen() {
    return Boolean(state.root && state.root.parentElement);
  }

  function selectedVersion() {
    return state.selectedVersion;
  }

  return {
    open,
    handlePayload,
    handleError,
    close,
    isOpen,
    selectedVersion,
    // Exposed for tests; not part of the public WS contract.
    _select: select,
    _state: state,
  };
}
