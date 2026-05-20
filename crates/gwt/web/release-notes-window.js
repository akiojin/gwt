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
    entries: [],
    selectedVersion: null,
    pendingFocusVersion: null,
    lastRequestId: null,
  };

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
    const h2 = document.createElement("h2");
    h2.textContent = `v${entry.version}`;
    header.appendChild(h2);
    if (entry.date) {
      const dateEl = document.createElement("span");
      dateEl.className = "release-notes-date";
      dateEl.textContent = entry.date;
      header.appendChild(dateEl);
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
    root.className = "release-notes-window";
    root.id = "release-notes-window";
    root.setAttribute("role", "dialog");
    root.setAttribute("aria-modal", "false");
    root.setAttribute("aria-label", "Release notes");

    const header = document.createElement("header");
    header.className = "release-notes-window-header";
    const title = document.createElement("h1");
    title.textContent = "Release notes";
    header.appendChild(title);
    const closeBtn = document.createElement("button");
    closeBtn.type = "button";
    closeBtn.className = "release-notes-close";
    closeBtn.setAttribute("aria-label", "Close release notes");
    closeBtn.textContent = "×";
    closeBtn.addEventListener("click", () => close());
    header.appendChild(closeBtn);
    root.appendChild(header);

    const body = document.createElement("div");
    body.className = "release-notes-body";

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
    document.body.appendChild(root);
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
    if (state.root && state.root.parentElement) {
      state.root.parentElement.removeChild(state.root);
    }
    state.root = null;
    state.sidebar = null;
    state.content = null;
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
