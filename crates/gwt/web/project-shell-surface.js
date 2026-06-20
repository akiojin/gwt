// SPEC-3064 Phase 3 (E7) — Project & workspace shell chrome surface
// extracted from app.js. Owns the project tab strip (close-tab confirm
// modal + running-agent cues), the Recent Projects list (picker overlay +
// Open Project split-button menu), the project picker / onboarding
// renderers, the toolbar action availability, the Windows dropdown
// (window list state + render key + renderer), the maximized-window
// viewport sync, the Open/Clone project modal glue, the migration modal
// glue, the window_list / clone_* / migration_* receive() bodies, and the
// project-shell chrome listeners. Pure movement from app.js: behavior,
// DOM output, and WS protocol are unchanged; the moved code keeps its
// original app.js indentation. Textual changes are limited to:
// - `appState` reads go through the `getAppState()` accessor (a local
//   `const appState = getAppState();` prologue), because app.js reassigns
//   `appState` on every workspace_state,
// - `projectError` reads go through `getProjectError()` the same way,
// - in-module references that previously went through
//   `frontendUnits.projectWorkspaceShell.*` became direct local calls
//   (sendOpenProjectDialog, toggleWindowList, renderWindowList),
// - the global-Esc branches for the migration modal and the Windows
//   dropdown moved behind handleMigrationModalEscape /
//   handleWindowListEscape (same delegate pattern as the launch wizard
//   surface's handleWizardEscapeKeydown),
// - the receive() bodies for window_list / clone_* / migration_* moved
//   behind applyWindowListEvent / applyCloneProjectReceiveEvent /
//   applyMigrationReceiveEvent; the case arms stay in app.js as thin
//   delegates.
//
// deps:
// - send(message): forward a frontend event over the WebSocket bridge.
// - createNode(tag, className, textContent): shared DOM helper owned by
//   app.js (also used by Board / settings / wizard surfaces).
// - getAppState() / getProjectError(): accessors for app.js-owned mutable
//   shell state.
// - activeProjectTab() / activeWorkspace(): app.js workspace selectors.
// - windowMap: workspace window element map owned by app.js.
// - appendRenderKeyPart(parts, value): shared render-key primitive (also
//   used by the render keys that stayed in app.js).
// - runtimeStateForWindow / windowDisplayTitle / windowTitleTooltip /
//   windowRoleBadgeLabel / windowGeometryLabel / shouldShowRuntimeStatus /
//   visibleWindowData / presetSurface: window presentation helpers owned
//   by app.js (still used by the core window renderers there).
// - focusWindowRemotely(windowId, opts): focus path owned by app.js.
// - frameWindow(windowId, opts): SPEC-2008 camera-focus path owned by app.js;
//   flies the local camera to frame a window (used by the switcher rows).
// - scheduleTerminalFit(windowId, persist): terminal core scheduler.
// - visibleBounds(): current visible canvas bounds in world space.
// - addButton / tileButton / stackButton: toolbar buttons owned by app.js
//   (their click listeners stay there).
// - cloneProjectModal / cloneProjectDialog / migrationModal /
//   migrationDialog: modal elements owned by app.js because the focus
//   shortcut guard and clone-modal-focus-guard wiring also read them.
import { renderMigrationModal as renderMigrationModalView } from "/migration-modal.js";
import { renderProjectCloneModal as renderProjectCloneModalView } from "/project-clone-modal.js";
import {
  renderProjectTabs as renderProjectTabsView,
  updateProjectTabStateCue as updateProjectTabStateCueView,
} from "/project-tabs-renderer.js";
import { renderCloseProjectTabConfirmModal } from "/close-project-tab-confirm-modal.js";
import {
  createProjectSwitcherController,
  nextProjectTabId,
  shouldHandleProjectSwitcherShortcut,
  shouldTriggerOpenFolderHotkey,
} from "/project-switcher.js";
import { windowRuntimeLabel } from "/window-runtime-state.js";

export function createProjectShellSurface({
  send,
  createNode,
  getAppState,
  getProjectError,
  activeProjectTab,
  activeWorkspace,
  windowMap,
  appendRenderKeyPart,
  runtimeStateForWindow,
  windowDisplayTitle,
  windowTitleTooltip,
  windowRoleBadgeLabel,
  windowGeometryLabel,
  shouldShowRuntimeStatus,
  visibleWindowData,
  presetSurface,
  focusWindowRemotely,
  // SPEC-2008 camera-focus: window-list rows teleport the camera to a window
  // (frameWindow) instead of maximizing/restoring it.
  frameWindow,
  scheduleTerminalFit,
  visibleBounds,
  addButton,
  tileButton,
  stackButton,
  cloneProjectModal,
  cloneProjectDialog,
  migrationModal,
  migrationDialog,
  isModalOpen = () => false,
}) {
      const projectTabs = document.getElementById("project-tabs");
      const projectSwitcherButton = document.getElementById(
        "project-switcher-button",
      );
      const projectSwitcherPanel = document.getElementById(
        "project-switcher-panel",
      );
      const projectPicker = document.getElementById("project-picker");
      const projectPickerError = document.getElementById("project-picker-error");
      const pickerOpenProjectButton = document.getElementById("picker-open-project");
      const pickerCloneProjectButton = document.getElementById("picker-clone-project");
      const recentProjectList = document.getElementById("recent-project-list");
      const projectOnboarding = document.getElementById("project-onboarding");
      const projectOnboardingTitle = document.getElementById(
        "project-onboarding-title",
      );
      const projectOnboardingCopy = document.getElementById(
        "project-onboarding-copy",
      );
      const onboardingOpenProjectButton = document.getElementById(
        "onboarding-open-project",
      );
      const windowListButton = document.getElementById("window-list-button");
      const windowListPanel = document.getElementById("window-list-panel");

      let windowListOpen = false;
      let windowListEntries = [];
      let renderedRecentProjectsKey = "";
      let renderedWindowListKey = "";
      let renderedProjectPickerKey = "";
      let renderedProjectOnboardingKey = "";
      let renderedActionAvailabilityKey = "";
      const unreadProjectIds = new Set();

      function recentProjectsRenderKey(state) {
        return JSON.stringify(
          (state?.recent_projects || []).map((project) => ({
            title: project?.title || "",
            kind: project?.kind || "",
            path: project?.path || "",
          })),
        );
      }

      function desktopNotificationPermission() {
        return window?.Notification?.permission || "unsupported";
      }

      function requestDesktopNotificationPermission() {
        if (typeof window?.Notification?.requestPermission !== "function") {
          return Promise.resolve(desktopNotificationPermission());
        }
        return window.Notification.requestPermission();
      }

      function renderProjectSwitcherButtonState() {
        if (!projectSwitcherButton) {
          return;
        }
        projectSwitcherButton.classList.toggle(
          "has-unread",
          unreadProjectIds.size > 0,
        );
      }

      function renderProjectSwitcher() {
        projectSwitcherController.render();
        renderProjectSwitcherButtonState();
      }

      function clearProjectUnread(projectId, { render = true } = {}) {
        if (!projectId || !unreadProjectIds.delete(projectId)) {
          return;
        }
        if (render) {
          renderProjectSwitcher();
        }
      }

      function markProjectUnread(projectId) {
        if (!projectId || projectId === getAppState()?.active_tab_id) {
          return;
        }
        unreadProjectIds.add(projectId);
        renderProjectSwitcher();
      }

      function closeProjectSwitcher({ restoreFocus = false } = {}) {
        projectSwitcherController.close({ restoreFocus });
      }

      const projectSwitcherController = createProjectSwitcherController({
        buttonEl: projectSwitcherButton,
        panelEl: projectSwitcherPanel,
        getState: getAppState,
        send,
        createNode,
        runtimeStateForWindow,
        unreadProjectIds,
        clearUnreadProject: clearProjectUnread,
        getNotificationPermission: desktopNotificationPermission,
        requestNotificationPermission: requestDesktopNotificationPermission,
        onOpenFolder: sendOpenProjectDialog,
        onCloneFromGithub: openCloneProjectModal,
      });

      function windowListRenderKey() {
        const appState = getAppState();
        const workspace = activeWorkspace();
        const workspaceWindows = workspace.windows || [];
        const workspaceWindowMap = new Map();
        for (const windowData of workspaceWindows) {
          workspaceWindowMap.set(windowData.id, windowData);
        }
        const appendEntryKey = (parts, entry) => {
          const geometry = entry?.geometry || {};
          const runtimeState = runtimeStateForWindow(entry);
          appendRenderKeyPart(parts, "id");
          appendRenderKeyPart(parts, entry?.id || "");
          appendRenderKeyPart(parts, "preset");
          appendRenderKeyPart(parts, entry?.preset || "");
          appendRenderKeyPart(parts, "title");
          appendRenderKeyPart(parts, entry?.title || "");
          appendRenderKeyPart(parts, "dynamic_title");
          appendRenderKeyPart(parts, entry?.dynamic_title || "");
          appendRenderKeyPart(parts, "dynamic_title_detail");
          appendRenderKeyPart(parts, entry?.dynamic_title_detail || "");
          appendRenderKeyPart(parts, "purpose_title");
          appendRenderKeyPart(parts, entry?.purpose_title || "");
          appendRenderKeyPart(parts, "agent_id");
          appendRenderKeyPart(parts, entry?.agent_id || "");
          appendRenderKeyPart(parts, "agent_color");
          appendRenderKeyPart(parts, entry?.agent_color || "");
          appendRenderKeyPart(parts, "status");
          appendRenderKeyPart(parts, entry?.status || "");
          appendRenderKeyPart(parts, "runtime_state");
          appendRenderKeyPart(parts, runtimeState);
          appendRenderKeyPart(parts, "runtime_label");
          appendRenderKeyPart(parts, windowRuntimeLabel(runtimeState));
          appendRenderKeyPart(parts, "role_badge");
          appendRenderKeyPart(parts, windowRoleBadgeLabel(entry) || "");
          appendRenderKeyPart(parts, "display_title");
          appendRenderKeyPart(parts, windowDisplayTitle(entry));
          appendRenderKeyPart(parts, "title_tooltip");
          appendRenderKeyPart(parts, windowTitleTooltip(entry));
          appendRenderKeyPart(parts, "geometry");
          appendRenderKeyPart(parts, "x");
          appendRenderKeyPart(parts, geometry.x ?? 0);
          appendRenderKeyPart(parts, "y");
          appendRenderKeyPart(parts, geometry.y ?? 0);
          appendRenderKeyPart(parts, "width");
          appendRenderKeyPart(parts, geometry.width ?? 0);
          appendRenderKeyPart(parts, "height");
          appendRenderKeyPart(parts, geometry.height ?? 0);
          appendRenderKeyPart(parts, "geometry_label");
          appendRenderKeyPart(parts, windowGeometryLabel(entry));
          appendRenderKeyPart(parts, "minimized");
          appendRenderKeyPart(parts, Boolean(entry?.minimized));
          appendRenderKeyPart(parts, "maximized");
          appendRenderKeyPart(parts, Boolean(entry?.maximized));
          appendRenderKeyPart(parts, "z_index");
          appendRenderKeyPart(parts, entry?.z_index ?? 0);
          appendRenderKeyPart(parts, "tab_group_id");
          appendRenderKeyPart(parts, entry?.tab_group_id || "");
          appendRenderKeyPart(parts, "tab_group_active");
          appendRenderKeyPart(parts, Boolean(entry?.tab_group_active));
        };
        const parts = [];
        appendRenderKeyPart(parts, "open");
        appendRenderKeyPart(parts, Boolean(windowListOpen));
        appendRenderKeyPart(parts, "active_tab_id");
        appendRenderKeyPart(parts, appState?.active_tab_id || null);
        appendRenderKeyPart(parts, "window_list_entries");
        appendRenderKeyPart(parts, windowListEntries.length);
        for (const entry of windowListEntries) {
          appendEntryKey(parts, entry);
        }
        appendRenderKeyPart(parts, "active_window_ids");
        appendRenderKeyPart(parts, workspaceWindows.length);
        for (const windowData of workspaceWindows) {
          appendRenderKeyPart(parts, windowData?.id || "");
        }
        appendRenderKeyPart(parts, "rows");
        if (windowListEntries.length > 0) {
          let rowCount = 0;
          for (const entry of windowListEntries) {
            if (workspaceWindowMap.size === 0 || workspaceWindowMap.has(entry.id)) {
              rowCount += 1;
            }
          }
          appendRenderKeyPart(parts, rowCount);
          for (const entry of windowListEntries) {
            if (workspaceWindowMap.size === 0 || workspaceWindowMap.has(entry.id)) {
              appendEntryKey(parts, workspaceWindowMap.get(entry.id) || entry);
            }
          }
        } else {
          appendRenderKeyPart(parts, workspaceWindows.length);
          for (const entry of workspaceWindows) {
            appendEntryKey(parts, entry);
          }
        }
        return parts.join("");
      }

      function projectPickerRenderKey(activeTab = activeProjectTab()) {
        const appState = getAppState();
        const projectError = getProjectError();
        const shouldShow = !activeTab;
        const parts = [];
        appendRenderKeyPart(parts, "visible");
        appendRenderKeyPart(parts, shouldShow);
        appendRenderKeyPart(parts, "error");
        appendRenderKeyPart(parts, projectError || "");
        appendRenderKeyPart(parts, "recent_projects");
        appendRenderKeyPart(
          parts,
          shouldShow ? recentProjectsRenderKey(appState) : "",
        );
        return parts.join("");
      }

      function projectOnboardingRenderKey(tab) {
        const visible = Boolean(tab && tab.kind !== "git");
        const parts = [];
        appendRenderKeyPart(parts, "visible");
        appendRenderKeyPart(parts, visible);
        appendRenderKeyPart(parts, "kind");
        appendRenderKeyPart(parts, visible ? tab.kind || "" : "");
        appendRenderKeyPart(parts, "project_root");
        appendRenderKeyPart(parts, visible ? tab.project_root || "" : "");
        return parts.join("");
      }

      function actionAvailabilityRenderKey(activeTab = activeProjectTab()) {
        return activeTab ? "active" : "empty";
      }
      // SPEC-1934 US-6: state for the migration confirmation / progress modal.
      // `tabId` identifies which tab the active migration belongs to so a
      // multi-project frontend never mixes events from different repos.
      let migrationModalState = {
        open: false,
        stage: "confirm", // "confirm" | "running" | "error"
        tabId: null,
        projectRoot: "",
        branch: null,
        hasDirty: false,
        hasLocked: false,
        hasSubmodules: false,
        phase: "confirm",
        percent: 0,
        message: "",
        recovery: "",
      };
      let cloneProjectModalState = {
        open: false,
        mode: "url",
        url: "",
        parentPath: "",
        query: "",
        repositories: [],
        selectedRepositoryUrl: "",
        searching: false,
        cloning: false,
        progress: "",
        error: "",
      };

      function sendOpenProjectDialog() {
        send({ kind: "open_project_dialog" });
      }

      function openCloneProjectModal() {
        cloneProjectModalState = {
          ...cloneProjectModalState,
          open: true,
          error: "",
          progress: "",
        };
        renderProjectCloneModal();
      }

      function closeCloneProjectModal() {
        if (cloneProjectModalState.cloning) {
          return;
        }
        cloneProjectModalState = {
          ...cloneProjectModalState,
          open: false,
          searching: false,
          error: "",
          progress: "",
        };
        renderProjectCloneModal();
      }

      function renderProjectCloneModal() {
        if (!cloneProjectModal || !cloneProjectDialog) {
          return;
        }
        renderProjectCloneModalView({
          modalEl: cloneProjectModal,
          dialogEl: cloneProjectDialog,
          state: cloneProjectModalState,
          createNode: (tagName, className, textContent) => {
            const node = document.createElement(tagName);
            if (className) node.className = className;
            if (textContent !== undefined) node.textContent = textContent;
            return node;
          },
          onClose: closeCloneProjectModal,
          onModeChange: (mode) => {
            cloneProjectModalState = {
              ...cloneProjectModalState,
              mode,
              error: "",
              progress: "",
            };
            renderProjectCloneModal();
          },
          onUrlChange: (url) => {
            cloneProjectModalState = { ...cloneProjectModalState, url };
          },
          onParentSelect: () => {
            send({ kind: "select_clone_project_parent" });
          },
          onSearchQueryChange: (query) => {
            cloneProjectModalState = { ...cloneProjectModalState, query };
          },
          onSearch: () => {
            const query = cloneProjectModalState.query.trim();
            if (!query) return;
            cloneProjectModalState = {
              ...cloneProjectModalState,
              searching: true,
              error: "",
            };
            renderProjectCloneModal();
            send({ kind: "github_repository_search", query });
          },
          onRepositorySelect: (url) => {
            cloneProjectModalState = {
              ...cloneProjectModalState,
              selectedRepositoryUrl: url,
              url,
              error: "",
            };
            renderProjectCloneModal();
          },
          onClone: () => {
            const url = (
              cloneProjectModalState.mode === "search"
                ? cloneProjectModalState.selectedRepositoryUrl
                : cloneProjectModalState.url
            ).trim();
            const parentPath = cloneProjectModalState.parentPath.trim();
            if (!url || !parentPath) {
              cloneProjectModalState = {
                ...cloneProjectModalState,
                error: !url
                  ? "Select or enter a repository URL."
                  : "Choose a destination parent folder.",
              };
              renderProjectCloneModal();
              return;
            }
            cloneProjectModalState = {
              ...cloneProjectModalState,
              cloning: true,
              progress: "Cloning repository...",
              error: "",
            };
            renderProjectCloneModal();
            send({
              kind: "clone_project_start",
              url,
              parent_path: parentPath,
            });
          },
        });
      }

      function updateActionAvailability(activeTab = activeProjectTab()) {
        const nextActionAvailabilityKey = actionAvailabilityRenderKey(activeTab);
        if (renderedActionAvailabilityKey === nextActionAvailabilityKey) {
          return;
        }
        renderedActionAvailabilityKey = nextActionAvailabilityKey;
        const hasActiveTab = nextActionAvailabilityKey === "active";
        addButton.disabled = !hasActiveTab;
        tileButton.disabled = !hasActiveTab;
        stackButton.disabled = !hasActiveTab;
        windowListButton.disabled = !hasActiveTab;
        if (!hasActiveTab) {
          windowListOpen = false;
          windowListEntries = [];
          renderWindowList();
        }
      }

      function escapeHtml(value) {
        return String(value || "")
          .replace(/&/g, "&amp;")
          .replace(/</g, "&lt;")
          .replace(/>/g, "&gt;")
          .replace(/"/g, "&quot;")
          .replace(/'/g, "&#39;");
      }

      function requestWindowList() {
        if (!activeProjectTab()) {
          return;
        }
        send({ kind: "list_windows" });
      }

      function renderWindowList() {
        windowListPanel.hidden = !windowListOpen;
        windowListButton.setAttribute("aria-expanded", windowListOpen ? "true" : "false");
        if (!windowListOpen) {
          renderedWindowListKey = "__closed__";
          return;
        }
        const nextWindowListKey = windowListRenderKey();
        if (renderedWindowListKey === nextWindowListKey) {
          return;
        }
        renderedWindowListKey = nextWindowListKey;
        const workspaceWindows = activeWorkspace().windows || [];
        let entries = workspaceWindows;
        if (windowListEntries.length > 0) {
          const workspaceWindowMap = new Map();
          for (const windowData of workspaceWindows) {
            workspaceWindowMap.set(windowData.id, windowData);
          }
          entries = [];
          for (const entry of windowListEntries) {
            const workspaceEntry = workspaceWindowMap.get(entry.id);
            if (workspaceWindowMap.size === 0 || workspaceEntry) {
              entries.push(workspaceEntry || entry);
            }
          }
        }
        windowListPanel.innerHTML = "";
        if (entries.length === 0) {
          const empty = document.createElement("div");
          empty.className = "window-list-empty";
          empty.textContent = "No windows";
          windowListPanel.appendChild(empty);
          return;
        }
        for (const entry of entries) {
          const row = document.createElement("button");
          row.type = "button";
          row.className = "window-list-row";
          if (entry.agent_color) {
            row.dataset.agentColor = entry.agent_color;
          }
          const geometryLabel = windowGeometryLabel(entry);
          const runtimeState = runtimeStateForWindow(entry);
          const runtimeLabel = windowRuntimeLabel(runtimeState);
          const runtimeChip = shouldShowRuntimeStatus(entry)
            ? `<span class="status-chip ${runtimeState}">
                <span class="status-dot"></span>
                <span class="status-label">${runtimeLabel}</span>
              </span>`
            : "";
          const roleBadgeLabel = windowRoleBadgeLabel(entry);
          const roleBadge = roleBadgeLabel
            ? `<span class="window-role-badge window-list-role">${escapeHtml(roleBadgeLabel)}</span>`
            : "";
          row.innerHTML = `
            <div class="window-list-copy">
              <div class="window-list-title">${escapeHtml(windowDisplayTitle(entry))}</div>
              <div class="window-list-meta">
                ${roleBadge}
                <span class="window-list-geometry">${geometryLabel}</span>
              </div>
            </div>
            ${runtimeChip}
          `;
          const windowListTitle = row.querySelector(".window-list-title");
          if (windowListTitle) windowListTitle.title = windowTitleTooltip(entry);
          // SPEC-2008 camera-focus / FR-094: the persistent switcher row flies
          // the local camera to frame the chosen window (teleport). frameWindow
          // applies local focus + sends focus_window for highlight; there is no
          // maximize/restore anymore.
          row.addEventListener("click", () => {
            windowListOpen = false;
            renderWindowList();
            frameWindow(entry.id);
          });
          windowListPanel.appendChild(row);
        }
      }

      function toggleWindowList() {
        if (windowListButton.disabled) {
          return;
        }
        windowListOpen = !windowListOpen;
        windowListEntries = [];
        renderedWindowListKey = "";
        renderWindowList();
        if (windowListOpen) {
          requestWindowList();
        }
      }

      // SPEC-2008 camera-focus: the maximized-window viewport sync
      // (geometryMatches / syncMaximizedWindowsToViewport /
      // scheduleMaximizedWindowsToViewportSync /
      // workspaceHasVisibleMaximizedWindow) was removed. Windows render at
      // their own geometry and the camera is driven locally by app.js
      // (frameWindow / enterOverview), so there is no shared maximized fill to
      // reconcile per client.

      function renderProjectTabs() {
        const appState = getAppState();
        clearProjectUnread(appState.active_tab_id, { render: false });
        renderProjectTabsView({
          projectTabs,
          tabs: appState.tabs || [],
          activeTabId: appState.active_tab_id,
          runtimeStateForWindow,
          send,
          requestCloseProjectTab,
          onSelectProjectTab: clearProjectUnread,
        });
        renderProjectSwitcherButtonState();
        if (projectSwitcherController.isOpen()) {
          renderProjectSwitcher();
        }
      }

      // SPEC-2013 FR-012 / 2026-06-16 amendment: project tab close always
      // opens the confirm modal. Running agents only change the copy and
      // destructive emphasis. Confirm emits close_project_tab; Cancel never
      // emits a message.
      const closeProjectTabModalEl = document.getElementById(
        "close-project-tab-modal",
      );
      const closeProjectTabModalDialogEl = closeProjectTabModalEl
        ? closeProjectTabModalEl.querySelector(".modal-shell")
        : null;
      let closeProjectTabModalState = {
        open: false,
        tabId: null,
        tabTitle: null,
        runningAgents: [],
      };

      function renderCloseProjectTabModal() {
        if (!closeProjectTabModalEl || !closeProjectTabModalDialogEl) {
          return;
        }
        renderCloseProjectTabConfirmModal({
          modalEl: closeProjectTabModalEl,
          dialogEl: closeProjectTabModalDialogEl,
          state: closeProjectTabModalState,
          createNode,
          onCancel: () => {
            closeProjectTabModalState = {
              open: false,
              tabId: null,
              tabTitle: null,
              runningAgents: [],
            };
            renderCloseProjectTabModal();
          },
          onConfirm: () => {
            const targetId = closeProjectTabModalState.tabId;
            closeProjectTabModalState = {
              open: false,
              tabId: null,
              tabTitle: null,
              runningAgents: [],
            };
            renderCloseProjectTabModal();
            if (targetId) {
              send({ kind: "close_project_tab", tab_id: targetId });
            }
          },
        });
      }

      function requestCloseProjectTab(tabId) {
        const appState = getAppState();
        const tabs = appState.tabs || [];
        const tab = tabs.find((entry) => entry.id === tabId);
        const runningAgents = Array.isArray(tab && tab.running_agents)
          ? tab.running_agents
          : [];
        const count = Number.isFinite(tab && tab.running_agent_count)
          ? tab.running_agent_count
          : runningAgents.length;
        const effectiveRunningAgents =
          runningAgents.length > 0 || count <= 0
            ? runningAgents
            : Array.from({ length: count }, () => ({ display_name: "agent" }));
        closeProjectTabModalState = {
          open: true,
          tabId,
          tabTitle: (tab && tab.title) || null,
          runningAgents: effectiveRunningAgents,
        };
        renderCloseProjectTabModal();
      }

      function updateProjectTabStateCue(buttonEl, tab) {
        updateProjectTabStateCueView(buttonEl, tab, { runtimeStateForWindow });
      }

      function refreshProjectTabStateCues() {
        const appState = getAppState();
        const tabsById = new Map(
          (appState.tabs || []).map((tab) => [tab.id, tab]),
        );
        for (const buttonEl of projectTabs.querySelectorAll(".project-tab")) {
          updateProjectTabStateCue(
            buttonEl,
            tabsById.get(buttonEl.dataset.projectTabId),
          );
        }
      }

      function renderRecentProjects({ force = false } = {}) {
        const appState = getAppState();
        const nextKey = recentProjectsRenderKey(appState);
        if (!force && renderedRecentProjectsKey === nextKey) {
          return;
        }
        renderedRecentProjectsKey = nextKey;
        recentProjectList.replaceChildren();
        const recentProjects = appState?.recent_projects || [];
        if (recentProjects.length === 0) {
          const empty = document.createElement("div");
          empty.className = "file-tree-empty workspace-empty-state";
          empty.textContent = "No recent projects";
          recentProjectList.appendChild(empty);
        } else {
          for (const project of recentProjects) {
            const row = document.createElement("button");
            row.type = "button";
            row.className = "recent-project-row";
            // Issue #2746 follow-up — meta truncates with ellipsis, so the
            // full path lives in the native title tooltip on hover/focus.
            row.title = `${project.kind} · ${project.path}`;
            const titleEl = document.createElement("span");
            titleEl.className = "recent-project-title";
            titleEl.textContent = project.title;
            const metaEl = document.createElement("span");
            metaEl.className = "recent-project-meta";
            metaEl.textContent = `${project.kind} · ${project.path}`;
            row.append(titleEl, metaEl);
            row.addEventListener("click", () => {
              send({ kind: "reopen_recent_project", path: project.path });
            });
            recentProjectList.appendChild(row);
          }
        }
        if (projectSwitcherController.isOpen()) {
          renderProjectSwitcher();
        }
      }

      function renderProjectPicker(activeTab = activeProjectTab()) {
        const projectError = getProjectError();
        const nextProjectPickerKey = projectPickerRenderKey(activeTab);
        if (renderedProjectPickerKey === nextProjectPickerKey) {
          return;
        }
        renderedProjectPickerKey = nextProjectPickerKey;
        const shouldShow = !activeTab;
        projectPicker.classList.toggle("visible", shouldShow);
        projectPickerError.hidden = !projectError;
        projectPickerError.textContent = projectError;
        if (shouldShow) {
          renderRecentProjects();
        }
      }

      function renderProjectOnboarding(tab) {
        const nextProjectOnboardingKey = projectOnboardingRenderKey(tab);
        if (renderedProjectOnboardingKey === nextProjectOnboardingKey) {
          return;
        }
        renderedProjectOnboardingKey = nextProjectOnboardingKey;
        if (!tab || tab.kind === "git") {
          projectOnboarding.classList.remove("visible");
          return;
        }
        projectOnboarding.classList.add("visible");
        projectOnboardingTitle.textContent =
          tab.kind === "bare" ? "Bare repository selected" : "Project setup required";
        projectOnboardingCopy.textContent =
          tab.kind === "bare"
            ? `No develop worktree was found for ${tab.project_root}. Open a worktree folder or another project.`
            : `${tab.project_root} is not a Git workspace yet. Open another project or initialize this folder in your shell first.`;
      }

      // SPEC-1934 US-6: Migration modal entry point. Re-rendered on every
      // BackendEvent::Migration* arrival so the UI mirrors the executor state.
      function renderMigrationModal() {
        if (!migrationModal || !migrationDialog) return;
        renderMigrationModalView({
          modalEl: migrationModal,
          dialogEl: migrationDialog,
          state: { migrationModal: migrationModalState },
          createNode,
          onMigrate: () => {
            if (!migrationModalState.tabId) return;
            const tabId = migrationModalState.tabId;
            migrationModalState.stage = "running";
            migrationModalState.phase = "validate";
            migrationModalState.percent = 0;
            renderMigrationModal();
            send({ kind: "start_migration", tab_id: tabId });
          },
          // SPEC-1934 US-7 / FR-032: only the error stage exposes a dismissal
          // affordance ("Close"), and it must not flip migration_pending —
          // the tab keeps its pending status so the next Open Project shows
          // the Accept-only modal again.
          onClose: () => {
            migrationModalState.open = false;
            migrationModalState.stage = "confirm";
            migrationModalState.message = "";
            migrationModalState.recovery = "";
            renderMigrationModal();
          },
        });
      }

      // SPEC-3064 Phase 3 (E7): receive() body for window_list. The case arm
      // stays in app.js as a thin delegate.
      function applyWindowListEvent(event) {
        windowListEntries = event.windows || [];
        renderWindowList();
      }

      // SPEC-3064 Phase 3 (E7): receive() bodies for the clone-project /
      // GitHub repository search events. The case arms stay in app.js as
      // thin delegates.
      function applyCloneProjectReceiveEvent(event) {
        switch (event.kind) {
          case "clone_project_parent_selected":
            cloneProjectModalState = {
              ...cloneProjectModalState,
              parentPath: event.path || "",
              error: "",
            };
            renderProjectCloneModal();
            break;
          case "github_repository_search_results":
            if (event.query !== cloneProjectModalState.query.trim()) {
              break;
            }
            cloneProjectModalState = {
              ...cloneProjectModalState,
              repositories: event.repositories || [],
              selectedRepositoryUrl: "",
              searching: false,
              error: "",
            };
            renderProjectCloneModal();
            break;
          case "github_repository_search_error":
            if (event.query !== cloneProjectModalState.query.trim()) {
              break;
            }
            cloneProjectModalState = {
              ...cloneProjectModalState,
              searching: false,
              error: event.message || "Repository search failed.",
            };
            renderProjectCloneModal();
            break;
          case "clone_project_progress":
            cloneProjectModalState = {
              ...cloneProjectModalState,
              cloning: true,
              progress: event.message || "Cloning repository...",
              error: "",
            };
            renderProjectCloneModal();
            break;
          case "clone_project_done":
            cloneProjectModalState = {
              ...cloneProjectModalState,
              open: false,
              cloning: false,
              searching: false,
              progress: "",
              error: "",
            };
            renderProjectCloneModal();
            break;
          case "clone_project_error":
            cloneProjectModalState = {
              ...cloneProjectModalState,
              cloning: false,
              progress: "",
              error: event.message || "Clone failed.",
            };
            renderProjectCloneModal();
            break;
          default:
            break;
        }
      }

      // SPEC-3064 Phase 3 (E7): receive() bodies for the migration events.
      // The case arms stay in app.js as thin delegates.
      function applyMigrationReceiveEvent(event) {
        switch (event.kind) {
          case "migration_detected": {
            // SPEC-1934 US-6.1: server says a project tab needs migration.
            migrationModalState = {
              open: true,
              stage: "confirm",
              tabId: event.tab_id,
              projectRoot: event.project_root || "",
              branch: event.branch || null,
              hasDirty: Boolean(event.has_dirty),
              hasLocked: Boolean(event.has_locked),
              hasSubmodules: Boolean(event.has_submodules),
              phase: "confirm",
              percent: 0,
              message: "",
              recovery: "",
            };
            renderMigrationModal();
            break;
          }
          case "migration_progress": {
            // Phase tick from execute_migration. Ignore if the modal already
            // closed (e.g. user pressed Quit) so we never re-open it.
            if (
              !migrationModalState.open ||
              migrationModalState.tabId !== event.tab_id
            ) {
              break;
            }
            migrationModalState.stage = "running";
            migrationModalState.phase = event.phase || "confirm";
            migrationModalState.percent = Number.isFinite(event.percent)
              ? event.percent
              : 0;
            renderMigrationModal();
            break;
          }
          case "migration_done": {
            if (migrationModalState.tabId === event.tab_id) {
              migrationModalState.open = false;
              migrationModalState.stage = "confirm";
              migrationModalState.percent = 0;
              renderMigrationModal();
            }
            break;
          }
          case "migration_error": {
            if (migrationModalState.tabId !== event.tab_id) break;
            migrationModalState.open = true;
            migrationModalState.stage = "error";
            migrationModalState.phase = event.phase || "error";
            migrationModalState.message = event.message || "";
            migrationModalState.recovery = event.recovery || "";
            renderMigrationModal();
            break;
          }
          default:
            break;
        }
      }

      // SPEC-3064 Phase 3 (E7): the migration branch of the app.js global
      // Esc handler. Returns true when the open migration modal consumed
      // the event so the caller stops walking the remaining branches.
      function handleMigrationModalEscape(event) {
        if (migrationModal && migrationModal.classList.contains("open")) {
          // SPEC-1934 US-7 / FR-032: the confirmation modal is Accept-only.
          // Esc no longer routes to skip — at the confirm stage it is
          // swallowed (so the user can't bypass migration), at the error
          // stage it dismisses the failure UI without flipping
          // migration_pending so the next Open Project re-presents Accept.
          if (migrationModalState.stage === "error") {
            migrationModalState.open = false;
            migrationModalState.stage = "confirm";
            migrationModalState.message = "";
            migrationModalState.recovery = "";
            renderMigrationModal();
          }
          event.preventDefault();
          return true;
        }
        return false;
      }

      // SPEC-3064 Phase 3 (E7): the Windows dropdown branch of the app.js
      // global Esc handler. Returns true when the open dropdown consumed
      // the event.
      function handleWindowListEscape(event) {
        if (windowListOpen) {
          // Close the Windows dropdown and return focus to its trigger
          // button (matches the modal pattern of restoring focus to the
          // element that opened the dropdown).
          windowListOpen = false;
          renderWindowList();
          if (windowListButton && typeof windowListButton.focus === "function") {
            try { windowListButton.focus({ preventScroll: true }); }
            catch { windowListButton.focus(); }
          }
          event.preventDefault();
          return true;
        }
        return false;
      }

      function projectShellModalOrDropdownOpen() {
        return (
          Boolean(isModalOpen?.()) ||
          closeProjectTabModalEl?.classList.contains("open") ||
          cloneProjectModal?.classList.contains("open") ||
          migrationModal?.classList.contains("open") ||
          windowListOpen ||
          projectSwitcherController.isOpen()
        );
      }

      function selectProjectTab(tabId) {
        if (!tabId) {
          return;
        }
        clearProjectUnread(tabId);
        send({ kind: "select_project_tab", tab_id: tabId });
      }

      function handleProjectSwitcherShortcut(event) {
        const appState = getAppState();
        if (
          !shouldHandleProjectSwitcherShortcut(event, {
            projectCount: (appState.tabs || []).length,
            modalOpen: projectShellModalOrDropdownOpen(),
          })
        ) {
          return false;
        }
        event.preventDefault();
        if (String(event.key || "").toLowerCase() === "p") {
          projectSwitcherController.open();
          return true;
        }
        const direction = event.key === "ArrowUp" ? "previous" : "next";
        const tabId = nextProjectTabId(
          appState.tabs || [],
          appState.active_tab_id,
          direction,
        );
        if (tabId) {
          selectProjectTab(tabId);
        }
        return true;
      }

      // SPEC-2013 US-10 / FR-025: Cmd+O (mac) / Ctrl+O (Windows/Linux) opens the
      // native folder dialog directly, recovering the one-press Open Folder the
      // retired split button's primary half used to give.
      function handleOpenFolderHotkey(event) {
        if (
          !shouldTriggerOpenFolderHotkey(event, {
            modalOpen: projectShellModalOrDropdownOpen(),
          })
        ) {
          return false;
        }
        event.preventDefault();
        sendOpenProjectDialog();
        return true;
      }

      // SPEC-3064 Phase 3 (E7): project-shell chrome listeners (picker /
      // onboarding entry points, the Windows dropdown trigger, and the dropdown
      // outside-click close) moved out of the app.js bootstrap. References
      // that previously went through frontendUnits.projectWorkspaceShell.*
      // are direct local calls here.
      //
      // SPEC-2013 Phase 8: the standalone Open Project split-button was retired.
      // Open Folder / Clone now live inside the consolidated `Projects ▾`
      // switcher (onOpenFolder / onCloneFromGithub above) and Open Folder is
      // also reachable via the Cmd+O / Ctrl+O hotkey (handleOpenFolderHotkey).
      function installProjectShellChrome() {
        pickerOpenProjectButton.addEventListener("click", sendOpenProjectDialog);
        pickerCloneProjectButton.addEventListener("click", openCloneProjectModal);
        onboardingOpenProjectButton.addEventListener("click", sendOpenProjectDialog);

        // Escape closes the Projects switcher even when focus has left its panel
        // (e.g. returned to the trigger button or the document body).
        document.addEventListener("keydown", (event) => {
          if (event.key === "Escape" && projectSwitcherController.isOpen()) {
            event.preventDefault();
            closeProjectSwitcher({ restoreFocus: true });
          }
        });

        windowListButton.addEventListener("click", toggleWindowList);

        window.addEventListener(
          "keydown",
          (event) => {
            handleProjectSwitcherShortcut(event);
            handleOpenFolderHotkey(event);
          },
          true,
        );

        window.addEventListener("pointerdown", (event) => {
          if (projectSwitcherController.isOpen()) {
            const insideProjectSwitcher =
              projectSwitcherPanel?.contains(event.target) ||
              projectSwitcherButton?.contains(event.target);
            if (!insideProjectSwitcher) {
              closeProjectSwitcher();
            }
          }
          if (!windowListOpen) {
            return;
          }
          if (
            windowListPanel.contains(event.target) ||
            windowListButton.contains(event.target)
          ) {
            return;
          }
          windowListOpen = false;
          renderWindowList();
        });
      }

      return {
        sendOpenProjectDialog,
        closeCloneProjectModal,
        updateActionAvailability,
        requestWindowList,
        renderWindowList,
        toggleWindowList,
        renderProjectTabs,
        refreshProjectTabStateCues,
        markProjectUnread,
        clearProjectUnread,
        renderProjectSwitcher,
        renderRecentProjects,
        renderProjectPicker,
        renderProjectOnboarding,
        applyWindowListEvent,
        applyCloneProjectReceiveEvent,
        applyMigrationReceiveEvent,
        handleMigrationModalEscape,
        handleWindowListEscape,
        installProjectShellChrome,
      };
}
