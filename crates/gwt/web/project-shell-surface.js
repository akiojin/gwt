// SPEC-3064 Phase 3 (E7) — Project & workspace shell chrome surface
// extracted from app.js. Owns the project tab strip (close-tab confirm
// modal + running-agent dots), the Recent Projects list (picker overlay +
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
// - `viewport.zoom` reads go through `getViewport().zoom` (viewport is
//   reassigned on pan/zoom in app.js),
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
// - getAppState() / getProjectError() / getViewport(): accessors for
//   app.js-owned mutable shell state.
// - activeProjectTab() / activeWorkspace(): app.js workspace selectors.
// - windowMap: workspace window element map owned by app.js.
// - appendRenderKeyPart(parts, value): shared render-key primitive (also
//   used by the render keys that stayed in app.js).
// - runtimeStateForWindow / windowDisplayTitle / windowTitleTooltip /
//   windowRoleBadgeLabel / windowGeometryLabel / shouldShowRuntimeStatus /
//   visibleWindowData / presetSurface: window presentation helpers owned
//   by app.js (still used by the core window renderers there).
// - focusWindowRemotely(windowId, opts): focus path owned by app.js.
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
  updateProjectTabDot as updateProjectTabDotView,
} from "/project-tabs-renderer.js";
import { renderCloseProjectTabConfirmModal } from "/close-project-tab-confirm-modal.js";
import { createFocusTrap } from "/focus-trap.js";
import { maximizedGeometry } from "/window-geometry-sync.js";
import { windowRuntimeLabel } from "/window-runtime-state.js";

export function createProjectShellSurface({
  send,
  createNode,
  getAppState,
  getProjectError,
  getViewport,
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
  scheduleTerminalFit,
  visibleBounds,
  addButton,
  tileButton,
  stackButton,
  cloneProjectModal,
  cloneProjectDialog,
  migrationModal,
  migrationDialog,
}) {
      const projectTabs = document.getElementById("project-tabs");
      const openProjectButton = document.getElementById("open-project-button");
      const openProjectMenuButton = document.getElementById(
        "open-project-menu-button",
      );
      const openProjectMenu = document.getElementById("open-project-menu");
      const openProjectMenuOpenItem = document.getElementById(
        "open-project-menu-open",
      );
      const openProjectMenuCloneItem = document.getElementById(
        "open-project-menu-clone",
      );
      const openProjectMenuRecent = document.getElementById(
        "open-project-menu-recent",
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
      let renderedRecentProjectsMenuKey = "";
      let renderedWindowListKey = "";
      let renderedProjectPickerKey = "";
      let renderedProjectOnboardingKey = "";
      let renderedActionAvailabilityKey = "";
      let maximizedViewportSyncFrame = null;

      function recentProjectsRenderKey(state) {
        return JSON.stringify(
          (state?.recent_projects || []).map((project) => ({
            title: project?.title || "",
            kind: project?.kind || "",
            path: project?.path || "",
          })),
        );
      }

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
          row.addEventListener("click", () => {
            windowListOpen = false;
            renderWindowList();
            focusWindowRemotely(entry.id, { center: true });
            if (entry.minimized) {
              send({ kind: "restore_window", id: entry.id });
            }
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

      function geometryMatches(left, right) {
        return (
          Math.abs(left.x - right.x) < 0.5 &&
          Math.abs(left.y - right.y) < 0.5 &&
          Math.abs(left.width - right.width) < 0.5 &&
          Math.abs(left.height - right.height) < 0.5
        );
      }

      function syncMaximizedWindowsToViewport() {
        // A maximized window fills THIS client's viewport locally. Re-apply the
        // fill directly to the element and NEVER send a maximize_window
        // correction: broadcasting one let two clients with different viewport
        // sizes ping-pong the shared maximized geometry forever (the flicker
        // bug). The shared `maximized` flag is enough; the pixel fill is a
        // per-client view concern computed from each client's visibleBounds.
        const fill = maximizedGeometry(visibleBounds(), getViewport().zoom);
        for (const windowData of activeWorkspace().windows || []) {
          if (!windowData.maximized || windowData.minimized) {
            continue;
          }
          const element = windowMap.get(windowData.id);
          if (!element) {
            continue;
          }
          const current = {
            x: parseFloat(element.style.left || "0"),
            y: parseFloat(element.style.top || "0"),
            width: parseFloat(element.style.width || "0"),
            height: parseFloat(element.style.height || "0"),
          };
          if (geometryMatches(current, fill)) {
            continue;
          }
          element.style.left = `${fill.x}px`;
          element.style.top = `${fill.y}px`;
          element.style.width = `${fill.width}px`;
          element.style.height = `${fill.height}px`;
          if (presetSurface(windowData.preset) === "terminal") {
            // Visual re-fit only (persist=false): never round-trip geometry from
            // the sync path, so this client cannot churn the shared state.
            scheduleTerminalFit(windowData.id, false);
          }
        }
      }

      function scheduleMaximizedWindowsToViewportSync() {
        if (maximizedViewportSyncFrame !== null) {
          return;
        }
        maximizedViewportSyncFrame = requestAnimationFrame(() => {
          maximizedViewportSyncFrame = null;
          syncMaximizedWindowsToViewport();
        });
      }

      function workspaceHasVisibleMaximizedWindow(workspace) {
        return (workspace?.windows || []).some(
          (windowData) =>
            Boolean(windowData?.maximized) &&
            !windowData?.minimized &&
            visibleWindowData(windowData),
        );
      }

      function renderProjectTabs() {
        const appState = getAppState();
        renderProjectTabsView({
          projectTabs,
          tabs: appState.tabs || [],
          activeTabId: appState.active_tab_id,
          runtimeStateForWindow,
          send,
          requestCloseProjectTab,
        });
      }

      // SPEC-2013 FR-012: the frontend decides whether the close request
      // needs confirmation. When running_agent_count is 0 we send the
      // existing close_project_tab message immediately; otherwise we open
      // the confirm modal and only emit the message once Close anyway is
      // pressed. Cancel never emits a message.
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
        if (count <= 0) {
          send({ kind: "close_project_tab", tab_id: tabId });
          return;
        }
        closeProjectTabModalState = {
          open: true,
          tabId,
          tabTitle: (tab && tab.title) || null,
          runningAgents,
        };
        renderCloseProjectTabModal();
      }

      function updateProjectTabDot(buttonEl, tab) {
        updateProjectTabDotView(buttonEl, tab, { runtimeStateForWindow });
      }

      function refreshProjectTabDots() {
        const appState = getAppState();
        const tabsById = new Map(
          (appState.tabs || []).map((tab) => [tab.id, tab]),
        );
        for (const buttonEl of projectTabs.querySelectorAll(".project-tab")) {
          updateProjectTabDot(
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
      }

      // Issue #2684 — mirror Recent projects inside the split-button dropdown
      // so users can reach them without first closing every project tab.
      function renderRecentProjectsIntoMenu({ force = false } = {}) {
        if (!openProjectMenuRecent) {
          return;
        }
        const appState = getAppState();
        const nextKey = recentProjectsRenderKey(appState);
        if (!force && renderedRecentProjectsMenuKey === nextKey) {
          return;
        }
        renderedRecentProjectsMenuKey = nextKey;
        openProjectMenuRecent.replaceChildren();
        const recentProjects = appState?.recent_projects || [];
        if (recentProjects.length === 0) {
          openProjectMenuRecent.dataset.empty = "true";
          // Real DOM node beats CSS pseudo-content so screen readers reach it.
          const empty = document.createElement("div");
          empty.className = "split-button-menu-empty";
          empty.setAttribute("role", "presentation");
          empty.textContent = "No recent projects";
          openProjectMenuRecent.appendChild(empty);
          return;
        }
        delete openProjectMenuRecent.dataset.empty;
        for (const project of recentProjects) {
          const row = document.createElement("button");
          row.type = "button";
          row.className =
            "split-button-menu-item split-button-menu-recent-row";
          row.setAttribute("role", "menuitem");
          row.tabIndex = -1;
          // Issue #2746 follow-up — meta truncates with ellipsis, so the
          // full path lives in the native title tooltip on hover/focus.
          row.title = `${project.kind} · ${project.path}`;
          const titleEl = document.createElement("span");
          titleEl.className = "split-button-menu-recent-title";
          titleEl.textContent = project.title;
          const metaEl = document.createElement("span");
          metaEl.className = "split-button-menu-recent-meta";
          metaEl.textContent = `${project.kind} · ${project.path}`;
          row.append(titleEl, metaEl);
          row.addEventListener("click", () => {
            closeOpenProjectMenu();
            send({ kind: "reopen_recent_project", path: project.path });
          });
          openProjectMenuRecent.appendChild(row);
        }
      }

      // Issue #2684 — split-button dropdown that mirrors the picker so users
      // can reach Clone from GitHub while a project tab is active.
      let openProjectMenuFocusRelease = null;

      function isOpenProjectMenuOpen() {
        return openProjectMenu?.classList.contains("open") || false;
      }

      function openOpenProjectMenu() {
        if (!openProjectMenu || isOpenProjectMenuOpen()) {
          return;
        }
        renderRecentProjectsIntoMenu({ force: true });
        openProjectMenu.classList.add("open");
        openProjectMenu.setAttribute("aria-hidden", "false");
        openProjectMenuButton.setAttribute("aria-expanded", "true");
        try {
          openProjectMenuOpenItem.focus({ preventScroll: true });
        } catch {
          openProjectMenuOpenItem.focus();
        }
        openProjectMenuFocusRelease = createFocusTrap(openProjectMenu, {
          document,
        });
      }

      function closeOpenProjectMenu({ restoreFocus = false } = {}) {
        if (!openProjectMenu || !isOpenProjectMenuOpen()) {
          return;
        }
        openProjectMenu.classList.remove("open");
        openProjectMenu.setAttribute("aria-hidden", "true");
        openProjectMenuButton.setAttribute("aria-expanded", "false");
        if (typeof openProjectMenuFocusRelease === "function") {
          openProjectMenuFocusRelease();
        }
        openProjectMenuFocusRelease = null;
        if (restoreFocus) {
          try {
            openProjectMenuButton.focus({ preventScroll: true });
          } catch {
            openProjectMenuButton.focus();
          }
        }
      }

      function toggleOpenProjectMenu() {
        if (isOpenProjectMenuOpen()) {
          closeOpenProjectMenu({ restoreFocus: true });
        } else {
          openOpenProjectMenu();
        }
      }

      // Issue #2684 — roving focus across menu items. Items carry
      // tabindex="-1" by ARIA APG convention so Tab does not stop on each
      // individual entry; arrow keys take over once the menu is open.
      function openProjectMenuItems() {
        if (!openProjectMenu) {
          return [];
        }
        return Array.from(
          openProjectMenu.querySelectorAll('[role="menuitem"]'),
        ).filter((el) => !el.disabled);
      }

      function focusOpenProjectMenuItemAt(index) {
        const items = openProjectMenuItems();
        if (items.length === 0) {
          return;
        }
        const wrapped = ((index % items.length) + items.length) % items.length;
        try {
          items[wrapped].focus({ preventScroll: true });
        } catch {
          items[wrapped].focus();
        }
      }

      function moveOpenProjectMenuFocus(direction) {
        const items = openProjectMenuItems();
        if (items.length === 0) {
          return;
        }
        const active = document.activeElement;
        const currentIndex = items.indexOf(active);
        const nextIndex =
          currentIndex === -1
            ? direction > 0
              ? 0
              : items.length - 1
            : currentIndex + direction;
        focusOpenProjectMenuItemAt(nextIndex);
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

      // SPEC-3064 Phase 3 (E7): project-shell chrome listeners (Open Project
      // button + picker/onboarding entry points, the Open Project
      // split-button menu, the Windows dropdown trigger, and the dropdown
      // outside-click close) moved out of the app.js bootstrap. References
      // that previously went through frontendUnits.projectWorkspaceShell.*
      // are direct local calls here.
      function installProjectShellChrome() {
        openProjectButton.addEventListener("click", sendOpenProjectDialog);
        pickerOpenProjectButton.addEventListener("click", sendOpenProjectDialog);
        pickerCloneProjectButton.addEventListener("click", openCloneProjectModal);
        onboardingOpenProjectButton.addEventListener("click", sendOpenProjectDialog);

        // Issue #2684 — split-button caret toggles the dropdown menu, and each
        // menu item routes to the same handler the project picker uses. Outside
        // clicks and Escape close the menu so it never strands keyboard users.
        if (openProjectMenuButton && openProjectMenu) {
          openProjectMenuButton.addEventListener("click", (event) => {
            event.stopPropagation();
            toggleOpenProjectMenu();
          });
          openProjectMenuButton.addEventListener("keydown", (event) => {
            if (event.key === "ArrowDown" || event.key === "Down") {
              event.preventDefault();
              openOpenProjectMenu();
            }
          });
          openProjectMenuOpenItem.addEventListener("click", () => {
            closeOpenProjectMenu();
            sendOpenProjectDialog();
          });
          openProjectMenuCloneItem.addEventListener("click", () => {
            closeOpenProjectMenu();
            openCloneProjectModal();
          });
          document.addEventListener("click", (event) => {
            if (!isOpenProjectMenuOpen()) {
              return;
            }
            if (
              openProjectMenu.contains(event.target) ||
              openProjectMenuButton.contains(event.target)
            ) {
              return;
            }
            closeOpenProjectMenu();
          });
          document.addEventListener("keydown", (event) => {
            if (event.key === "Escape" && isOpenProjectMenuOpen()) {
              event.preventDefault();
              closeOpenProjectMenu({ restoreFocus: true });
            }
          });
          openProjectMenu.addEventListener("keydown", (event) => {
            if (!isOpenProjectMenuOpen()) {
              return;
            }
            switch (event.key) {
              case "ArrowDown":
                event.preventDefault();
                moveOpenProjectMenuFocus(1);
                break;
              case "ArrowUp":
                event.preventDefault();
                moveOpenProjectMenuFocus(-1);
                break;
              case "Home":
                event.preventDefault();
                focusOpenProjectMenuItemAt(0);
                break;
              case "End":
                event.preventDefault();
                focusOpenProjectMenuItemAt(-1);
                break;
              default:
                break;
            }
          });
        }

        windowListButton.addEventListener("click", toggleWindowList);

        window.addEventListener("pointerdown", (event) => {
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
        syncMaximizedWindowsToViewport,
        scheduleMaximizedWindowsToViewportSync,
        workspaceHasVisibleMaximizedWindow,
        renderProjectTabs,
        refreshProjectTabDots,
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
