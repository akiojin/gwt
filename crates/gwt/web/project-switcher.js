const AGENT_WINDOW_PRESETS = new Set(["agent", "claude", "codex"]);

function normalizeRuntimeState(value) {
  return String(value || "").toLowerCase();
}

function windowIsAgentPane(windowData) {
  const preset = String(windowData?.preset || "").toLowerCase();
  return Boolean(windowData?.agent_id) || AGENT_WINDOW_PRESETS.has(preset);
}

function windowsForTab(tab) {
  const windows = tab?.workspace?.windows;
  return Array.isArray(windows) ? windows : [];
}

function runningAgentWindows(tab, runtimeStateForWindow) {
  return windowsForTab(tab).filter((windowData) => {
    if (!windowIsAgentPane(windowData)) {
      return false;
    }
    const state =
      typeof runtimeStateForWindow === "function"
        ? runtimeStateForWindow(windowData)
        : windowData.status;
    return normalizeRuntimeState(state) === "running";
  });
}

function isEditableTarget(target) {
  if (!target) {
    return false;
  }
  const tagName = String(target.tagName || "").toLowerCase();
  if (tagName === "input" || tagName === "textarea" || tagName === "select") {
    return true;
  }
  return Boolean(target.isContentEditable);
}

function rowMetaForRecent(project) {
  const kind = String(project?.kind || "").trim();
  const path = String(project?.path || "").trim();
  if (kind && path) return `${kind} · ${path}`;
  return path || kind || "";
}

export function nextProjectTabId(tabs, activeTabId, direction = "next") {
  const ids = (Array.isArray(tabs) ? tabs : [])
    .map((tab) => tab?.id)
    .filter(Boolean);
  if (ids.length <= 1) {
    return null;
  }
  const delta = direction === "previous" || direction === -1 ? -1 : 1;
  const currentIndex = ids.indexOf(activeTabId);
  if (currentIndex === -1) {
    return delta > 0 ? ids[0] : ids[ids.length - 1];
  }
  return ids[(currentIndex + delta + ids.length) % ids.length];
}

// Cmd+O (mac) / Ctrl+O (Windows/Linux) opens the native folder dialog. The
// single `Projects ▾` control absorbed the standalone split-button, so this
// keyboard path preserves the one-press Open Folder muscle memory the split
// button's primary half used to provide (SPEC-2013 US-10 / FR-025). It mirrors
// the project-switcher shortcut guard: never steals editable input, never
// fires on key-repeat, and stays inert while a modal/dropdown owns focus.
export function shouldTriggerOpenFolderHotkey(event, { modalOpen = false } = {}) {
  if (!event || event.repeat) {
    return false;
  }
  if (!(event.metaKey || event.ctrlKey) || event.shiftKey || event.altKey) {
    return false;
  }
  if (modalOpen) {
    return false;
  }
  if (isEditableTarget(event.target)) {
    return false;
  }
  const key = String(event.key || "").toLowerCase();
  return key === "o" || String(event.code || "") === "KeyO";
}

export function shouldHandleProjectSwitcherShortcut(
  event,
  {
    projectCount = 0,
    modalOpen = false,
    dropdownOpen = false,
  } = {},
) {
  if (!event || event.repeat) {
    return false;
  }
  if (!(event.metaKey || event.ctrlKey) || !event.shiftKey || event.altKey) {
    return false;
  }
  if (modalOpen || dropdownOpen) {
    return false;
  }
  if (isEditableTarget(event.target)) {
    return false;
  }
  if (event.key === "ArrowUp" || event.key === "ArrowDown") {
    return projectCount > 1;
  }
  return String(event.key || "").toLowerCase() === "p";
}

export function buildProjectSwitcherRows({
  tabs,
  recentProjects,
  activeTabId,
  unreadProjectIds = new Set(),
  runtimeStateForWindow,
} = {}) {
  const openTabs = Array.isArray(tabs) ? tabs : [];
  const rows = [];
  const openPaths = new Set();

  for (const tab of openTabs) {
    if (!tab?.id) {
      continue;
    }
    const runningAgents = runningAgentWindows(tab, runtimeStateForWindow);
    const path = String(tab.project_root || "").trim();
    if (path) {
      openPaths.add(path);
    }
    rows.push({
      section: "open",
      type: "open",
      id: tab.id,
      title: tab.title || path || "Project",
      path,
      meta:
        runningAgents.length > 0
          ? `${runningAgents.length} running`
          : path || "Open project",
      active: tab.id === activeTabId,
      unread: unreadProjectIds.has(tab.id),
      runningCount: runningAgents.length,
    });
  }

  for (const project of Array.isArray(recentProjects) ? recentProjects : []) {
    const path = String(project?.path || "").trim();
    if (!path || openPaths.has(path)) {
      continue;
    }
    rows.push({
      section: "recent",
      type: "recent",
      title: project?.title || path,
      path,
      meta: rowMetaForRecent(project),
      active: false,
      unread: false,
      runningCount: 0,
    });
  }

  return rows;
}

export function createProjectSwitcherController({
  buttonEl,
  panelEl,
  getState,
  send,
  createNode,
  runtimeStateForWindow,
  unreadProjectIds = new Set(),
  clearUnreadProject = () => {},
  getNotificationPermission = () => "unsupported",
  requestNotificationPermission = null,
  onOpenFolder = () => {},
  onCloneFromGithub = () => {},
} = {}) {
  let open = false;
  let selectedIndex = 0;
  let lastRows = [];

  function currentRows() {
    const state = getState?.() || {};
    return buildProjectSwitcherRows({
      tabs: state.tabs || [],
      recentProjects: state.recent_projects || [],
      activeTabId: state.active_tab_id || null,
      unreadProjectIds,
      runtimeStateForWindow,
    });
  }

  function close({ restoreFocus = false } = {}) {
    open = false;
    render();
    if (restoreFocus && buttonEl && typeof buttonEl.focus === "function") {
      try {
        buttonEl.focus({ preventScroll: true });
      } catch {
        buttonEl.focus();
      }
    }
  }

  function selectRow(row) {
    if (!row) {
      return;
    }
    if (row.type === "open") {
      clearUnreadProject(row.id);
      send?.({ kind: "select_project_tab", tab_id: row.id });
      close({ restoreFocus: true });
      return;
    }
    if (row.type === "recent") {
      send?.({ kind: "reopen_recent_project", path: row.path });
      close({ restoreFocus: true });
    }
  }

  function appendSectionLabel(fragment, text) {
    const label = createNode("div", "project-switcher-section-label", text);
    label.setAttribute("role", "presentation");
    fragment.appendChild(label);
  }

  function appendRow(fragment, row, index) {
    const item = createNode("button", "project-switcher-row");
    item.type = "button";
    item.dataset.projectSwitcherIndex = String(index);
    item.dataset.projectSwitcherType = row.type;
    if (row.id) {
      item.dataset.projectTabId = row.id;
    }
    if (row.path) {
      item.dataset.projectPath = row.path;
    }
    item.setAttribute("role", "option");
    item.setAttribute("aria-selected", index === selectedIndex ? "true" : "false");
    item.classList.toggle("active", Boolean(row.active));
    item.classList.toggle("selected", index === selectedIndex);
    item.addEventListener("click", () => selectRow(row));

    const title = createNode("span", "project-switcher-row__title", row.title);
    const meta = createNode("span", "project-switcher-row__meta", row.meta);
    item.append(title, meta);

    if (row.unread) {
      const badge = createNode("span", "project-switcher-row__badge", "New");
      item.appendChild(badge);
    } else if (row.runningCount > 0) {
      const badge = createNode(
        "span",
        "project-switcher-row__badge project-switcher-row__badge--running",
        String(row.runningCount),
      );
      badge.setAttribute("aria-label", `${row.runningCount} running agents`);
      item.appendChild(badge);
    }

    fragment.appendChild(item);
  }

  function appendActions(fragment) {
    // SPEC-2013 US-9: the consolidated `Projects ▾` menu owns project intake
    // too. The separator divides the switchable OPEN / RECENT rows from the
    // Open Folder / Clone actions that replace the retired split button.
    const separator = createNode("div", "project-switcher-actions-separator");
    separator.setAttribute("role", "presentation");
    fragment.appendChild(separator);

    const openFolder = createNode(
      "button",
      "project-switcher-action",
      "Open Folder…",
    );
    openFolder.type = "button";
    openFolder.dataset.action = "open-folder";
    openFolder.addEventListener("click", () => {
      close({ restoreFocus: true });
      onOpenFolder?.();
    });

    const clone = createNode(
      "button",
      "project-switcher-action",
      "Clone from GitHub…",
    );
    clone.type = "button";
    clone.dataset.action = "clone-from-github";
    clone.addEventListener("click", () => {
      close({ restoreFocus: true });
      onCloneFromGithub?.();
    });

    fragment.append(openFolder, clone);
  }

  function appendNotificationPermissionAction(fragment) {
    if (
      getNotificationPermission() !== "default" ||
      typeof requestNotificationPermission !== "function"
    ) {
      return;
    }
    const button = createNode(
      "button",
      "project-switcher-permission",
      "Enable desktop notifications",
    );
    button.type = "button";
    button.dataset.action = "enable-desktop-notifications";
    button.addEventListener("click", () => {
      requestNotificationPermission().finally(() => render());
    });
    fragment.appendChild(button);
  }

  function render() {
    if (!buttonEl || !panelEl) {
      return;
    }
    buttonEl.setAttribute("aria-expanded", open ? "true" : "false");
    panelEl.classList.toggle("open", open);
    panelEl.hidden = !open;
    panelEl.setAttribute("aria-hidden", open ? "false" : "true");

    if (!open) {
      panelEl.replaceChildren();
      return;
    }

    lastRows = currentRows();
    if (selectedIndex < 0 || selectedIndex >= lastRows.length) {
      selectedIndex = 0;
    }

    const fragment = panelEl.ownerDocument.createDocumentFragment();
    if (lastRows.length === 0) {
      fragment.appendChild(
        createNode("div", "project-switcher-empty", "No projects"),
      );
    } else {
      let previousSection = "";
      lastRows.forEach((row, index) => {
        if (row.section !== previousSection) {
          appendSectionLabel(
            fragment,
            row.section === "open" ? "Open Projects" : "Recent",
          );
          previousSection = row.section;
        }
        appendRow(fragment, row, index);
      });
    }
    appendNotificationPermissionAction(fragment);
    appendActions(fragment);
    panelEl.replaceChildren(fragment);
  }

  function openSwitcher() {
    open = true;
    selectedIndex = Math.max(0, selectedIndex);
    render();
    const selected = panelEl?.querySelector(".project-switcher-row.selected");
    if (selected && typeof selected.focus === "function") {
      try {
        selected.focus({ preventScroll: true });
      } catch {
        selected.focus();
      }
    }
  }

  function toggle() {
    if (open) {
      close({ restoreFocus: true });
    } else {
      openSwitcher();
    }
  }

  function moveSelection(delta) {
    if (!open || lastRows.length === 0) {
      return;
    }
    selectedIndex = (selectedIndex + delta + lastRows.length) % lastRows.length;
    render();
    panelEl
      ?.querySelector(".project-switcher-row.selected")
      ?.focus?.({ preventScroll: true });
  }

  function handlePanelKeydown(event) {
    if (!open) {
      return false;
    }
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        moveSelection(1);
        return true;
      case "ArrowUp":
        event.preventDefault();
        moveSelection(-1);
        return true;
      case "Enter": {
        // Enter selects only when focus is on an OPEN / RECENT row. Action
        // buttons (Open Folder / Clone) and the notification-permission button
        // must keep their native Enter -> click activation, so the panel-level
        // handler must not preventDefault / selectRow for them.
        const target = event.target;
        const isRow =
          typeof target?.classList?.contains === "function" &&
          target.classList.contains("project-switcher-row");
        if (!isRow) {
          return false;
        }
        event.preventDefault();
        selectRow(lastRows[selectedIndex]);
        return true;
      }
      case "Escape":
        event.preventDefault();
        close({ restoreFocus: true });
        return true;
      default:
        return false;
    }
  }

  buttonEl?.addEventListener("click", (event) => {
    event.stopPropagation();
    toggle();
  });
  panelEl?.addEventListener("keydown", handlePanelKeydown);

  return {
    render,
    open: openSwitcher,
    close,
    toggle,
    isOpen: () => open,
    handlePanelKeydown,
    selectRow,
  };
}
