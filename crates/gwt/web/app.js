      import { Terminal } from "/assets/xterm/xterm.mjs";
      import { FitAddon } from "/assets/xterm/addon-fit.mjs";
      import { renderBranchCleanupModal as renderBranchCleanupModalView } from "/branch-cleanup-modal.js";
      import { renderMigrationModal as renderMigrationModalView } from "/migration-modal.js";
      import { initOperatorShell, applyTelemetryCounts } from "/operator-shell.js";
      import { createFocusTrap } from "/focus-trap.js";
      import {
        TITLEBAR_DOCK_HIT_HEIGHT,
        findTitlebarDockTarget,
      } from "/window-docking.js";
      import {
        applyBoardMentionNotificationFocus,
        boardEntryAudienceLabels,
        boardEntryMentionsSelf,
        boardEntryPreview,
        findBoardEntry,
        mentionsForBoardSubmit,
        visibleBoardEntries,
      } from "/board-surface.js";
      import { createUpdateCtaController } from "/update-cta.js";
      import { createTerminalContextMenuController } from "/terminal-context-menu.js";

      // SPEC-2356 Operator Design System — boot the chrome shell as soon as the
      // module loads so the theme toggle, command palette, hotkey overlay,
      // status strip clock, and Mission Briefing intro are wired before the
      // rest of app.js continues bootstrapping the legacy surfaces.
      let __op;
      try {
        __op = initOperatorShell();
      } catch (error) {
        console.error("operator shell failed during startup", error);
        dismissOperatorBriefing();
        __op = {
          themeManager: null,
          hotkey: null,
          palette: null,
        };
      }
      window.__operatorShell = {
        themeManager: __op.themeManager,
        hotkey: __op.hotkey,
        palette: __op.palette,
        applyTelemetryCounts: (counts) => applyTelemetryCounts(document, counts),
      };

      const canvas = document.getElementById("canvas");
      const stage = document.getElementById("canvas-stage");
      const projectTabs = document.getElementById("project-tabs");
      const openProjectButton = document.getElementById("open-project-button");
      const projectPicker = document.getElementById("project-picker");
      const projectPickerError = document.getElementById("project-picker-error");
      const pickerOpenProjectButton = document.getElementById("picker-open-project");
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
      const addButton = document.getElementById("add-button");
      const tileButton = document.getElementById("tile-button");
      const stackButton = document.getElementById("stack-button");
      const alignButton = document.getElementById("align-button");
      const windowListButton = document.getElementById("window-list-button");
      const windowListPanel = document.getElementById("window-list-panel");
      const worldGrid = document.getElementById("canvas-world-grid");
      const activeWorkSection = document.getElementById("op-active-work");
      const activeWorkCount = document.getElementById("op-active-work-count");
      const activeWorkSummary = document.getElementById("op-active-work-summary");
      const activeWorkAgents = document.getElementById("op-active-work-agents");
      const workspaceOverviewEntry = document.getElementById("op-workspace-overview-entry");
      const projectWorkspaceOverviewButton = document.getElementById(
        "project-workspace-overview-button",
      );
      const zoomOutButton = document.getElementById("zoom-out-button");
      const zoomResetButton = document.getElementById("zoom-reset-button");
      const zoomInButton = document.getElementById("zoom-in-button");
      const modal = document.getElementById("preset-modal");
      const closeModalButton = document.getElementById("close-modal");
      const wizardModal = document.getElementById("wizard-modal");
      const wizardDialog = wizardModal.querySelector(".modal-shell");
      const wizardTitle = document.getElementById("wizard-title");
      const wizardMeta = document.getElementById("wizard-meta");
      const wizardSummary = document.getElementById("wizard-summary");
      const wizardBody = document.getElementById("wizard-body");
      const wizardError = document.getElementById("wizard-error");
      const wizardCloseButton = document.getElementById("wizard-close-button");
      const wizardCancelButton = document.getElementById("wizard-cancel-button");
      const wizardSubmitButton = document.getElementById("wizard-submit-button");
      const branchCleanupModal = document.getElementById("branch-cleanup-modal");
      const branchCleanupDialog = branchCleanupModal.querySelector(".modal-shell");
      const migrationModal = document.getElementById("migration-modal");
      const migrationDialog = migrationModal
        ? migrationModal.querySelector(".modal-shell")
        : null;
      const connectionDot = document.getElementById("connection-dot");
      const connectionLabel = document.getElementById("connection-label");
      const appVersionLabel = document.getElementById("app-version");
      const indexStatusLabel = document.getElementById("index-status");

      const decoderMap = new Map();
      const pendingOutputMap = new Map();
      const pendingSnapshotMap = new Map();
      const detailMap = new Map();
      const windowRuntimeStateMap = new Map();
      const terminalMap = new Map();
      const windowMap = new Map();
      const fileTreeStateMap = new Map();
      const branchListStateMap = new Map();
      const profileStateMap = new Map();
      const boardStateMap = new Map();
      const logStateMap = new Map();
      const knowledgeBridgeStateMap = new Map();
      let nextKnowledgeLoadRequestId = 1;
      let nextKnowledgeSearchRequestId = 1;
      const pendingMessages = [];
      // Phase 1B extraction map: each entry names the surface that owns the
      // backing state today. New helpers should mutate state through the owning
      // surface instead of adding cross-surface writes here.
      const frontendStateOwners = Object.freeze({
        terminalMap: Object.freeze({
          owner: "terminal-host",
          mutatedBy: Object.freeze([
            "createTerminalRuntime",
            "fitTerminal",
            "writeOutput",
            "replaceTerminalSnapshot",
            "applyStatus",
          ]),
        }),
        windowMap: Object.freeze({
          owner: "workspace-window-manager",
          mutatedBy: Object.freeze([
            "ensureWindow",
            "mountWindowBody",
            "renderWorkspace",
          ]),
        }),
        fileTreeStateMap: Object.freeze({
          owner: "file-tree-surface",
          mutatedBy: Object.freeze([
            "ensureFileTreeState",
            "renderFileTree",
          ]),
        }),
        branchListStateMap: Object.freeze({
          owner: "branches-surface",
          mutatedBy: Object.freeze([
            "ensureBranchListState",
            "renderBranches",
          ]),
        }),
        profileStateMap: Object.freeze({
          owner: "profile-surface",
          mutatedBy: Object.freeze([
            "ensureProfileState",
            "requestProfile",
            "renderProfile",
            "createProfile",
            "flushProfileSave",
            "deleteProfile",
          ]),
        }),
        boardStateMap: Object.freeze({
          owner: "board-surface",
          mutatedBy: Object.freeze([
            "ensureBoardState",
            "renderBoard",
            "requestBoard",
            "requestOlderBoardEntries",
            "submitBoardEntry",
            "focusBoardEntry",
          ]),
        }),
        logStateMap: Object.freeze({
          owner: "logs-surface",
          mutatedBy: Object.freeze([
            "ensureLogState",
            "requestLogs",
            "renderLogs",
            "appendLiveLogEntry",
            "jumpToUnreadLogs",
          ]),
        }),
        knowledgeBridgeStateMap: Object.freeze({
          owner: "knowledge-bridge-surface",
          mutatedBy: Object.freeze([
            "ensureKnowledgeBridgeState",
            "renderKnowledgeBridge",
          ]),
        }),
        wizardState: Object.freeze({
          owner: "launch-wizard-surface",
          state: Object.freeze([
            "launchWizard",
            "wizardWasOpen",
            "wizardBranchDraft",
            "wizardBranchBackendValue",
          ]),
          mutatedBy: Object.freeze([
            "openIssueLaunchWizard",
            "sendWizardAction",
            "renderLaunchWizard",
            "flushWizardBranchDraft",
            "syncWizardDraftState",
          ]),
        }),
        projectState: Object.freeze({
          owner: "project-workspace-shell",
          state: Object.freeze([
            "appState",
            "versionState",
            "projectError",
            "viewport",
            "windowListOpen",
            "windowListEntries",
          ]),
          mutatedBy: Object.freeze([
            "renderAppState",
            "renderWorkspace",
            "renderProjectTabs",
            "renderProjectPicker",
            "renderProjectOnboarding",
            "renderWindowList",
          ]),
        }),
      });

      function dismissOperatorBriefing() {
        const briefing = document.getElementById("op-briefing");
        if (!briefing) return;
        briefing.dataset.state = "exiting";
        briefing.hidden = true;
        briefing.setAttribute("aria-hidden", "true");
      }

      // Diagnostic counter for intermittent key-input drops (bugfix/input-key).
      // Incremented on every `terminal.onData` firing so layer-by-layer counts
      // can be diffed against backend `gwt_input_trace` logs.
      let inputTraceSeq = 0;

      let socket = null;
      let reconnectTimer = null;
      let focusedId = null;
      let dragState = null;
      let windowTabDragState = null;
      let panState = null;
      let resizeState = null;
      let viewport = { x: 0, y: 0, zoom: 1 };
      let viewportRasterTimer = null;
      let launchWizard = null;
      let activeWorkProjection = null;
      let pendingBoardEntryFocusId = null;
      let wizardWasOpen = false;
      let wizardBranchDraft = "";
      let wizardBranchBackendValue = "";
      let branchCleanupWindowId = null;
      const WORKSPACE_CLEANUP_WINDOW_ID = "__workspace_cleanup__";
      let windowListOpen = false;
      let windowListEntries = [];
      let titlebarClickState = null;
      let appState = {
        app_version: "",
        tabs: [],
        active_tab_id: null,
        recent_projects: [],
      };
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
      let versionState = { current: "", latest: "" };
      const indexStatusByProjectRoot = new Map();
      let projectError = "";
      const TERMINAL_SELECTION_DRAG_THRESHOLD = 4;

      function renderIndexStatus() {
        const activeProjectRoot = activeProjectTab()?.project_root || "";
        const indexStatusState =
          (activeProjectRoot && indexStatusByProjectRoot.get(activeProjectRoot)) || {
            state: "",
            detail: "",
          };
        const state = indexStatusState.state || "";
        indexStatusLabel.hidden = !state || state === "skipped";
        indexStatusLabel.className = `index-status ${state}`;
        const label =
          state === "ready"
            ? "Index: ready"
            : state === "repair_required"
              ? "Index: repair"
              : state === "error"
                ? "Index: error"
                : "Index: checking";
        indexStatusLabel.textContent = label;
        indexStatusLabel.title = indexStatusState.detail || label;
      }

      function setIndexStatus(projectRoot, status) {
        if (!projectRoot) {
          return;
        }
        indexStatusByProjectRoot.set(projectRoot, {
          state: status?.state || "",
          detail: status?.detail || "",
        });
        renderIndexStatus();
      }

      function formatVersionLabel() {
        const current = versionState.current;
        const latest = versionState.latest;
        if (!current) {
          return "";
        }
        if (latest && latest !== current) {
          return `v${current} -> v${latest}`;
        }
        return `v${current}`;
      }

      function renderAppVersion() {
        const label = formatVersionLabel();
        appVersionLabel.hidden = !label;
        appVersionLabel.textContent = label;
        appVersionLabel.title = label;
      }

      function setVersionState(current, latest = null) {
        if (current) {
          versionState.current = current;
        }
        if (!latest || latest === versionState.current) {
          versionState.latest = "";
        } else {
          versionState.latest = latest;
        }
        renderAppVersion();
      }

      function presetSurface(preset) {
        if (
          preset === "shell" ||
          preset === "claude" ||
          preset === "codex" ||
          preset === "agent"
        ) {
          return "terminal";
        }
        if (preset === "file_tree") {
          return "file-tree";
        }
        if (preset === "branches") {
          return "branches";
        }
        if (preset === "profile") {
          return "profile";
        }
        if (preset === "board") {
          return "board";
        }
        if (preset === "logs") {
          return "logs";
        }
        if (preset === "issue" || preset === "spec" || preset === "pr") {
          return "knowledge";
        }
        return "mock";
      }

      function knowledgeKindForPreset(preset) {
        if (preset === "issue" || preset === "spec" || preset === "pr") {
          return preset;
        }
        return null;
      }

      function send(message) {
        if (socket && socket.readyState === WebSocket.OPEN) {
          socket.send(JSON.stringify(message));
          return;
        }
        pendingMessages.push(message);
      }

      const updateCtaController = createUpdateCtaController({
        document,
        send,
        setVersionState,
        confirmUpdate: (version) =>
          window.confirm(`Apply update to v${version} now?\n\ngwt will restart automatically.`),
      });

      function setConnectionState(connected) {
        connectionDot.classList.toggle("connected", connected);
        connectionLabel.textContent = connected ? "Connected" : "Reconnecting";
        // SPEC-2356 — propagate connection state to the Operator Status Strip
        // so the LIVE cell visibly reflects whether the WebSocket bridge is
        // up. The class is set on the strip element and consumed via CSS.
        const strip = document.getElementById("op-status-strip");
        if (strip) {
          strip.classList.toggle("op-status-strip--offline", !connected);
        }
        if (!connected) {
          for (const [windowId] of branchListStateMap.entries()) {
            if (
              failRunningBranchCleanup(
                windowId,
                "Connection lost while cleaning up branches",
              )
            ) {
              renderBranches(windowId);
            }
          }
        }
      }

      function websocketUrl() {
        const url = new URL(window.location.href);
        url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
        url.pathname = "/ws";
        url.search = "";
        url.hash = "";
        return url.toString();
      }

      function handleSocketOpen() {
        setConnectionState(true);
        send({ kind: "frontend_ready" });
        while (pendingMessages.length > 0) {
          socket.send(JSON.stringify(pendingMessages.shift()));
        }
      }

      function handleSocketMessage(event) {
        receive(JSON.parse(event.data));
      }

      function handleSocketClose() {
        setConnectionState(false);
        if (reconnectTimer) {
          clearTimeout(reconnectTimer);
        }
        reconnectTimer = window.setTimeout(connectSocket, 1000);
      }

      function installSocketEventHandlers(activeSocket) {
        activeSocket.addEventListener("open", handleSocketOpen);
        activeSocket.addEventListener("message", handleSocketMessage);
        activeSocket.addEventListener("close", handleSocketClose);
      }

      function connectSocket() {
        if (socket && socket.readyState <= WebSocket.OPEN) {
          return;
        }
        socket = new WebSocket(websocketUrl());
        setConnectionState(false);
        installSocketEventHandlers(socket);
      }

      function emptyWorkspace() {
        return {
          viewport: { x: 0, y: 0, zoom: 1 },
          windows: [],
        };
      }

      function activeProjectTab() {
        if (!appState?.tabs?.length) {
          return null;
        }
        return (
          appState.tabs.find((tab) => tab.id === appState.active_tab_id) ||
          appState.tabs[0] ||
          null
        );
      }

      function activeWorkspace() {
        return activeProjectTab()?.workspace || emptyWorkspace();
      }

      function workspaceWindowById(windowId) {
        return activeWorkspace().windows.find((windowData) => windowData.id === windowId) || null;
      }

      function windowGroupId(windowData) {
        return windowData?.tab_group_id || windowData?.id || "";
      }

      function windowTabsFor(windowData) {
        const groupId = windowGroupId(windowData);
        return (activeWorkspace().windows || []).filter(
          (candidate) => windowGroupId(candidate) === groupId,
        );
      }

      function visibleWindowData(windowData) {
        if (!windowData?.tab_group_id) {
          return true;
        }
        return Boolean(windowData.tab_group_active);
      }

      function detachGeometryFromPointer(event, windowData) {
        const bounds = visibleBounds();
        const width = windowData?.geometry?.width || 720;
        const height = windowData?.geometry?.height || 420;
        const canvasRect = canvas.getBoundingClientRect();
        const worldX = bounds.x + (event.clientX - canvasRect.left) / viewport.zoom;
        const worldY = bounds.y + (event.clientY - canvasRect.top) / viewport.zoom;
        return {
          x: worldX - 32,
          y: worldY - 19,
          width,
          height,
        };
      }

      function pointerWorldPoint(event) {
        const bounds = visibleBounds();
        const canvasRect = canvas.getBoundingClientRect();
        return {
          x: bounds.x + (event.clientX - canvasRect.left) / viewport.zoom,
          y: bounds.y + (event.clientY - canvasRect.top) / viewport.zoom,
        };
      }

      function titlebarDockTargetAt(event, sourceId) {
        const point = pointerWorldPoint(event);
        return findTitlebarDockTarget(
          activeWorkspace().windows || [],
          point,
          sourceId,
          TITLEBAR_DOCK_HIT_HEIGHT,
        );
      }

      function clearTitlebarDockPreview() {
        for (const element of windowMap.values()) {
          element.classList.remove("dock-target");
        }
      }

      function updateTitlebarDockPreview(event) {
        if (!dragState || !dragState.moved || !dragState.allowMove) {
          clearTitlebarDockPreview();
          if (dragState) {
            dragState.dockTargetId = null;
          }
          return null;
        }
        const targetId = titlebarDockTargetAt(event, dragState.id);
        if (dragState.dockTargetId === targetId) {
          return targetId;
        }
        if (dragState.dockTargetId) {
          windowMap.get(dragState.dockTargetId)?.classList.remove("dock-target");
        }
        if (targetId) {
          windowMap.get(targetId)?.classList.add("dock-target");
        }
        dragState.dockTargetId = targetId;
        return targetId;
      }

      function sendOpenProjectDialog() {
        send({ kind: "open_project_dialog" });
      }

      function updateActionAvailability() {
        const hasActiveTab = Boolean(activeProjectTab());
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

      function presetLabel(preset) {
        return preset
          .split("_")
          .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
          .join(" ");
      }

      function presetRoleLabel(preset) {
        const labels = {
          shell: "Shell",
          claude: "Claude",
          codex: "Codex",
          agent: "Agent",
          file_tree: "File Tree",
          branches: "Branches",
          settings: "Settings",
          profile: "Profile",
          logs: "Logs",
          issue: "Issue",
          spec: "SPEC",
          board: "Board",
          pr: "PR",
        };
        return labels[preset] || presetLabel(preset);
      }

      function shouldShowRuntimeStatus(windowData) {
        return presetSurface(windowData?.preset) === "terminal";
      }

      const WINDOW_RUNTIME_STATE_LABELS = Object.freeze({
        running: "Running",
        waiting: "Waiting",
        stopped: "Stopped",
        error: "Error",
      });

      const LEGACY_WINDOW_RUNTIME_STATE_ALIASES = Object.freeze({
        starting: "running",
        ready: "waiting",
        exited: "stopped",
      });

      function presetSupportsWaitingStatus(preset) {
        return preset === "agent" || preset === "claude" || preset === "codex";
      }

      function normalizeWindowRuntimeState(status, preset) {
        const rawState = String(status || "running").toLowerCase();
        const normalizedState = LEGACY_WINDOW_RUNTIME_STATE_ALIASES[rawState] || rawState;
        if (!presetSupportsWaitingStatus(preset) && normalizedState === "waiting") {
          return "running";
        }
        if (!WINDOW_RUNTIME_STATE_LABELS[normalizedState]) {
          return "running";
        }
        return normalizedState;
      }

      function windowGeometryLabel(windowData) {
        if (windowData.minimized) {
          return "Minimized";
        }
        if (windowData.maximized) {
          return "Maximized";
        }
        return "Normal";
      }

      function windowRuntimeLabel(status) {
        return WINDOW_RUNTIME_STATE_LABELS[status] || WINDOW_RUNTIME_STATE_LABELS.running;
      }

      function windowDisplayTitle(windowData) {
        const candidates = [
          windowData?.dynamic_title,
          windowData?.purpose_title,
          windowData?.title,
          windowData?.agent_id,
        ];
        for (const value of candidates) {
          const title = String(value || "").trim();
          if (title) return title;
        }
        return "Window";
      }

      function windowTitleTooltip(windowData) {
        const detail = String(windowData?.dynamic_title_detail || "").trim();
        if (detail) return detail;
        return windowDisplayTitle(windowData);
      }

      function escapeHtml(value) {
        return String(value || "")
          .replace(/&/g, "&amp;")
          .replace(/</g, "&lt;")
          .replace(/>/g, "&gt;")
          .replace(/"/g, "&quot;")
          .replace(/'/g, "&#39;");
      }

      function runtimeStateForWindow(windowData) {
        const cachedState = windowRuntimeStateMap.get(windowData.id);
        if (cachedState) {
          return cachedState;
        }
        return normalizeWindowRuntimeState(windowData.status, windowData.preset);
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
          return;
        }
        const workspaceWindows = activeWorkspace().windows || [];
        const workspaceWindowMap = new Map(
          workspaceWindows.map((windowData) => [windowData.id, windowData]),
        );
        const entries =
          windowListEntries.length > 0
            ? windowListEntries
                .map((entry) => workspaceWindowMap.get(entry.id) || entry)
                .filter((entry) => workspaceWindowMap.size === 0 || workspaceWindowMap.has(entry.id))
            : workspaceWindows;
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
          row.innerHTML = `
            <div class="window-list-copy">
              <div class="window-list-title">${escapeHtml(windowDisplayTitle(entry))}</div>
              <div class="window-list-meta">
                <span class="window-role-badge window-list-role">${presetRoleLabel(entry.preset)}</span>
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
        renderWindowList();
        if (windowListOpen) {
          requestWindowList();
        }
      }

      function maximizedGeometry(bounds) {
        return {
          x: bounds.x + 24,
          y: bounds.y + 24,
          width: Math.max(bounds.width - 48, 0),
          height: Math.max(bounds.height - 48, 0),
        };
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
        const bounds = visibleBounds();
        const nextGeometry = maximizedGeometry(bounds);
        for (const windowData of activeWorkspace().windows || []) {
          if (!windowData.maximized) {
            continue;
          }
          if (geometryMatches(windowData.geometry, nextGeometry)) {
            continue;
          }
          send({
            kind: "maximize_window",
            id: windowData.id,
            bounds,
          });
        }
      }

      function renderProjectTabs() {
        projectTabs.innerHTML = "";
        for (const tab of appState.tabs || []) {
          const button = document.createElement("div");
          button.className = "project-tab";
          button.title = tab.project_root;
          button.setAttribute("role", "button");
          button.tabIndex = 0;
          // SPEC-2356 — aria-current="page" announces the active tab to
          // screen readers without changing the role semantics. Inactive
          // tabs explicitly clear the attribute so the previously-active
          // tab doesn't keep the marker after a switch.
          if (tab.id === appState.active_tab_id) {
            button.classList.add("active");
            button.setAttribute("aria-current", "page");
          } else {
            button.removeAttribute("aria-current");
          }
          button.innerHTML = `
            <span class="project-tab-label">${tab.title}</span>
            <button class="project-tab-close" type="button" aria-label="Close ${tab.title}">×</button>
          `;
          button.addEventListener("click", () => {
            send({ kind: "select_project_tab", tab_id: tab.id });
          });
          button.addEventListener("keydown", (event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              send({ kind: "select_project_tab", tab_id: tab.id });
            }
          });
          button
            .querySelector(".project-tab-close")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              send({ kind: "close_project_tab", tab_id: tab.id });
            });
          projectTabs.appendChild(button);
        }
      }

      function renderRecentProjects() {
        recentProjectList.innerHTML = "";
        const recentProjects = appState?.recent_projects || [];
        if (recentProjects.length === 0) {
          const empty = document.createElement("div");
          empty.className = "file-tree-empty workspace-empty-state";
          empty.textContent = "No recent projects";
          recentProjectList.appendChild(empty);
          return;
        }

        for (const project of recentProjects) {
          const row = document.createElement("button");
          row.type = "button";
          row.className = "recent-project-row";
          row.innerHTML = `
            <span class="recent-project-title">${project.title}</span>
            <span class="recent-project-meta">${project.kind} · ${project.path}</span>
          `;
          row.addEventListener("click", () => {
            send({ kind: "reopen_recent_project", path: project.path });
          });
          recentProjectList.appendChild(row);
        }
      }

      function renderProjectPicker() {
        const shouldShow = !activeProjectTab();
        projectPicker.classList.toggle("visible", shouldShow);
        projectPickerError.hidden = !projectError;
        projectPickerError.textContent = projectError;
        renderRecentProjects();
      }

      function renderProjectOnboarding(tab) {
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

      function renderAppState(nextState) {
        dismissOperatorBriefing();
        appState = nextState || {
          app_version: "",
          tabs: [],
          active_tab_id: null,
          recent_projects: [],
        };
        setVersionState(appState.app_version, versionState.latest);
        renderProjectTabs();
        renderProjectPicker();
        renderIndexStatus();
        updateActionAvailability();
        const tab = activeProjectTab();
        renderProjectOnboarding(tab);
        renderWorkspace(tab?.workspace || emptyWorkspace());
        renderWindowList();
        renderActiveWorkOverview();
      }

      let presetModalFocusReturn = null;
      let presetModalFocusTrapRelease = null;
      function openModal() {
        // SPEC-2356 — capture trigger BEFORE adding .open so we can
        // restore focus on close. The preset modal is invoked via the
        // "+" Add Window button; without restore, focus falls to body.
        presetModalFocusReturn = document.activeElement;
        modal.classList.add("open");
        modal.removeAttribute("aria-hidden");
        const presetShell = modal.querySelector(".modal-shell");
        if (presetShell && typeof presetShell.focus === "function") {
          try { presetShell.focus({ preventScroll: true }); }
          catch { presetShell.focus(); }
        }
        // SPEC-2356 — trap Tab inside the modal so keyboard users can't
        // escape into background content while the modal is open.
        if (presetShell) {
          presetModalFocusTrapRelease = createFocusTrap(presetShell, { document });
        }
      }

      function closeModal() {
        const wasOpenPreset = modal.classList.contains("open");
        modal.classList.remove("open");
        modal.setAttribute("aria-hidden", "true");
        if (wasOpenPreset) {
          if (typeof presetModalFocusTrapRelease === "function") {
            presetModalFocusTrapRelease();
            presetModalFocusTrapRelease = null;
          }
          if (presetModalFocusReturn && typeof presetModalFocusReturn.focus === "function") {
            try { presetModalFocusReturn.focus({ preventScroll: true }); }
            catch { presetModalFocusReturn.focus(); }
          }
          presetModalFocusReturn = null;
        }
      }

      // SPEC-2017 US-9 — Kanban Drawer (slide-over detail). Reuses the
      // SPEC-2356 .op-drawer pattern; backdrop click and Esc both
      // dismiss it; createFocusTrap keeps Tab within the dialog while
      // open. State is module-scoped because only one Drawer is open
      // at a time even when multiple Kanban windows exist.
      let kanbanDrawerFocusReturn = null;
      let kanbanDrawerFocusTrapRelease = null;
      let kanbanDrawerActiveContext = null;
      let workspaceOverviewFocusReturn = null;
      let workspaceOverviewFocusTrapRelease = null;
      function openKanbanDrawer(context) {
        const drawer = document.getElementById("kanban-drawer");
        const backdrop = document.getElementById("kanban-drawer-backdrop");
        if (!drawer || !backdrop) return;
        kanbanDrawerActiveContext = context || null;
        kanbanDrawerFocusReturn = document.activeElement;
        backdrop.hidden = false;
        backdrop.dataset.open = "true";
        drawer.hidden = false;
        drawer.dataset.open = "true";
        renderKanbanDrawerBody();
        try { drawer.focus({ preventScroll: true }); }
        catch { drawer.focus(); }
        if (typeof kanbanDrawerFocusTrapRelease === "function") {
          kanbanDrawerFocusTrapRelease();
        }
        kanbanDrawerFocusTrapRelease = createFocusTrap(drawer, { document });
      }

      function closeKanbanDrawer() {
        const drawer = document.getElementById("kanban-drawer");
        const backdrop = document.getElementById("kanban-drawer-backdrop");
        if (!drawer || !backdrop) return;
        if (drawer.dataset.open !== "true") return;
        drawer.dataset.open = "false";
        backdrop.dataset.open = "false";
        // Hide after the transition so prefers-reduced-motion users
        // still see the focus trap dismantle cleanly.
        backdrop.hidden = true;
        drawer.hidden = true;
        if (typeof kanbanDrawerFocusTrapRelease === "function") {
          kanbanDrawerFocusTrapRelease();
          kanbanDrawerFocusTrapRelease = null;
        }
        if (
          kanbanDrawerFocusReturn &&
          typeof kanbanDrawerFocusReturn.focus === "function"
        ) {
          try { kanbanDrawerFocusReturn.focus({ preventScroll: true }); }
          catch { kanbanDrawerFocusReturn.focus(); }
        }
        kanbanDrawerFocusReturn = null;
        kanbanDrawerActiveContext = null;
      }

      function openWorkspaceOverview() {
        const drawer = document.getElementById("workspace-overview-drawer");
        const backdrop = document.getElementById("workspace-overview-drawer-backdrop");
        if (!drawer || !backdrop) return;
        workspaceOverviewFocusReturn = document.activeElement;
        backdrop.hidden = false;
        backdrop.dataset.open = "true";
        drawer.hidden = false;
        drawer.dataset.open = "true";
        renderWorkspaceOverview();
        try { drawer.focus({ preventScroll: true }); }
        catch { drawer.focus(); }
        if (typeof workspaceOverviewFocusTrapRelease === "function") {
          workspaceOverviewFocusTrapRelease();
        }
        workspaceOverviewFocusTrapRelease = createFocusTrap(drawer, { document });
      }

      function closeWorkspaceOverview() {
        const drawer = document.getElementById("workspace-overview-drawer");
        const backdrop = document.getElementById("workspace-overview-drawer-backdrop");
        if (!drawer || !backdrop) return;
        if (drawer.dataset.open !== "true") return;
        drawer.dataset.open = "false";
        backdrop.dataset.open = "false";
        backdrop.hidden = true;
        drawer.hidden = true;
        if (typeof workspaceOverviewFocusTrapRelease === "function") {
          workspaceOverviewFocusTrapRelease();
          workspaceOverviewFocusTrapRelease = null;
        }
        if (
          workspaceOverviewFocusReturn &&
          typeof workspaceOverviewFocusReturn.focus === "function"
        ) {
          try { workspaceOverviewFocusReturn.focus({ preventScroll: true }); }
          catch { workspaceOverviewFocusReturn.focus(); }
        }
        workspaceOverviewFocusReturn = null;
      }

      function workspaceCleanupEntry(candidate) {
        return {
          name: candidate.branch,
          cleanup_ready: true,
          cleanup: {
            availability: "safe",
            upstream: candidate.remote_delete_available
              ? `origin/${candidate.branch}`
              : null,
            merge_target: { kind: "workspace", reference: "Workspace complete" },
            execution_branch: candidate.branch,
            risks: [],
          },
        };
      }

      function openWorkspaceCleanup() {
        const candidate = activeWorkProjection?.cleanup_candidate;
        if (!candidate?.branch) return;
        const state = ensureBranchListState(WORKSPACE_CLEANUP_WINDOW_ID);
        state.entries = [workspaceCleanupEntry(candidate)];
        state.cleanupSelected = new Set([candidate.branch]);
        state.notice = "";
        state.cleanupModal = {
          open: true,
          stage: "confirm",
          // Workspace cleanup is local-only by default even when
          // cleanup_candidate.default_delete_remote is present on the wire.
          deleteRemote: false,
          results: [],
        };
        branchCleanupWindowId = WORKSPACE_CLEANUP_WINDOW_ID;
        renderBranchCleanupModal();
      }

      function renderWorkspaceOverview() {
        const titleEl = document.getElementById("workspace-overview-title");
        const body = document.getElementById("workspace-overview-body");
        const footer = document.getElementById("workspace-overview-footer");
        if (!body || !titleEl || !footer) return;

        const projection = activeWorkProjection || {};
        const title = projection.title || `${activeWorkspace().title || "Project"} workspace`;
        titleEl.textContent = title;
        body.innerHTML = "";
        footer.innerHTML = "";

        const summaryCard = createNode("section", "workspace-overview-card");
        summaryCard.appendChild(createNode("div", "workspace-overview-title", title));
        const meta = createNode("div", "workspace-overview-meta");
        appendMeta(meta, projection.status_category ? agentStatusLabel(projection.status_category) : "");
        appendMeta(meta, projection.owner);
        appendMeta(meta, projection.pr_number ? `PR #${projection.pr_number}` : "");
        const agentsTotal =
          Number(projection.active_agents || 0) + Number(projection.blocked_agents || 0);
        appendMeta(meta, agentsTotal ? `${agentsTotal} agent${agentsTotal === 1 ? "" : "s"}` : "");
        summaryCard.appendChild(meta);
        summaryCard.appendChild(
          createNode(
            "div",
            "workspace-overview-summary",
            projection.summary || projection.status_text || "No Workspace summary yet",
          ),
        );
        if (projection.next_action) {
          summaryCard.appendChild(
            createNode("div", "workspace-overview-next", projection.next_action),
          );
        }
        body.appendChild(summaryCard);

        const cleanupCandidate = projection.cleanup_candidate;
        if (cleanupCandidate?.branch) {
          const cleanupSection = createNode("section", "workspace-overview-section");
          cleanupSection.appendChild(
            createNode("div", "workspace-overview-heading", "Workspace Cleanup"),
          );
          const cleanupCopy = createNode(
            "div",
            "workspace-overview-summary",
            `Local workspace ${cleanupCandidate.branch} is ready for cleanup.`,
          );
          cleanupSection.appendChild(cleanupCopy);
          const cleanupActions = createNode("div", "op-work-actions");
          const cleanupButton = createNode("button", "op-work-action", "Review Cleanup");
          cleanupButton.type = "button";
          cleanupButton.addEventListener("click", openWorkspaceCleanup);
          cleanupActions.appendChild(cleanupButton);
          cleanupSection.appendChild(cleanupActions);
          body.appendChild(cleanupSection);
        }

        const agents = Array.isArray(projection.agents) ? projection.agents : [];
        const agentSection = createNode("section", "workspace-overview-section");
        agentSection.appendChild(createNode("div", "workspace-overview-heading", "Current Agents"));
        const agentList = createNode("div", "workspace-overview-list");
        if (agents.length === 0) {
          agentList.appendChild(createNode("div", "workspace-overview-empty", "No live Agents"));
        } else {
          for (const agent of agents) {
            const item = createNode("article", "workspace-journal-entry");
            item.appendChild(
              createNode(
                "div",
                "workspace-journal-summary",
                agent.current_focus || agent.display_name || agent.agent_id || "Agent",
              ),
            );
            const itemMeta = createNode("div", "workspace-journal-meta");
            appendMeta(itemMeta, agent.display_name || agent.agent_id);
            appendMeta(itemMeta, agentStatusLabel(agent.status_category));
            appendMeta(itemMeta, agent.coordination_scope);
            item.appendChild(itemMeta);
            agentList.appendChild(item);
          }
        }
        agentSection.appendChild(agentList);
        body.appendChild(agentSection);

        const journalEntries = Array.isArray(projection.journal_entries)
          ? projection.journal_entries
          : [];
        const journalSection = createNode("section", "workspace-overview-section");
        journalSection.appendChild(createNode("div", "workspace-overview-heading", "Recent Summary"));
        const journalList = createNode("div", "workspace-overview-list");
        if (journalEntries.length === 0) {
          journalList.appendChild(
            createNode("div", "workspace-overview-empty", "No Workspace journal entries"),
          );
        } else {
          for (const entry of journalEntries) {
            const item = createNode("article", "workspace-journal-entry");
            item.appendChild(
              createNode(
                "div",
                "workspace-journal-summary",
                entry.summary ||
                  entry.status_text ||
                  entry.next_action ||
                  entry.title ||
                  "Workspace update",
              ),
            );
            const itemMeta = createNode("div", "workspace-journal-meta");
            appendMeta(itemMeta, entry.updated_at);
            appendMeta(itemMeta, entry.owner);
            appendMeta(itemMeta, entry.next_action ? "Next action" : "");
            item.appendChild(itemMeta);
            journalList.appendChild(item);
          }
        }
        journalSection.appendChild(journalList);
        body.appendChild(journalSection);

        const boardRefs = Array.isArray(projection.board_refs) ? projection.board_refs : [];
        const latestBoardRef = boardRefs.length > 0 ? boardRefs[boardRefs.length - 1] : "";
        if (latestBoardRef) {
          const openBoard = createNode("button", "op-work-action", "Open Latest Board Entry");
          openBoard.type = "button";
          openBoard.addEventListener("click", () => focusBoardEntry(latestBoardRef));
          footer.appendChild(openBoard);
        }
        const startWork = createNode("button", "op-work-action", "Start Work");
        startWork.type = "button";
        startWork.addEventListener("click", () => {
          closeWorkspaceOverview();
          send({ kind: "open_start_work" });
        });
        footer.appendChild(startWork);
      }

      function renderKanbanDrawerBody() {
        const body = document.getElementById("kanban-drawer-body");
        const titleEl = document.getElementById("kanban-drawer-title");
        const footer = document.getElementById("kanban-drawer-footer");
        if (!body || !titleEl || !footer) return;
        const context = kanbanDrawerActiveContext;
        if (!context) {
          body.innerHTML = "";
          footer.innerHTML = "";
          titleEl.textContent = "Detail";
          return;
        }
        const state = ensureKnowledgeBridgeState(context.windowId, context.kind);
        const detail = state.detail;
        body.innerHTML = "";
        footer.innerHTML = "";
        titleEl.textContent = detail?.title || "Loading detail";
        if (state.detailLoading || !detail) {
          body.appendChild(
            createNode(
              "div",
              "kanban-drawer-section-body",
              state.detailLoading ? "Loading detail" : "No cached detail available",
            ),
          );
          return;
        }
        if (detail.subtitle) {
          body.appendChild(
            createNode("div", "knowledge-detail-subtitle", detail.subtitle),
          );
        }
        if ((detail.labels || []).length > 0) {
          const labelRow = createNode("div", "knowledge-label-row");
          for (const label of detail.labels) {
            labelRow.appendChild(createNode("span", "kanban-card-chip", label));
          }
          body.appendChild(labelRow);
        }
        for (const section of detail.sections || []) {
          const card = createNode("section", "kanban-drawer-section");
          card.appendChild(
            createNode("div", "kanban-drawer-section-title", section.title),
          );
          card.appendChild(
            createNode("pre", "kanban-drawer-section-body", section.body),
          );
          body.appendChild(card);
        }
        if (
          detail.launch_issue_number !== null &&
          detail.launch_issue_number !== undefined
        ) {
          const launchButton = createNode(
            "button",
            "wizard-button primary",
            "Launch Agent",
          );
          launchButton.type = "button";
          launchButton.addEventListener("click", () => {
            openIssueLaunchWizard(context.windowId, detail.launch_issue_number);
          });
          footer.appendChild(launchButton);
        }
      }

      function clamp(value, min) {
        return Math.max(min, value);
      }

      function clampRange(value, min, max) {
        return Math.min(Math.max(value, min), max);
      }

      function parseNumber(value) {
        return Number.parseFloat(value || "0");
      }

      function applyWorldGridViewport() {
        if (!worldGrid) {
          return;
        }
        const gridSize = 32 * viewport.zoom;
        const majorGridSize = gridSize * 4;
        const gridPosition = `${viewport.x}px ${viewport.y}px`;
        worldGrid.style.backgroundSize = [
          `${gridSize}px ${gridSize}px`,
          `${gridSize}px ${gridSize}px`,
          `${majorGridSize}px ${majorGridSize}px`,
          `${majorGridSize}px ${majorGridSize}px`,
        ].join(", ");
        worldGrid.style.backgroundPosition = [
          gridPosition,
          gridPosition,
          gridPosition,
          gridPosition,
        ].join(", ");
      }

      function applyViewport() {
        stage.style.transform = `translate(${viewport.x}px, ${viewport.y}px) scale(${viewport.zoom})`;
        applyWorldGridViewport();
        stage.style.willChange = "transform";
        if (viewportRasterTimer !== null) {
          clearTimeout(viewportRasterTimer);
        }
        viewportRasterTimer = setTimeout(() => {
          stage.style.willChange = "auto";
          viewportRasterTimer = null;
        }, 300);
      }

      function persistViewport() {
        send({
          kind: "update_viewport",
          viewport,
        });
      }

      function canvasCenterAnchor() {
        const rect = canvas.getBoundingClientRect();
        return {
          x: rect.width / 2,
          y: rect.height / 2,
        };
      }

      function zoomCanvasAt(anchorX, anchorY, nextZoom) {
        const clampedZoom = clampRange(nextZoom, 0.6, 2.4);
        const worldX = (anchorX - viewport.x) / viewport.zoom;
        const worldY = (anchorY - viewport.y) / viewport.zoom;
        viewport.x = anchorX - worldX * clampedZoom;
        viewport.y = anchorY - worldY * clampedZoom;
        viewport.zoom = clampedZoom;
        applyViewport();
        persistViewport();
      }

      function zoomCanvasByFactor(factor) {
        const anchor = canvasCenterAnchor();
        zoomCanvasAt(anchor.x, anchor.y, viewport.zoom * factor);
      }

      function resetCanvasZoom() {
        const anchor = canvasCenterAnchor();
        zoomCanvasAt(anchor.x, anchor.y, 1);
      }

      function visibleBounds() {
        return {
          x: -viewport.x / viewport.zoom,
          y: -viewport.y / viewport.zoom,
          width: canvas.clientWidth / viewport.zoom,
          height: canvas.clientHeight / viewport.zoom,
        };
      }

      function topmostWindowId(workspace) {
        const visibleWindows = (workspace.windows || []).filter(visibleWindowData);
        if (visibleWindows.length === 0) {
          return null;
        }
        return visibleWindows.reduce((topmost, candidate) => {
          if (!topmost || candidate.z_index > topmost.z_index) {
            return candidate;
          }
          return topmost;
        }, null)?.id;
      }

      function cycleFocus(direction) {
        if (windowMap.size === 0) {
          return;
        }
        send({
          kind: "cycle_focus",
          direction,
          bounds: visibleBounds(),
        });
      }

      function shouldHandleFocusShortcut(event) {
        if (event.repeat) {
          return false;
        }
        if (!(event.metaKey || event.ctrlKey) || !event.shiftKey || event.altKey) {
          return false;
        }
        if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") {
          return false;
        }
        if (modal.classList.contains("open") || wizardModal.classList.contains("open")) {
          return false;
        }
        return true;
      }

      function arrangeWindows(mode) {
        send({
          kind: "arrange_windows",
          mode,
          bounds: visibleBounds(),
        });
      }

      function canRefreshTerminalViewport(windowId) {
        return !workspaceWindowById(windowId)?.minimized;
      }

      function fitTerminal(windowId, persist = false) {
        const runtime = terminalMap.get(windowId);
        const element = windowMap.get(windowId);
        if (!runtime || !element) {
          return;
        }
        if (!canRefreshTerminalViewport(windowId)) {
          if (persist) {
            sendGeometry(windowId, runtime.terminal.cols, runtime.terminal.rows);
          }
          return;
        }
        runtime.fitAddon.fit();
        if (!persist) {
          return;
        }
        sendGeometry(windowId, runtime.terminal.cols, runtime.terminal.rows);
      }

      function scheduleTerminalResizeFit(windowId) {
        if (!terminalMap.has(windowId)) {
          return;
        }
        if (!resizeState || resizeState.id !== windowId || resizeState.fitFrame !== null) {
          return;
        }
        resizeState.fitFrame = requestAnimationFrame(() => {
          if (!resizeState || resizeState.id !== windowId) {
            return;
          }
          resizeState.fitFrame = null;
          fitTerminal(windowId, false);
        });
      }

      function cancelTerminalResizeFit() {
        if (!resizeState || resizeState.fitFrame === null) {
          return;
        }
        cancelAnimationFrame(resizeState.fitFrame);
        resizeState.fitFrame = null;
      }

      function finishWindowResize(pointerId) {
        if (!resizeState || resizeState.pointerId !== pointerId) {
          return;
        }
        const runtime = terminalMap.get(resizeState.id);
        cancelTerminalResizeFit();
        fitTerminal(resizeState.id, false);
        sendGeometry(
          resizeState.id,
          runtime?.terminal.cols || 80,
          runtime?.terminal.rows || 24,
        );
        runtime?.terminal.focus();
        resizeState = null;
        // SPEC-2356 Phase 9 (T-136): release the hover-reveal peek strip lock
        // so pointer events resume on the screen-edge triggers once resize
        // ends.
        delete document.documentElement.dataset.opResizeActive;
      }

      function scheduleTerminalViewportRefresh(windowId) {
        const runtime = terminalMap.get(windowId);
        if (
          !runtime ||
          runtime.viewportRefreshFrame !== null ||
          !canRefreshTerminalViewport(windowId)
        ) {
          return;
        }
        runtime.viewportRefreshFrame = requestAnimationFrame(() => {
          runtime.viewportRefreshFrame = null;
          if (!canRefreshTerminalViewport(windowId)) {
            return;
          }
          refreshTerminalViewport(windowId);
        });
      }

      function refreshTerminalViewport(windowId) {
        const runtime = terminalMap.get(windowId);
        if (!runtime || !canRefreshTerminalViewport(windowId)) {
          return;
        }
        runtime.terminal.refresh(0, runtime.terminal.rows - 1);
      }

      function sendGeometry(windowId, cols, rows) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const windowData = workspaceWindowById(windowId);
        send({
          kind: "update_window_geometry",
          id: windowId,
          geometry: {
            x: parseNumber(element.style.left),
            y: parseNumber(element.style.top),
            width: windowData?.minimized
              ? windowData.geometry.width
              : parseNumber(element.style.width),
            height: windowData?.minimized
              ? windowData.geometry.height
              : parseNumber(element.style.height),
          },
          cols,
          rows,
        });
      }

      // SPEC-2356 — Living Telemetry counters in the Operator Status Strip.
      // Aggregates `data-agent-state` across all open windows and pushes the
      // counts into the bottom strip. We also expose agent count to the
      // sidebar layer for the "Quick" section's hint.
      function recomputeOperatorTelemetry() {
        if (!window.__operatorShell?.applyTelemetryCounts) return;
        const counts = { active: 0, idle: 0, blocked: 0, done: 0, agents: 0 };
        for (const el of windowMap.values()) {
          const state = el?.dataset?.agentState;
          if (!state) continue;
          if (state in counts) counts[state] += 1;
          counts.agents += 1;
        }
        if (activeWorkProjection) {
          const category = activeWorkProjection.status_category || "unknown";
          const activeAgents = Number(activeWorkProjection.active_agents || 0);
          const blockedAgents = Number(activeWorkProjection.blocked_agents || 0);
          if (category === "active") counts.active = Math.max(counts.active, activeAgents || 1);
          if (category === "idle") counts.idle = Math.max(counts.idle, 1);
          if (category === "blocked") counts.blocked = Math.max(counts.blocked, blockedAgents || 1);
          if (category === "done") counts.done = Math.max(counts.done, 1);
          counts.blocked = Math.max(counts.blocked, blockedAgents);
          counts.agents = Math.max(counts.agents, activeAgents + blockedAgents);
        }
        try {
          window.__operatorShell.applyTelemetryCounts(counts);
        } catch (e) {
          console.warn("operator telemetry update failed", e);
        }
      }

      function activeWorkFocusableAgents(projection) {
        const agents = Array.isArray(projection?.agents) ? projection.agents : [];
        return agents.filter((agent) => {
          if (!agent?.window_id) return false;
          const windowData = workspaceWindowById(agent.window_id);
          if (!windowData || !presetSupportsWaitingStatus(windowData.preset)) return false;
          const status = String(windowData.status || "running").toLowerCase();
          return status !== "stopped" && status !== "exited" && status !== "error";
        });
      }

      function agentStatusLabel(state) {
        switch (String(state || "").toLowerCase()) {
          case "active":
            return "Running";
          case "blocked":
            return "Blocked";
          case "idle":
            return "Idle";
          case "done":
            return "Done";
          default:
            return "Unknown";
        }
      }

      function focusActiveWorkAgentWindow(agent) {
        if (!agent?.window_id) return;
        const windowData = workspaceWindowById(agent.window_id);
        if (!windowData) return;
        if (windowData.minimized) {
          send({ kind: "restore_window", id: agent.window_id });
        }
        focusWindowRemotely(agent.window_id, { center: true });
      }

      function setActiveWorkSectionVisible(visible) {
        if (activeWorkSection) {
          activeWorkSection.hidden = !visible;
        }
      }

      function projectionIssueNumber(projection) {
        const owner = String(projection?.owner || "");
        const match = owner.match(/^Issue\s+#(\d+)$/i);
        return match ? Number(match[1]) : null;
      }

      function compactPathLabel(value) {
        if (!value) return "";
        const parts = String(value).split(/[\\/]+/).filter(Boolean);
        if (parts.length <= 2) return String(value);
        return `${parts[parts.length - 2]}/${parts[parts.length - 1]}`;
      }

      function appendMeta(container, value) {
        if (!value) return;
        container.appendChild(createNode("span", "", value));
      }

      function coordinationKindLabel(kind) {
        switch (String(kind || "").toLowerCase()) {
          case "blocked":
            return "Blocked";
          case "handoff":
            return "Handoff";
          case "next":
            return "Next";
          case "claim":
            return "Claim";
          case "decision":
            return "Decision";
          case "status":
            return "Status";
          default:
            return "";
        }
      }

      function activeBoardWindowIds() {
        return (activeWorkspace().windows || [])
          .filter((windowData) => windowData.preset === "board" && !windowData.minimized)
          .map((windowData) => windowData.id);
      }

      function focusBoardEntry(entryId) {
        if (!entryId) {
          focusOrSpawnPreset("board");
          return;
        }
        pendingBoardEntryFocusId = entryId;
        for (const windowId of activeBoardWindowIds()) {
          const state = ensureBoardState(windowId);
          state.focusEntryId = entryId;
          state.pendingFocusScroll = true;
          state.audienceFilter = "all";
          if (
            !state.entries.some((entry) => entry.id === entryId) &&
            state.hasMoreBefore &&
            !state.loadingOlder
          ) {
            requestOlderBoardEntries(windowId);
          }
          renderBoard(windowId);
        }
        focusOrSpawnPreset("board");
      }

      function renderActiveWorkOverview() {
        if (!activeWorkSummary || !activeWorkAgents) return;
        activeWorkSummary.innerHTML = "";
        activeWorkAgents.innerHTML = "";

        if (!activeWorkProjection) {
          if (activeWorkCount) activeWorkCount.textContent = "0";
          setActiveWorkSectionVisible(false);
          return;
        }

        const agents = activeWorkFocusableAgents(activeWorkProjection);
        const agentCount = agents.length;
        if (activeWorkCount) activeWorkCount.textContent = String(agentCount);

        if (agentCount === 0) {
          setActiveWorkSectionVisible(false);
          return;
        }

        setActiveWorkSectionVisible(true);

        activeWorkSummary.appendChild(
          createNode("div", "op-work-title", activeWorkProjection.title || "Active Work"),
        );
        const meta = createNode("div", "op-work-meta");
        appendMeta(meta, activeWorkProjection.owner);
        appendMeta(meta, activeWorkProjection.pr_number ? `PR #${activeWorkProjection.pr_number}` : "");
        activeWorkSummary.appendChild(meta);
        activeWorkSummary.appendChild(
          createNode(
            "div",
            "op-work-status",
            activeWorkProjection.next_action ||
              activeWorkProjection.status_text ||
              "Work is active",
          ),
        );

        const actions = createNode("div", "op-work-actions");
        if (activeWorkProjection.branch) {
          const addAgent = createNode("button", "op-work-action", "Add Agent to This Work");
          addAgent.type = "button";
          addAgent.addEventListener("click", () => {
            send({
              kind: "open_active_work_launch_wizard",
              branch_name: activeWorkProjection.branch,
              linked_issue_number: projectionIssueNumber(activeWorkProjection),
            });
          });
          actions.appendChild(addAgent);
        }
        const boardRefs = activeWorkProjection.board_refs || [];
        const latestBoardRef = boardRefs.length > 0 ? boardRefs[boardRefs.length - 1] : "";
        if (latestBoardRef) {
          const openBoard = createNode("button", "op-work-action", "Open Latest Board Entry");
          openBoard.type = "button";
          openBoard.addEventListener("click", () => focusBoardEntry(latestBoardRef));
          actions.appendChild(openBoard);
        }
        if (actions.childNodes.length > 0) {
          activeWorkSummary.appendChild(actions);
        }

        for (const agent of agents) {
          const state = String(agent.status_category || "unknown").toLowerCase();
          const coordinationKind = String(agent.last_board_entry_kind || "").toLowerCase();
          const coordinationLabel = coordinationKindLabel(coordinationKind);
          const card = createNode("article", "op-agent-card");
          card.dataset.state = state;
          if (coordinationKind) card.dataset.kind = coordinationKind;
          if (agent.last_board_entry_id) card.dataset.boardRef = agent.last_board_entry_id;

          const head = createNode("div", "op-agent-head");
          head.appendChild(
            createNode("div", "op-agent-name", agent.display_name || agent.agent_id || "Agent"),
          );
          const chips = createNode("div", "op-agent-chips");
          if (coordinationLabel) {
            chips.appendChild(createNode("div", "op-agent-kind", coordinationLabel));
          }
          chips.appendChild(createNode("div", "op-agent-state", agentStatusLabel(state)));
          head.appendChild(chips);
          card.appendChild(head);

          const agentMeta = createNode("div", "op-agent-meta");
          appendMeta(agentMeta, agent.last_board_entry_id ? "Board linked" : "");
          card.appendChild(agentMeta);

          if (agent.coordination_scope) {
            card.appendChild(createNode("div", "op-agent-scope", agent.coordination_scope));
          }

          const agentFocusText = agent.title_summary || agent.current_focus;
          if (agentFocusText) {
            const focusText = coordinationLabel
              ? `${coordinationLabel}: ${agentFocusText}`
              : agentFocusText;
            const agentFocus = createNode("div", "op-agent-focus", focusText);
            if (agent.current_focus && agent.current_focus !== agentFocusText) {
              agentFocus.title = agent.current_focus;
            }
            card.appendChild(agentFocus);
          }

          const agentActions = createNode("div", "op-agent-actions");
          const focusButton = createNode("button", "op-agent-action", "Focus");
          focusButton.type = "button";
          focusButton.addEventListener("click", () => {
            focusActiveWorkAgentWindow(agent);
          });
          agentActions.appendChild(focusButton);
          if (agent.last_board_entry_id) {
            const boardButton = createNode("button", "op-agent-action", "Open Entry");
            boardButton.type = "button";
            boardButton.addEventListener("click", () => focusBoardEntry(agent.last_board_entry_id));
            agentActions.appendChild(boardButton);
          }
          if (agentActions.childNodes.length > 0) {
            card.appendChild(agentActions);
          }

          activeWorkAgents.appendChild(card);
        }
      }

      // SPEC-2356 — translate legacy runtime state vocabulary to Living
      // Telemetry semantic states (`active|idle|blocked|done`). The mapping is
      // intentionally narrow so future runtime states surface as
      // `idle` until the design language explicitly handles them.
      function mapAgentTelemetryState(runtimeState) {
        switch (runtimeState) {
          case "starting":
          case "running":
          case "waiting":
            return "active";
          case "ready":
            return "idle";
          case "stopped":
          case "exited":
            return "done";
          case "error":
            return "blocked";
          default:
            return "idle";
        }
      }

      function applyStatus(windowId, status, detail) {
        const windowData = workspaceWindowById(windowId);
        const runtimeState = normalizeWindowRuntimeState(status, windowData?.preset);
        windowRuntimeStateMap.set(windowId, runtimeState);
        if (detail) {
          detailMap.set(windowId, detail);
        } else if (runtimeState === "running" || runtimeState === "waiting") {
          detailMap.delete(windowId);
        }
        const element = windowMap.get(windowId);
        if (!element) {
          renderWindowList();
          return;
        }
        const chip = element.querySelector(".status-chip");
        const label = element.querySelector(".status-label");
        const overlay = element.querySelector(".terminal-overlay");
        const runtimeChip = chip;
        runtimeChip.hidden = !shouldShowRuntimeStatus(windowData);
        chip.classList.remove(
          "starting",
          "running",
          "ready",
          "waiting",
          "stopped",
          "exited",
          "error",
        );
        chip.classList.add(runtimeState);
        // SPEC-2356 — Living Telemetry: project the runtime state onto a stable
        // `data-agent-state` attribute the components.css layer animates.
        element.dataset.agentState = mapAgentTelemetryState(runtimeState);
        recomputeOperatorTelemetry();
        label.textContent = windowRuntimeLabel(runtimeState);
        const effectiveDetail = detailMap.get(windowId);
        if (overlay) {
          const messageEl = overlay.querySelector(".overlay-message");
          if (messageEl) {
            messageEl.textContent = effectiveDetail || "";
          } else {
            overlay.textContent = effectiveDetail || "";
          }
          updateTerminalOverlayCopyState(overlay);
          overlay.classList.toggle(
            "visible",
            runtimeState === "error" ||
              runtimeState === "stopped" ||
              (runtimeState === "running" && Boolean(effectiveDetail)),
          );
          if (runtimeState === "running" && Boolean(effectiveDetail)) {
            startSpinnerAnimation(overlay);
          } else {
            stopSpinnerAnimation(overlay);
          }
        }
        renderWindowList();
      }

      function stopSpinnerAnimation(overlay) {
        if (overlay.__spinnerIntervalId) {
          clearInterval(overlay.__spinnerIntervalId);
          overlay.__spinnerIntervalId = null;
        }
        if (overlay.__spinnerObserver) {
          overlay.__spinnerObserver.disconnect();
          overlay.__spinnerObserver = null;
        }
      }

      function startSpinnerAnimation(overlay) {
        const spinner = overlay.querySelector(".overlay-spinner");
        if (!spinner) return;
        if (overlay.__spinnerIntervalId) return;
        const chars = ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];
        let index = 0;
        spinner.textContent = chars[0];
        overlay.__spinnerIntervalId = setInterval(() => {
          spinner.textContent = chars[index % chars.length];
          index++;
        }, 100);
        const observer = new MutationObserver(() => {
          if (!overlay.classList.contains("visible")) {
            stopSpinnerAnimation(overlay);
          }
        });
        observer.observe(overlay, { attributes: true });
        overlay.__spinnerObserver = observer;
      }

      function focusWindowLocally(windowId) {
        focusedId = windowId;
        for (const [id, element] of windowMap.entries()) {
          element.classList.toggle("focused", id === windowId);
        }
      }

      function focusWindowRemotely(windowId, { center = false } = {}) {
        focusWindowLocally(windowId);
        const payload = { kind: "focus_window", id: windowId };
        if (center) payload.bounds = visibleBounds();
        send(payload);
      }

      function toggleMinimizeWindow(windowId) {
        const windowData = workspaceWindowById(windowId);
        if (!windowData) {
          return;
        }
        focusWindowRemotely(windowId);
        send({
          kind: windowData.minimized ? "restore_window" : "minimize_window",
          id: windowId,
        });
      }

      function toggleMaximizeWindow(windowId) {
        const windowData = workspaceWindowById(windowId);
        if (!windowData) {
          return;
        }
        focusWindowRemotely(windowId);
        if (windowData.maximized) {
          send({ kind: "restore_window", id: windowId });
          return;
        }
        if (windowData.minimized) {
          send({ kind: "restore_window", id: windowId });
        }
        send({
          kind: "maximize_window",
          id: windowId,
          bounds: visibleBounds(),
        });
      }

      function renderWindowTabs(windowData, element) {
        const strip = element.querySelector(".window-tab-strip");
        if (!strip) return;
        const tabs = windowTabsFor(windowData);
        strip.innerHTML = "";
        for (const tab of tabs) {
          const tabItem = document.createElement("div");
          tabItem.className = "window-tab-item";
          const tabButton = document.createElement("button");
          tabButton.type = "button";
          tabButton.className = "window-tab";
          tabButton.draggable = true;
          tabButton.dataset.windowTabId = tab.id;
          tabButton.setAttribute("aria-label", `Activate ${tab.title}`);
          if (tab.id === windowData.id || tab.tab_group_active) {
            tabButton.classList.add("active");
            tabButton.setAttribute("aria-current", "page");
          }
          tabButton.textContent = tab.title;
          tabButton.addEventListener("click", (event) => {
            event.stopPropagation();
            send({ kind: "activate_window_tab", id: tab.id });
          });
          tabButton.addEventListener("dragstart", (event) => {
            windowTabDragState = { id: tab.id, docked: false };
            event.dataTransfer?.setData("text/plain", tab.id);
            if (event.dataTransfer) {
              event.dataTransfer.effectAllowed = "move";
            }
          });
          tabButton.addEventListener("dragend", (event) => {
            const drag = windowTabDragState;
            windowTabDragState = null;
            if (!drag || drag.docked) return;
            const draggedWindow = workspaceWindowById(drag.id);
            if (!draggedWindow?.tab_group_id) return;
            send({
              kind: "detach_window_tab",
              id: drag.id,
              geometry: detachGeometryFromPointer(event, draggedWindow),
            });
          });
          const closeButton = document.createElement("button");
          closeButton.type = "button";
          closeButton.className = "window-tab-close";
          closeButton.setAttribute("aria-label", `Close ${tab.title}`);
          closeButton.textContent = "×";
          closeButton.addEventListener("click", (event) => {
            event.stopPropagation();
            send({ kind: "close_window", id: tab.id });
          });
          tabItem.appendChild(tabButton);
          tabItem.appendChild(closeButton);
          strip.appendChild(tabItem);
        }
      }

      function handleTitlebarClick(windowId) {
        const now = Date.now();
        if (
          titlebarClickState &&
          titlebarClickState.id === windowId &&
          now - titlebarClickState.timestamp <= 300
        ) {
          titlebarClickState = null;
          const windowData = workspaceWindowById(windowId);
          if (windowData?.minimized) {
            focusWindowRemotely(windowId);
            send({ kind: "restore_window", id: windowId });
            return;
          }
          toggleMaximizeWindow(windowId);
          return;
        }
        titlebarClickState = {
          id: windowId,
          timestamp: now,
        };
      }

      function decodeBase64(base64) {
        return Uint8Array.from(atob(base64), (value) => value.charCodeAt(0));
      }

      function isMacPlatform() {
        const platform = navigator.userAgentData?.platform || navigator.platform || "";
        return /mac|iphone|ipad|ipod/i.test(platform);
      }

      function isTerminalCopyShortcut(event) {
        if (isMacPlatform()) {
          return false;
        }
        const key = typeof event.key === "string" ? event.key.toLowerCase() : "";
        return (
          event.ctrlKey &&
          event.shiftKey &&
          !event.altKey &&
          !event.metaKey &&
          key === "c"
        );
      }

      const SUPPORTED_IMAGE_PASTE_MIME_TYPES = new Set([
        "image/png",
        "image/jpeg",
        "image/webp",
      ]);

      function findClipboardImagePasteItem(items) {
        for (const item of Array.from(items || [])) {
          if (
            item?.kind === "file" &&
            SUPPORTED_IMAGE_PASTE_MIME_TYPES.has(item.type)
          ) {
            return item;
          }
        }
        return null;
      }

      function dataUrlBase64Payload(dataUrl) {
        if (typeof dataUrl !== "string") {
          return null;
        }
        const commaIndex = dataUrl.indexOf(",");
        if (commaIndex < 0) {
          return null;
        }
        const payload = dataUrl.slice(commaIndex + 1);
        return payload || null;
      }

      function readClipboardImageAsBase64(file) {
        return new Promise((resolve, reject) => {
          const reader = new FileReader();
          reader.addEventListener("load", () => {
            resolve(dataUrlBase64Payload(reader.result));
          });
          reader.addEventListener("error", () => {
            reject(reader.error || new Error("Failed to read clipboard image"));
          });
          reader.readAsDataURL(file);
        });
      }

      async function readNavigatorClipboardItems() {
        if (!navigator.clipboard?.read) {
          return [];
        }
        return navigator.clipboard.read();
      }

      async function readNavigatorClipboardText() {
        if (!navigator.clipboard?.readText) {
          return "";
        }
        return navigator.clipboard.readText();
      }

      async function writeClipboardText(text, restoreFocus = null) {
        if (!text) {
          return false;
        }
        if (navigator.clipboard?.writeText) {
          try {
            await navigator.clipboard.writeText(text);
            restoreFocus?.();
            return true;
          } catch (_error) {
            // Fall back to a temporary textarea when the async clipboard API is unavailable.
          }
        }

        const textarea = document.createElement("textarea");
        textarea.value = text;
        textarea.setAttribute("readonly", "");
        textarea.style.position = "fixed";
        textarea.style.top = "-1000px";
        textarea.style.left = "-1000px";
        textarea.style.opacity = "0";
        document.body.appendChild(textarea);
        textarea.focus();
        textarea.select();

        try {
          return document.execCommand("copy");
        } catch (_error) {
          return false;
        } finally {
          textarea.remove();
          restoreFocus?.();
        }
      }

      async function copyTerminalSelection(windowId) {
        const runtime = terminalMap.get(windowId);
        if (!runtime || !runtime.terminal.hasSelection()) {
          return false;
        }
        const selection = runtime.terminal.getSelection();
        if (!selection) {
          return false;
        }
        return writeClipboardText(selection, () => runtime.terminal.focus());
      }

      async function copyTerminalOverlayMessage(windowId) {
        const element = windowMap.get(windowId);
        const messageEl = element?.querySelector(".terminal-overlay .overlay-message");
        if (!messageEl) {
          return false;
        }
        return writeClipboardText(messageEl.textContent, () => {
          terminalMap.get(windowId)?.terminal.focus();
        });
      }

      function updateTerminalOverlayCopyState(overlay) {
        const button = overlay?.querySelector(".overlay-copy-button");
        const messageEl = overlay?.querySelector(".overlay-message");
        if (!button || !messageEl) {
          return;
        }
        const hasMessage = Boolean(messageEl.textContent);
        button.hidden = !hasMessage;
        button.disabled = !hasMessage;
      }

      function installTerminalCopyHandlers(windowId, terminalRoot, terminal) {
        const copyState = {
          mouseDown: false,
          dragged: false,
          startX: 0,
          startY: 0,
        };

        function resetCopyState() {
          copyState.mouseDown = false;
          copyState.dragged = false;
          copyState.startX = 0;
          copyState.startY = 0;
        }

        const handleMouseDown = (event) => {
          if (event.button !== 0) {
            return;
          }
          copyState.mouseDown = true;
          copyState.dragged = false;
          copyState.startX = event.clientX;
          copyState.startY = event.clientY;
        };

        const handleMouseMove = (event) => {
          if (!copyState.mouseDown || copyState.dragged) {
            return;
          }
          const movedX = Math.abs(event.clientX - copyState.startX);
          const movedY = Math.abs(event.clientY - copyState.startY);
          if (
            movedX >= TERMINAL_SELECTION_DRAG_THRESHOLD ||
            movedY >= TERMINAL_SELECTION_DRAG_THRESHOLD
          ) {
            copyState.dragged = true;
          }
        };

        const handleMouseUp = (event) => {
          if (!copyState.mouseDown) {
            return;
          }
          const shouldCopy = event.button === 0 && copyState.dragged;
          resetCopyState();
          if (!shouldCopy) {
            return;
          }
          requestAnimationFrame(() => {
            if (!terminal.hasSelection()) {
              return;
            }
            void copyTerminalSelection(windowId);
          });
        };

        const handleWindowBlur = () => {
          resetCopyState();
        };

        terminal.attachCustomKeyEventHandler((event) => {
          if (!isTerminalCopyShortcut(event)) {
            return true;
          }
          event.preventDefault();
          event.stopPropagation();
          if (!terminal.hasSelection()) {
            return false;
          }
          void copyTerminalSelection(windowId);
          return false;
        });

        terminalRoot.addEventListener("mousedown", handleMouseDown);
        window.addEventListener("mousemove", handleMouseMove, true);
        window.addEventListener("mouseup", handleMouseUp, true);
        window.addEventListener("blur", handleWindowBlur);

        return () => {
          terminal.attachCustomKeyEventHandler(() => true);
          terminalRoot.removeEventListener("mousedown", handleMouseDown);
          window.removeEventListener("mousemove", handleMouseMove, true);
          window.removeEventListener("mouseup", handleMouseUp, true);
          window.removeEventListener("blur", handleWindowBlur);
        };
      }

      function installTerminalImagePasteHandlers(windowId, terminalRoot, terminal) {
        const handlePaste = (event) => {
          const item = findClipboardImagePasteItem(event.clipboardData?.items);
          if (!item) {
            return;
          }
          const file = item.getAsFile?.();
          if (!file) {
            return;
          }
          event.preventDefault();
          event.stopPropagation();

          void readClipboardImageAsBase64(file)
            .then((dataBase64) => {
              if (!dataBase64) {
                return;
              }
              send({
                kind: "paste_image",
                id: windowId,
                data_base64: dataBase64,
                mime_type: file.type || item.type,
                filename: file.name || null,
              });
              terminal.focus();
            })
            .catch(() => {
              terminal.focus();
            });
        };

        terminalRoot.addEventListener("paste", handlePaste, true);
        return () => {
          terminalRoot.removeEventListener("paste", handlePaste, true);
        };
      }

      function installTerminalContextMenuHandlers(windowId, terminalRoot, terminal) {
        const controller = createTerminalContextMenuController({
          document,
          window,
          terminalRoot,
          readClipboardText: readNavigatorClipboardText,
          readClipboardItems: readNavigatorClipboardItems,
          blobToBase64: readClipboardImageAsBase64,
          supportedImageTypes: SUPPORTED_IMAGE_PASTE_MIME_TYPES,
          pasteText: (text) => terminal.paste(text),
          pasteImage: ({ dataBase64, mimeType, filename }) => {
            send({
              kind: "paste_image",
              id: windowId,
              data_base64: dataBase64,
              mime_type: mimeType,
              filename,
            });
          },
          focusTerminal: () => terminal.focus(),
        });
        return () => {
          controller.dispose();
        };
      }

      function installTerminalViewportRefreshHandlers(windowId, terminal) {
        const viewportScrollDisposable = terminal.onScroll(() => {
          scheduleTerminalViewportRefresh(windowId);
        });

        return () => {
          viewportScrollDisposable.dispose();
        };
      }

      // SPEC-2356 — xterm content stays on the Dark Operator palette even when
      // surrounding window chrome follows the app's light theme. High-contrast
      // forced-colors mode is still delegated to system colors by CSS.
      const XTERM_THEME_DARK = {
        background: "#0a0d12",
        foreground: "#e8eaed",
        cursor: "#f8fafc",
        selectionBackground: "#1e293b",
        black: "#0f172a",
        red: "#ef4444",
        green: "#22c55e",
        yellow: "#f59e0b",
        blue: "#3b82f6",
        magenta: "#a855f7",
        cyan: "#06b6d4",
        white: "#cbd5e1",
        brightBlack: "#334155",
        brightRed: "#f87171",
        brightGreen: "#4ade80",
        brightYellow: "#fbbf24",
        brightBlue: "#60a5fa",
        brightMagenta: "#c084fc",
        brightCyan: "#22d3ee",
        brightWhite: "#f8fafc",
      };

      function createTerminalRuntime(windowId, terminalContainer) {
        if (terminalMap.has(windowId)) {
          return terminalMap.get(windowId);
        }
        const terminal = new Terminal({
          cursorBlink: true,
          convertEol: true,
          theme: XTERM_THEME_DARK,
          fontFamily:
            "var(--font-mono), ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
          fontSize: 14,
          lineHeight: 1.28,
          scrollback: 5000,
        });
        const fitAddon = new FitAddon();
        terminal.loadAddon(fitAddon);
        terminal.open(terminalContainer);
        const copyCleanup = installTerminalCopyHandlers(windowId, terminalContainer, terminal);
        const imagePasteCleanup = installTerminalImagePasteHandlers(
          windowId,
          terminalContainer,
          terminal,
        );
        const contextMenuCleanup = installTerminalContextMenuHandlers(
          windowId,
          terminalContainer,
          terminal,
        );
        const viewportRefreshCleanup = installTerminalViewportRefreshHandlers(windowId, terminal);
        const cleanup = () => {
          copyCleanup();
          imagePasteCleanup();
          contextMenuCleanup();
          viewportRefreshCleanup();
        };
        terminal.onData((data) => {
          inputTraceSeq += 1;
          const wsState = socket ? socket.readyState : -1;
          console.debug("[gwt_input_trace:onData]", {
            seq: inputTraceSeq,
            windowId,
            dataLen: data.length,
            wsState,
          });
          send({ kind: "terminal_input", id: windowId, data });
        });
        const runtime = { terminal, fitAddon, cleanup, viewportRefreshFrame: null };
        terminalMap.set(windowId, runtime);
        decoderMap.set(windowId, new TextDecoder());
        requestAnimationFrame(() => fitTerminal(windowId, true));

        const snapshot = pendingSnapshotMap.get(windowId);
        if (snapshot) {
          replaceTerminalSnapshot(windowId, snapshot);
          pendingSnapshotMap.delete(windowId);
        }

        const pending = pendingOutputMap.get(windowId);
        if (pending?.length) {
          for (const chunk of pending) {
            writeOutput(windowId, chunk);
          }
          pendingOutputMap.delete(windowId);
        }
        return runtime;
      }

      function writeOutput(windowId, base64) {
        const runtime = terminalMap.get(windowId);
        if (!runtime) {
          const queue = pendingOutputMap.get(windowId) || [];
          queue.push(base64);
          pendingOutputMap.set(windowId, queue);
          return;
        }
        const decoder = decoderMap.get(windowId);
        runtime.terminal.write(decoder.decode(decodeBase64(base64), { stream: true }), () => {
          scheduleTerminalViewportRefresh(windowId);
        });
      }

      function replaceTerminalSnapshot(windowId, base64) {
        const runtime = terminalMap.get(windowId);
        if (!runtime) {
          pendingSnapshotMap.set(windowId, base64);
          return;
        }
        const decoder = decoderMap.get(windowId);
        runtime.terminal.reset();
        runtime.terminal.write(decoder.decode(decodeBase64(base64)), () => {
          scheduleTerminalViewportRefresh(windowId);
        });
      }

      function mockContentForPreset(preset) {
        switch (preset) {
          case "settings":
            return {
              heading: "Environment",
              rows: [
                ["Theme", "Canvas"],
                ["Agents", "Claude / Codex"],
                ["Transport", "Local server"],
              ],
            };
          case "profile":
            return {
              heading: "Active Profile",
              rows: [
                ["Profile", "default"],
                ["Model", "Mixed"],
                ["Shell", "Interactive"],
              ],
            };
          case "logs":
            return {
              heading: "Recent Events",
              rows: [
                ["Workspace", "synced"],
                ["Transport", "connected"],
                ["Canvas", "ready"],
              ],
            };
          case "issue":
            return {
              heading: "Issue Bridge",
              rows: [
                ["Cache", "repo-scoped"],
                ["Refresh", "gwt-managed"],
                ["Launch", "issue context"],
              ],
            };
          case "spec":
            return {
              heading: "SPEC Bridge",
              rows: [
                ["Sections", "spec/plan/tasks"],
                ["Refresh", "cache pull"],
                ["Repair", "cache recovery"],
              ],
            };
          case "board":
            return {
              heading: "Coordination",
              rows: [
                ["Backlog", "4 cards"],
                ["Doing", "2 cards"],
                ["Blocked", "0 cards"],
              ],
            };
          case "pr":
            return {
              heading: "PR Bridge",
              rows: [
                ["Cache", "repo-scoped"],
                ["Checks", "cache-backed"],
                ["Workflow", "gwt-managed"],
              ],
            };
          default:
            return {
              heading: "Workspace",
              rows: [
                ["Canvas", "ready"],
                ["Windows", "floating"],
                ["Transport", "shared"],
              ],
            };
        }
      }

      function ensureFileTreeState(windowId) {
        if (!fileTreeStateMap.has(windowId)) {
          fileTreeStateMap.set(windowId, {
            loaded: new Map(),
            expanded: new Set(),
            loading: new Set(),
            selectedPath: "",
            error: "",
          });
        }
        return fileTreeStateMap.get(windowId);
      }

      function ensureBranchListState(windowId) {
        if (!branchListStateMap.has(windowId)) {
          branchListStateMap.set(windowId, {
            entries: [],
            phase: "hydrated",
            receivedFreshEntries: false,
            loading: false,
            error: "",
            selectedBranchName: "",
            filter: "local",
            cleanupSelected: new Set(),
            notice: "",
            cleanupModal: {
              open: false,
              stage: "confirm",
              deleteRemote: false,
              results: [],
            },
          });
        }
        return branchListStateMap.get(windowId);
      }

      function ensureProfileState(windowId) {
        if (!profileStateMap.has(windowId)) {
          profileStateMap.set(windowId, {
            snapshot: null,
            loading: false,
            saving: false,
            error: "",
            selectedProfile: null,
            draft: null,
            saveTimer: null,
          });
        }
        return profileStateMap.get(windowId);
      }

      function ensureKnowledgeBridgeState(windowId, knowledgeKind) {
        if (!knowledgeBridgeStateMap.has(windowId)) {
          knowledgeBridgeStateMap.set(windowId, {
            kind: knowledgeKind,
            listScope: "open",
            entries: [],
            baseEntries: [],
            selectedNumber: null,
            detail: null,
            query: "",
            loading: false,
            refreshing: false,
            searching: false,
            detailLoading: false,
            pendingSearchTimer: null,
            loadRequestId: 0,
            detailRequestId: 0,
            searchRequestId: 0,
            inFlightSearchRequestId: 0,
            searchInFlight: false,
            queuedSearchQuery: "",
            error: "",
            emptyMessage: "",
            baseEmptyMessage: "",
            refreshEnabled: true,
            // SPEC-2017 — Kanban state. hideDone hydrates from
            // localStorage so the user's preference survives reloads;
            // dndSnapshot stores the pre-drop column index to enable
            // optimistic-UI rollback when phase write-back fails;
            // pendingPhaseUpdates tracks in-flight requests so cards
            // render a spinner until the server confirms the move.
            hideDone: readKanbanHideDonePreference(),
            dndSnapshot: null,
            pendingPhaseUpdates: new Map(),
          });
        }
        const state = knowledgeBridgeStateMap.get(windowId);
        state.kind = knowledgeKind || state.kind;
        if (!state.listScope) {
          state.listScope = "open";
        }
        if (state.hideDone === undefined) {
          state.hideDone = readKanbanHideDonePreference();
        }
        if (!state.pendingPhaseUpdates) {
          state.pendingPhaseUpdates = new Map();
        }
        return state;
      }

      function readKanbanHideDonePreference() {
        try {
          if (typeof localStorage === "undefined") return false;
          return localStorage.getItem("kanban-hide-done") === "1";
        } catch (_err) {
          return false;
        }
      }

      function writeKanbanHideDonePreference(value) {
        try {
          if (typeof localStorage === "undefined") return;
          if (value) {
            localStorage.setItem("kanban-hide-done", "1");
          } else {
            localStorage.removeItem("kanban-hide-done");
          }
        } catch (_err) {
          // localStorage may be unavailable in private mode; ignore.
        }
      }

      function clearKnowledgeBridgeState(windowId) {
        const state = knowledgeBridgeStateMap.get(windowId);
        if (state?.pendingSearchTimer) {
          clearTimeout(state.pendingSearchTimer);
          state.pendingSearchTimer = null;
        }
        if (state) {
          state.queuedSearchQuery = "";
          state.searchInFlight = false;
          state.inFlightSearchRequestId = 0;
          state.detailRequestId = 0;
          state.pendingPhaseUpdates?.clear();
          state.dndSnapshot = null;
        }
        knowledgeBridgeStateMap.delete(windowId);
      }

      function ensureLogState(windowId) {
        if (!logStateMap.has(windowId)) {
          logStateMap.set(windowId, {
            entries: [],
            loading: false,
            error: "",
            severity: "debug",
            query: "",
            selectedEntryId: null,
            unreadAlerts: 0,
            unreadEntryId: null,
          });
        }
        return logStateMap.get(windowId);
      }

      function ensureBoardState(windowId) {
        if (!boardStateMap.has(windowId)) {
          boardStateMap.set(windowId, {
            entries: [],
            loading: false,
            submitting: false,
            error: "",
            replyParentId: null,
            composerKind: "status",
            composerBody: "",
            pendingSubmit: null,
            hasMoreBefore: false,
            oldestEntryId: null,
            loadingOlder: false,
            pendingSelfPostScroll: false,
            preserveBoardScrollPosition: false,
            shouldFollowBoardBottom: true,
            newEntriesAvailable: false,
            focusEntryId: null,
            pendingFocusScroll: false,
            audienceFilter: "all",
            forYouUnread: 0,
            lastNotifiedMentionEntryId: null,
          });
        }
        return boardStateMap.get(windowId);
      }

      function normalizeLogSeverity(severity) {
        switch (String(severity || "").toLowerCase()) {
          case "error":
          case "warn":
          case "info":
          case "debug":
            return String(severity).toLowerCase();
          default:
            return "info";
        }
      }

      function logSeverityRank(severity) {
        switch (normalizeLogSeverity(severity)) {
          case "error":
            return 3;
          case "warn":
            return 2;
          case "info":
            return 1;
          default:
            return 0;
        }
      }

      function requestFileTree(windowId, path = "") {
        const state = ensureFileTreeState(windowId);
        if (state.loading.has(path)) {
          return;
        }
        state.loading.add(path);
        send({
          kind: "load_file_tree",
          id: windowId,
          path: path || null,
        });
      }

      function requestBranches(windowId) {
        const state = ensureBranchListState(windowId);
        if (state.loading) {
          return;
        }
        state.loading = true;
        state.receivedFreshEntries = false;
        send({
          kind: "load_branches",
          id: windowId,
        });
      }

      function requestBoard(windowId) {
        const state = ensureBoardState(windowId);
        if (state.loading) {
          return;
        }
        state.loading = true;
        state.error = "";
        send({
          kind: "load_board",
          id: windowId,
        });
      }

      function requestOlderBoardEntries(windowId) {
        const state = ensureBoardState(windowId);
        if (state.loading || state.loadingOlder || !state.hasMoreBefore) {
          return;
        }
        const beforeEntryId = state.oldestEntryId || state.entries[0]?.id || null;
        if (!beforeEntryId) {
          return;
        }
        state.loadingOlder = true;
        state.error = "";
        send({
          kind: "load_board_history",
          id: windowId,
          before_entry_id: beforeEntryId,
          limit: 50,
        });
      }

      function requestProfile(windowId) {
        const state = ensureProfileState(windowId);
        if (state.loading) {
          return;
        }
        state.loading = true;
        state.error = "";
        send({
          kind: "load_profile",
          id: windowId,
        });
      }

      function requestLogs(windowId) {
        const state = ensureLogState(windowId);
        if (state.loading) {
          return;
        }
        state.loading = true;
        state.error = "";
        send({
          kind: "load_logs",
          id: windowId,
        });
      }

      function requestKnowledgeBridge(windowId, knowledgeKind, refresh = false) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        if (state.loading) {
          return;
        }
        if (state.pendingSearchTimer) {
          clearTimeout(state.pendingSearchTimer);
          state.pendingSearchTimer = null;
        }
        const requestId = nextKnowledgeLoadRequestId++;
        state.loadRequestId = requestId;
        state.detailRequestId = 0;
        state.loading = true;
        state.refreshing = Boolean(refresh);
        state.searching = false;
        state.searchInFlight = false;
        state.inFlightSearchRequestId = 0;
        state.queuedSearchQuery = "";
        state.searchRequestId += 1;
        state.error = "";
        const effectiveKind = knowledgeKind || state.kind;
        send({
          kind: "load_knowledge_bridge",
          id: windowId,
          knowledge_kind: effectiveKind,
          request_id: requestId,
          selected_number: state.selectedNumber ?? null,
          refresh,
          list_scope:
            effectiveKind === "issue" ? state.listScope || "open" : null,
        });
      }

      function restoreKnowledgeBaseEntries(state) {
        state.entries = Array.isArray(state.baseEntries)
          ? state.baseEntries.slice()
          : [];
        state.emptyMessage = state.baseEmptyMessage || "";
        if (
          state.selectedNumber &&
          !state.entries.some((entry) => entry.number === state.selectedNumber)
        ) {
          state.selectedNumber =
            state.entries.length > 0 ? state.entries[0].number : null;
        }
      }

      function knowledgeEventScopeMatches(state, event) {
        return !(
          state.kind === "issue" &&
          event.list_scope &&
          event.list_scope !== state.listScope
        );
      }

      function knowledgeDetailRequestMatches(state, event) {
        return (
          !event.request_id ||
          event.request_id === state.loadRequestId ||
          event.request_id === state.detailRequestId
        );
      }

      function sendKnowledgeSemanticSearch(windowId, knowledgeKind, query) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        const effectiveKind = knowledgeKind || state.kind;
        const requestId = nextKnowledgeSearchRequestId++;
        state.searchRequestId = requestId;
        state.inFlightSearchRequestId = requestId;
        state.searchInFlight = true;
        state.searching = true;
        send({
          kind: "search_knowledge_bridge",
          id: windowId,
          knowledge_kind: effectiveKind,
          query,
          request_id: requestId,
          selected_number: state.selectedNumber ?? null,
          list_scope:
            effectiveKind === "issue" ? state.listScope || "open" : null,
        });
      }

      function scheduleKnowledgeSearch(windowId, knowledgeKind) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        if (state.pendingSearchTimer) {
          clearTimeout(state.pendingSearchTimer);
          state.pendingSearchTimer = null;
        }
        const query = state.query.trim();
        state.error = "";
        if (!query) {
          state.searching = false;
          state.searchInFlight = false;
          state.inFlightSearchRequestId = 0;
          state.queuedSearchQuery = "";
          state.searchRequestId += 1;
          restoreKnowledgeBaseEntries(state);
          renderKnowledgeBridge(windowId);
          return;
        }
        if (state.loading && state.baseEntries.length === 0) {
          state.searching = true;
          renderKnowledgeBridge(windowId);
          return;
        }
        if (state.searchInFlight) {
          state.queuedSearchQuery = query;
          state.searching = true;
          renderKnowledgeBridge(windowId);
          return;
        }
        state.searching = true;
        state.pendingSearchTimer = setTimeout(() => {
          state.pendingSearchTimer = null;
          if (!workspaceWindowById(windowId)) {
            return;
          }
          const latestQuery = state.query.trim();
          if (!latestQuery) {
            state.searching = false;
            restoreKnowledgeBaseEntries(state);
            renderKnowledgeBridge(windowId);
            return;
          }
          if (state.searchInFlight) {
            state.queuedSearchQuery = latestQuery;
            renderKnowledgeBridge(windowId);
            return;
          }
          sendKnowledgeSemanticSearch(windowId, knowledgeKind, latestQuery);
        }, 250);
        renderKnowledgeBridge(windowId);
      }

      function requestKnowledgeDetail(windowId, knowledgeKind, number) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        state.selectedNumber = number;
        state.detailLoading = true;
        const requestId = nextKnowledgeLoadRequestId++;
        state.detailRequestId = requestId;
        const effectiveKind = knowledgeKind || state.kind;
        send({
          kind: "select_knowledge_bridge_entry",
          id: windowId,
          knowledge_kind: effectiveKind,
          request_id: requestId,
          number,
          list_scope:
            effectiveKind === "issue" ? state.listScope || "open" : null,
        });
      }

      // SPEC-2017 US-8 — push a Kanban phase change to the backend.
      // The optimistic UI move lives in renderKanbanCard's drop handler;
      // this helper just wires the WebSocket request and reserves a
      // request_id so knowledge_bridge_phase_updated can correlate the
      // response back to a specific drop. target_phase=null means
      // "Backlog" — the backend strips every phase/* label.
      function sendUpdateKnowledgePhase(windowId, issueNumber, targetPhase) {
        const requestId = nextKnowledgeLoadRequestId++;
        send({
          kind: "update_knowledge_bridge_phase",
          id: windowId,
          request_id: requestId,
          issue_number: issueNumber,
          target_phase: targetPhase,
        });
        return requestId;
      }

      function openIssueLaunchWizard(windowId, issueNumber) {
        send({
          kind: "open_issue_launch_wizard",
          id: windowId,
          issue_number: issueNumber,
        });
      }

      function sendWizardAction(action) {
        send({
          kind: "launch_wizard_action",
          action,
          bounds: visibleBounds(),
        });
      }

      function createNode(tagName, className, textContent) {
        const node = document.createElement(tagName);
        if (className) {
          node.className = className;
        }
        if (textContent !== undefined) {
          node.textContent = textContent;
        }
        return node;
      }

      function boardTimestampLabel(value) {
        if (!value) {
          return "";
        }
        const date = new Date(value);
        if (Number.isNaN(date.getTime())) {
          return value;
        }
        return date.toLocaleString("en-US", {
          month: "short",
          day: "numeric",
          hour: "2-digit",
          minute: "2-digit",
        });
      }

      function logMatchesQuery(entry, query) {
        if (!query) {
          return true;
        }
        const haystacks = [
          entry.message,
          entry.source,
          entry.detail,
          JSON.stringify(entry.fields || {}),
        ];
        return haystacks.some((value) =>
          String(value || "").toLowerCase().includes(query),
        );
      }

      function filteredLogEntries(state) {
        const minimumRank = logSeverityRank(state.severity);
        const query = String(state.query || "").trim().toLowerCase();
        return (state.entries || [])
          .filter(
            (entry) =>
              logSeverityRank(entry.severity) >= minimumRank &&
              logMatchesQuery(entry, query),
          )
          .slice()
          .reverse();
      }

      function appendLiveLogEntry(entry) {
        for (const [windowId, state] of logStateMap.entries()) {
          state.entries.push(entry);
          if (state.entries.length > 1000) {
            state.entries = state.entries.slice(-1000);
          }
          if (logSeverityRank(entry.severity) >= logSeverityRank("warn")) {
            state.unreadAlerts += 1;
            state.unreadEntryId = entry.id;
          }
          renderLogs(windowId);
        }
      }

      function jumpToUnreadLogs(windowId) {
        const state = ensureLogState(windowId);
        const unreadEntry =
          (state.unreadEntryId &&
            state.entries.find((entry) => entry.id === state.unreadEntryId)) ||
          [...state.entries]
            .reverse()
            .find((entry) => logSeverityRank(entry.severity) >= logSeverityRank("warn"));
        if (unreadEntry) {
          state.selectedEntryId = unreadEntry.id;
        }
        state.unreadAlerts = 0;
        state.unreadEntryId = null;
        renderLogs(windowId);
      }

      function renderLogs(windowId) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const body = element.querySelector(".window-body");
        if (!body) {
          return;
        }
        const state = ensureLogState(windowId);
        const status = body.querySelector(".logs-status");
        const unreadButton = body.querySelector(".logs-unread-button");
        const severitySelect = body.querySelector(".logs-severity-select");
        const searchInput = body.querySelector(".logs-search-input");
        const timeline = body.querySelector(".logs-timeline");
        const detailPane = body.querySelector(".logs-detail-pane");
        if (
          !status ||
          !unreadButton ||
          !severitySelect ||
          !searchInput ||
          !timeline ||
          !detailPane
        ) {
          return;
        }

        const filteredEntries = filteredLogEntries(state);
        const selectedEntry =
          state.entries.find((entry) => entry.id === state.selectedEntryId) ||
          filteredEntries[0] ||
          null;
        if (selectedEntry) {
          state.selectedEntryId = selectedEntry.id;
        } else {
          state.selectedEntryId = null;
        }

        status.textContent = state.error
          ? state.error
          : state.loading
            ? "Loading logs..."
            : `${filteredEntries.length} visible / ${state.entries.length} total`;
        status.className = "logs-status";
        if (state.error) {
          status.classList.add("error");
        } else if (state.loading) {
          status.classList.add("info");
        }

        unreadButton.hidden = state.unreadAlerts === 0;
        unreadButton.textContent =
          state.unreadAlerts === 1
            ? "1 unread alert"
            : `${state.unreadAlerts} unread alerts`;
        severitySelect.value = state.severity;
        searchInput.value = state.query;

        timeline.innerHTML = "";
        if (!state.loading && filteredEntries.length === 0) {
          timeline.appendChild(createNode("div", "logs-empty workspace-empty-state", "No log entries match."));
        }
        for (const entry of filteredEntries) {
          const row = createNode("button", "logs-entry");
          row.type = "button";
          if (selectedEntry && selectedEntry.id === entry.id) {
            row.classList.add("selected");
            row.setAttribute("aria-current", "true");
          } else {
            row.removeAttribute("aria-current");
          }
          row.addEventListener("click", () => {
            state.selectedEntryId = entry.id;
            if (logSeverityRank(entry.severity) >= logSeverityRank("warn")) {
              state.unreadAlerts = 0;
              state.unreadEntryId = null;
            }
            renderLogs(windowId);
          });

          const header = createNode("div", "logs-entry-header");
          header.appendChild(
            createNode(
              "span",
              `logs-severity-chip ${normalizeLogSeverity(entry.severity)}`,
              String(entry.severity || "info").toUpperCase(),
            ),
          );
          header.appendChild(
            createNode("span", "logs-entry-source", entry.source || "gwt"),
          );
          header.appendChild(
            createNode(
              "span",
              "logs-entry-time",
              boardTimestampLabel(entry.timestamp),
            ),
          );
          row.appendChild(header);
          row.appendChild(
            createNode("div", "logs-entry-message", entry.message || "(empty log event)"),
          );
          if (entry.detail) {
            row.appendChild(createNode("div", "logs-entry-detail", entry.detail));
          }
          timeline.appendChild(row);
        }

        detailPane.innerHTML = "";
        if (!selectedEntry) {
          detailPane.appendChild(
            createNode("div", "logs-empty workspace-empty-state", "Select a log entry to inspect details."),
          );
          return;
        }

        detailPane.appendChild(createNode("div", "mock-label", "Log detail"));
        detailPane.appendChild(
          createNode(
            "div",
            "logs-detail-message",
            selectedEntry.message || "(empty log event)",
          ),
        );
        detailPane.appendChild(
          createNode(
            "div",
            "logs-detail-meta",
            `${String(selectedEntry.severity || "info").toUpperCase()} · ${selectedEntry.source || "gwt"} · ${boardTimestampLabel(selectedEntry.timestamp)}`,
          ),
        );
        if (selectedEntry.detail) {
          detailPane.appendChild(
            createNode("pre", "logs-detail-block", selectedEntry.detail),
          );
        }
        const fields = selectedEntry.fields || {};
        if (Object.keys(fields).length > 0) {
          detailPane.appendChild(
            createNode(
              "pre",
              "logs-detail-block",
              JSON.stringify(fields, null, 2),
            ),
          );
        }
      }

      function clearProfileSaveTimer(state) {
        if (state.saveTimer !== null) {
          clearTimeout(state.saveTimer);
          state.saveTimer = null;
        }
      }

      function profileDraftFromEntry(profile) {
        if (!profile) {
          return null;
        }
        return {
          currentName: profile.name,
          name: profile.name,
          description: profile.description || "",
          envVars: (profile.env_vars || []).map((entry) => ({
            key: entry.key || "",
            value: entry.value || "",
          })),
          disabledEnv: (profile.disabled_env || []).map((entry) => entry || ""),
        };
      }

      function selectedProfileEntry(state) {
        const profiles = state.snapshot?.profiles || [];
        if (!state.selectedProfile) {
          return null;
        }
        return profiles.find((profile) => profile.name === state.selectedProfile) || null;
      }

      function syncProfileDraftFromSelection(state) {
        const selected = selectedProfileEntry(state);
        state.draft = profileDraftFromEntry(selected);
      }

      function profileDraftSignature(draft) {
        if (!draft) {
          return "";
        }
        return JSON.stringify({
          currentName: draft.currentName,
          name: draft.name,
          description: draft.description,
          envVars: draft.envVars.map((entry) => ({
            key: entry.key,
            value: entry.value,
          })),
          disabledEnv: draft.disabledEnv.slice(),
        });
      }

      function profileDraftIsDirty(state) {
        const selected = selectedProfileEntry(state);
        return profileDraftSignature(state.draft) !== profileDraftSignature(profileDraftFromEntry(selected));
      }

      function updateProfileStatus(windowId) {
        const element = windowMap.get(windowId);
        const status = element?.querySelector(".profile-status");
        if (!status) {
          return;
        }
        const state = ensureProfileState(windowId);
        const profileCount = state.snapshot?.profiles?.length || 0;
        const activeProfile = state.snapshot?.active_profile || "default";
        status.textContent = state.error
          ? state.error
          : state.loading
            ? state.saving
              ? "Saving profile..."
              : "Loading profiles..."
            : state.saving
              ? "Saving profile..."
              : `Active ${activeProfile} · ${profileCount} profile${profileCount === 1 ? "" : "s"}`;
        status.className = "profile-status";
        if (state.error) {
          status.classList.add("error");
        } else if (state.loading || state.saving) {
          status.classList.add("info");
        }
      }

      function flushProfileSave(windowId) {
        const state = ensureProfileState(windowId);
        clearProfileSaveTimer(state);
        if (!state.draft) {
          state.saving = false;
          updateProfileStatus(windowId);
          return;
        }
        if (!profileDraftIsDirty(state)) {
          state.saving = false;
          updateProfileStatus(windowId);
          return;
        }
        state.loading = true;
        state.saving = true;
        state.error = "";
        updateProfileStatus(windowId);
        send({
          kind: "save_profile",
          id: windowId,
          current_name: state.draft.currentName,
          name: state.draft.name,
          description: state.draft.description,
          env_vars: state.draft.envVars.filter((entry) => entry.key.trim() || entry.value),
          disabled_env: state.draft.disabledEnv.filter((entry) => entry.trim()),
        });
      }

      function scheduleProfileSave(windowId) {
        const state = ensureProfileState(windowId);
        clearProfileSaveTimer(state);
        state.saving = true;
        updateProfileStatus(windowId);
        state.saveTimer = setTimeout(() => {
          state.saveTimer = null;
          flushProfileSave(windowId);
        }, 250);
      }

      function selectProfile(windowId, profileName) {
        const state = ensureProfileState(windowId);
        if (state.selectedProfile === profileName) {
          return;
        }
        if (profileDraftIsDirty(state)) {
          flushProfileSave(windowId);
        } else {
          clearProfileSaveTimer(state);
        }
        state.loading = true;
        state.error = "";
        updateProfileStatus(windowId);
        send({
          kind: "select_profile",
          id: windowId,
          profile_name: profileName,
        });
      }

      function createProfile(windowId) {
        const state = ensureProfileState(windowId);
        if (profileDraftIsDirty(state)) {
          flushProfileSave(windowId);
        } else {
          clearProfileSaveTimer(state);
        }
        const name = window.prompt("Profile name", "review");
        if (!name) {
          return;
        }
        state.loading = true;
        state.error = "";
        updateProfileStatus(windowId);
        send({
          kind: "create_profile",
          id: windowId,
          name,
        });
      }

      function setActiveProfile(windowId) {
        const state = ensureProfileState(windowId);
        if (!state.selectedProfile) {
          return;
        }
        state.loading = true;
        state.error = "";
        updateProfileStatus(windowId);
        send({
          kind: "set_active_profile",
          id: windowId,
          profile_name: state.selectedProfile,
        });
      }

      function deleteProfile(windowId) {
        const state = ensureProfileState(windowId);
        if (!state.selectedProfile) {
          return;
        }
        if (!window.confirm(`Delete profile "${state.selectedProfile}"?`)) {
          return;
        }
        clearProfileSaveTimer(state);
        state.loading = true;
        state.saving = false;
        state.error = "";
        updateProfileStatus(windowId);
        send({
          kind: "delete_profile",
          id: windowId,
          profile_name: state.selectedProfile,
        });
      }

      function renderProfile(windowId, preserveDraft = false) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const body = element.querySelector(".window-body");
        if (!body) {
          return;
        }
        const state = ensureProfileState(windowId);
        const snapshot = state.snapshot || {
          active_profile: "default",
          selected_profile: "default",
          profiles: [],
          merged_preview: [],
        };
        const profiles = snapshot.profiles || [];
        const status = body.querySelector(".profile-status");
        const list = body.querySelector(".profile-list");
        const editor = body.querySelector(".profile-editor-pane");
        if (!status || !list || !editor) {
          return;
        }

        if (
          state.selectedProfile &&
          !profiles.some((profile) => profile.name === state.selectedProfile)
        ) {
          state.selectedProfile = null;
        }
        if (!state.selectedProfile) {
          state.selectedProfile =
            snapshot.selected_profile || snapshot.active_profile || (profiles[0] ? profiles[0].name : null);
          preserveDraft = false;
        }

        if (!preserveDraft || !state.draft || state.draft.currentName !== state.selectedProfile) {
          syncProfileDraftFromSelection(state);
        }

        updateProfileStatus(windowId);
        list.innerHTML = "";
        if (!state.loading && profiles.length === 0) {
          const empty = createNode("div", "profile-empty workspace-empty-state");
          empty.appendChild(createNode("div", "mock-label", "No profiles yet"));
          empty.appendChild(
            createNode(
              "div",
              "profile-empty-copy",
              "Create a profile to track env overrides, disabled OS variables, and merged preview output.",
            ),
          );
          const button = createNode("button", "wizard-button primary", "New profile");
          button.type = "button";
          button.addEventListener("click", () => createProfile(windowId));
          empty.appendChild(button);
          list.appendChild(empty);
        }

        for (const profile of profiles) {
          const row = createNode("button", "profile-row");
          row.type = "button";
          if (profile.name === state.selectedProfile) {
            row.classList.add("selected");
            row.setAttribute("aria-current", "true");
          } else {
            row.removeAttribute("aria-current");
          }
          row.addEventListener("click", () => selectProfile(windowId, profile.name));
          const header = createNode("div", "profile-row-header");
          header.appendChild(createNode("div", "profile-row-title", profile.name));
          const chips = createNode("div", "profile-row-chips");
          if (profile.is_active) {
            chips.appendChild(createNode("span", "profile-chip active", "Active"));
          }
          if (profile.is_default) {
            chips.appendChild(createNode("span", "profile-chip", "Default"));
          }
          header.appendChild(chips);
          row.appendChild(header);
          row.appendChild(
            createNode(
              "div",
              "profile-row-copy",
              profile.description || "No description yet",
            ),
          );
          const meta = createNode(
            "div",
            "profile-row-meta",
            `${profile.env_vars.length} env override${profile.env_vars.length === 1 ? "" : "s"} · ${profile.disabled_env.length} disabled variable${profile.disabled_env.length === 1 ? "" : "s"}`,
          );
          row.appendChild(meta);
          list.appendChild(row);
        }

        editor.innerHTML = "";
        const selected = selectedProfileEntry(state);
        if (!selected || !state.draft) {
          const empty = createNode("div", "profile-empty workspace-empty-state");
          empty.appendChild(createNode("div", "mock-label", "Select a profile"));
          empty.appendChild(
            createNode(
              "div",
              "profile-empty-copy",
              "Each profile keeps its own env overrides and merged preview output.",
            ),
          );
          editor.appendChild(empty);
          updateProfileStatus(windowId);
          return;
        }

        const actions = createNode("div", "profile-inline-actions");
        const activeButton = createNode("button", "wizard-button", "Set active");
        activeButton.type = "button";
        activeButton.disabled = selected.is_active || state.loading;
        activeButton.addEventListener("click", () => setActiveProfile(windowId));
        actions.appendChild(activeButton);

        const saveButton = createNode("button", "wizard-button", "Save now");
        saveButton.type = "button";
        saveButton.disabled = !profileDraftIsDirty(state) || state.loading;
        saveButton.addEventListener("click", () => flushProfileSave(windowId));
        actions.appendChild(saveButton);

        const deleteButton = createNode("button", "wizard-button", "Delete");
        deleteButton.type = "button";
        deleteButton.disabled = selected.is_default || state.loading;
        deleteButton.addEventListener("click", () => deleteProfile(windowId));
        actions.appendChild(deleteButton);
        editor.appendChild(actions);

        const metadata = createNode("div", "profile-section");
        metadata.appendChild(createNode("div", "mock-label", "Profile metadata"));
        const nameField = createNode("label", "profile-field");
        nameField.appendChild(createNode("span", "", "Name"));
        const nameInput = document.createElement("input");
        nameInput.type = "text";
        nameInput.value = state.draft.name;
        nameInput.addEventListener("input", () => {
          state.draft.name = nameInput.value;
          scheduleProfileSave(windowId);
        });
        nameInput.addEventListener("blur", () => flushProfileSave(windowId));
        nameField.appendChild(nameInput);
        metadata.appendChild(nameField);

        const descriptionField = createNode("label", "profile-field");
        descriptionField.appendChild(createNode("span", "", "Description"));
        const descriptionInput = document.createElement("textarea");
        descriptionInput.className = "profile-textarea";
        descriptionInput.value = state.draft.description;
        descriptionInput.addEventListener("input", () => {
          state.draft.description = descriptionInput.value;
          scheduleProfileSave(windowId);
        });
        descriptionInput.addEventListener("blur", () => flushProfileSave(windowId));
        descriptionField.appendChild(descriptionInput);
        metadata.appendChild(descriptionField);
        editor.appendChild(metadata);

        const envSection = createNode("div", "profile-section");
        const envHeader = createNode("div", "profile-row-header");
        envHeader.appendChild(createNode("div", "mock-label", "Profile variables"));
        const addEnvButton = createNode("button", "wizard-button", "Add variable");
        addEnvButton.type = "button";
        addEnvButton.addEventListener("click", () => {
          state.draft.envVars.push({ key: "", value: "" });
          renderProfile(windowId, true);
        });
        envHeader.appendChild(addEnvButton);
        envSection.appendChild(envHeader);
        const envTable = createNode("div", "profile-table");
        if (state.draft.envVars.length === 0) {
          envTable.appendChild(
            createNode("div", "profile-empty-copy", "No env overrides for this profile."),
          );
        }
        state.draft.envVars.forEach((entry, index) => {
          const row = createNode("div", "profile-table-row");
          const keyInput = document.createElement("input");
          keyInput.type = "text";
          keyInput.placeholder = "KEY";
          // SPEC-2356 — env var rows have no surrounding <label>; use
          // aria-label so screen readers announce purpose. The row index
          // disambiguates rows within the same list.
          keyInput.setAttribute("aria-label", `Env var key, row ${index + 1}`);
          keyInput.value = entry.key;
          keyInput.addEventListener("input", () => {
            state.draft.envVars[index].key = keyInput.value;
            scheduleProfileSave(windowId);
          });
          keyInput.addEventListener("blur", () => flushProfileSave(windowId));
          row.appendChild(keyInput);

          const valueInput = document.createElement("input");
          valueInput.type = "text";
          valueInput.placeholder = "Value";
          valueInput.setAttribute("aria-label", `Env var value, row ${index + 1}`);
          valueInput.value = entry.value;
          valueInput.addEventListener("input", () => {
            state.draft.envVars[index].value = valueInput.value;
            scheduleProfileSave(windowId);
          });
          valueInput.addEventListener("blur", () => flushProfileSave(windowId));
          row.appendChild(valueInput);

          const removeButton = createNode("button", "icon-button", "×");
          removeButton.type = "button";
          removeButton.setAttribute("aria-label", "Delete env var");
          removeButton.addEventListener("click", () => {
            state.draft.envVars.splice(index, 1);
            renderProfile(windowId, true);
            scheduleProfileSave(windowId);
          });
          row.appendChild(removeButton);
          envTable.appendChild(row);
        });
        envSection.appendChild(envTable);
        editor.appendChild(envSection);

        const disabledSection = createNode("div", "profile-section");
        const disabledHeader = createNode("div", "profile-row-header");
        disabledHeader.appendChild(createNode("div", "mock-label", "Disabled OS variables"));
        const addDisabledButton = createNode("button", "wizard-button", "Add disabled key");
        addDisabledButton.type = "button";
        addDisabledButton.addEventListener("click", () => {
          state.draft.disabledEnv.push("");
          renderProfile(windowId, true);
        });
        disabledHeader.appendChild(addDisabledButton);
        disabledSection.appendChild(disabledHeader);
        const disabledTable = createNode("div", "profile-table");
        if (state.draft.disabledEnv.length === 0) {
          disabledTable.appendChild(
            createNode("div", "profile-empty-copy", "No disabled OS variables for this profile."),
          );
        }
        state.draft.disabledEnv.forEach((entry, index) => {
          const row = createNode("div", "profile-table-row profile-disabled-row");
          const keyInput = document.createElement("input");
          keyInput.type = "text";
          keyInput.placeholder = "SECRET_KEY";
          keyInput.setAttribute("aria-label", `Disabled env key, row ${index + 1}`);
          keyInput.value = entry;
          keyInput.addEventListener("input", () => {
            state.draft.disabledEnv[index] = keyInput.value;
            scheduleProfileSave(windowId);
          });
          keyInput.addEventListener("blur", () => flushProfileSave(windowId));
          row.appendChild(keyInput);

          const removeButton = createNode("button", "icon-button", "×");
          removeButton.type = "button";
          removeButton.setAttribute("aria-label", "Delete disabled env key");
          removeButton.addEventListener("click", () => {
            state.draft.disabledEnv.splice(index, 1);
            renderProfile(windowId, true);
            scheduleProfileSave(windowId);
          });
          row.appendChild(removeButton);
          disabledTable.appendChild(row);
        });
        disabledSection.appendChild(disabledTable);
        editor.appendChild(disabledSection);

        const previewSection = createNode("div", "profile-section");
        previewSection.appendChild(createNode("div", "mock-label", "Merged preview"));
        previewSection.appendChild(
          createNode(
            "div",
            "profile-empty-copy",
            "The backend computes this preview from the current OS environment, disabled keys, and profile overrides.",
          ),
        );
        const preview = createNode("div", "profile-preview");
        if ((snapshot.merged_preview || []).length === 0) {
          preview.appendChild(
            createNode("div", "profile-empty-copy", "No merged entries to preview yet."),
          );
        }
        for (const entry of snapshot.merged_preview || []) {
          const row = createNode("div", "profile-preview-row");
          row.appendChild(createNode("div", "profile-preview-key", entry.key));
          row.appendChild(createNode("div", "profile-preview-value", entry.value));
          preview.appendChild(row);
        }
        previewSection.appendChild(preview);
        editor.appendChild(previewSection);

        updateProfileStatus(windowId);
      }

      function submitBoardEntry(windowId) {
        const state = ensureBoardState(windowId);
        const body = state.composerBody.trim();
        if (!body) {
          state.error = "Entry body is required.";
          renderBoard(windowId);
          return;
        }
        const mentions = mentionsForBoardSubmit(state);
        state.loading = true;
        state.submitting = true;
        state.error = "";
        const parentId = state.replyParentId || null;
        state.pendingSubmit = {
          body,
          parentId,
          existingEntryIds: new Set(state.entries.map((entry) => entry.id)),
        };
        send({
          kind: "post_board_entry",
          id: windowId,
          entry_kind: state.composerKind,
          body,
          parent_id: parentId,
          topics: [],
          owners: [],
          mentions,
        });
        renderBoard(windowId);
      }

      function forceBoardScrollToBottom(scroller) {
        scroller.scrollTop = scroller.scrollHeight;
      }

      function preserveBoardScrollPosition(scroller, previousScrollTop, previousScrollHeight) {
        const delta = scroller.scrollHeight - previousScrollHeight;
        scroller.scrollTop = previousScrollTop + Math.max(0, delta);
      }

      function mergeBoardEntries(existingEntries, incomingEntries) {
        const merged = new Map();
        for (const entry of existingEntries || []) {
          if (entry.id) {
            merged.set(entry.id, entry);
          }
        }
        for (const entry of incomingEntries || []) {
          if (entry.id) {
            merged.set(entry.id, entry);
          }
        }
        return Array.from(merged.values()).sort((left, right) => {
          const leftKey = String(left.created_at || left.updated_at || "");
          const rightKey = String(right.created_at || right.updated_at || "");
          return leftKey.localeCompare(rightKey)
            || String(left.id || "").localeCompare(String(right.id || ""));
        });
      }

      function showBoardMentionNotification(entry, windowId) {
        if (!entry?.id) return;
        let toast = document.getElementById("board-mention-toast");
        if (!toast) {
          toast = document.createElement("button");
          toast.id = "board-mention-toast";
          toast.className = "board-mention-toast";
          toast.type = "button";
          document.body.appendChild(toast);
        }
        toast.textContent = `Board reply for you - ${boardEntryPreview(entry)}`;
        toast.onclick = () => {
          const state = ensureBoardState(windowId);
          applyBoardMentionNotificationFocus(state, entry.id);
          focusBoardEntry(entry.id);
          toast.remove();
        };
        setTimeout(() => {
          if (document.getElementById("board-mention-toast") === toast) {
            toast.remove();
          }
        }, 8000);
      }

      function handleBoardHookEvent(event) {
        const hookEvent = event.event;
        if (!hookEvent || hookEvent.kind !== "coordination_event") {
          return;
        }
        const activeTab = activeProjectTab();
        if (!activeTab) {
          return;
        }
        if (hookEvent.project_root && hookEvent.project_root !== activeTab.project_root) {
          return;
        }
        for (const windowData of activeWorkspace().windows || []) {
          if (windowData.preset !== "board") {
            continue;
          }
          const state = ensureBoardState(windowData.id);
          if (!state.loading) {
            requestBoard(windowData.id);
          }
          renderBoard(windowData.id);
        }
      }

      function renderBoard(windowId) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const body = element.querySelector(".window-body");
        if (!body) {
          return;
        }
        const state = ensureBoardState(windowId);
        const status = body.querySelector(".board-status");
        const timeline = body.querySelector(".board-timeline");
        const composer = body.querySelector(".board-composer-pane");
        const forYouFilter = body.querySelector("[data-action='toggle-board-for-you']");
        if (!status || !timeline || !composer) {
          return;
        }
        if (pendingBoardEntryFocusId && !state.focusEntryId) {
          state.focusEntryId = pendingBoardEntryFocusId;
          state.pendingFocusScroll = true;
        }

        const entryCountLabel = `${state.entries.length} entr${state.entries.length === 1 ? "y" : "ies"}`;
        status.textContent = state.error
          ? state.error
          : state.loading
            ? state.submitting
              ? "Saving entry..."
              : "Loading coordination..."
            : state.loadingOlder
              ? `Loading earlier entries... - ${entryCountLabel}`
              : state.newEntriesAvailable
                ? `${entryCountLabel} - New updates`
                : entryCountLabel;
        status.className = "board-status";
        if (state.error) {
          status.classList.add("error");
        } else if (state.loading) {
          status.classList.add("info");
        }
        if (forYouFilter) {
          forYouFilter.setAttribute(
            "aria-pressed",
            state.audienceFilter === "for_you" ? "true" : "false",
          );
          forYouFilter.classList.toggle("active", state.audienceFilter === "for_you");
          forYouFilter.textContent =
            state.forYouUnread > 0 ? `For you (${state.forYouUnread})` : "For you";
        }

        // The actual scroll viewport is `.board-timeline-scroll`, the
        // parent wrapper that has `overflow: auto`. Reading scrollTop /
        // scrollHeight off `.board-timeline` returns 0/wrong values
        // because `.board-timeline` itself is sized to its content.
        const scroller = timeline.parentElement;
        const stickyBottomThreshold = 64;
        const previousScrollTop = scroller ? scroller.scrollTop : 0;
        const previousScrollHeight = scroller ? scroller.scrollHeight : 0;
        const previousScrollMax = scroller
          ? scroller.scrollHeight - scroller.clientHeight
          : 0;
        const shouldFollowBoardBottom =
          !scroller ||
          previousScrollMax <= 0 ||
          previousScrollMax - previousScrollTop <= stickyBottomThreshold;
        state.shouldFollowBoardBottom = shouldFollowBoardBottom;
        if (scroller && scroller.dataset.boardScrollBound !== "true") {
          scroller.dataset.boardScrollBound = "true";
          scroller.addEventListener("scroll", () => {
            const scrollMax = scroller.scrollHeight - scroller.clientHeight;
            const isNearBottom =
              scrollMax <= 0 || scrollMax - scroller.scrollTop <= stickyBottomThreshold;
            state.shouldFollowBoardBottom = isNearBottom;
            if (isNearBottom) {
              state.newEntriesAvailable = false;
            }
            if (scroller.scrollTop <= 48) {
              requestOlderBoardEntries(windowId);
            }
          });
        }

        const visibleEntries = visibleBoardEntries(state);

        timeline.innerHTML = "";
        if (state.hasMoreBefore) {
          const loadOlder = createNode(
            "button",
            "board-load-older",
            state.loadingOlder ? "Loading earlier entries..." : "Load earlier entries",
          );
          loadOlder.type = "button";
          loadOlder.disabled = state.loadingOlder;
          loadOlder.addEventListener("click", () => requestOlderBoardEntries(windowId));
          timeline.appendChild(loadOlder);
        }
        if (!state.loading && visibleEntries.length === 0) {
          timeline.appendChild(
            createNode(
              "div",
              "board-empty workspace-empty-state",
              state.audienceFilter === "for_you"
                ? "No posts addressed to you."
                : "No coordination entries yet.",
            ),
          );
        }
        let focusTarget = null;
        for (const entry of visibleEntries) {
          const authorKind = String(entry.author_kind || "").toLowerCase();
          let card;
          if (authorKind === "user") {
            card = createNode("article", "board-message user");
          } else if (authorKind === "system") {
            card = createNode("article", "board-message system");
          } else {
            card = createNode("article", "board-message agent");
          }
          if (entry.agent_color) {
            card.dataset.agentColor = entry.agent_color;
          }
          if (entry.id) {
            card.setAttribute("data-board-entry-id", entry.id);
          }
          if (state.focusEntryId && entry.id === state.focusEntryId) {
            card.classList.add("focus-target");
            card.tabIndex = -1;
            focusTarget = card;
          }
          if (boardEntryMentionsSelf(entry)) {
            card.classList.add("for-you");
            card.setAttribute("aria-label", "Board post addressed to you");
          }

          const meta = createNode("div", "board-message-meta");
          if (entry.agent_color) {
            meta.appendChild(createNode("span", "agent-dot"));
          }
          meta.appendChild(
            document.createTextNode(
              `${entry.author || "Unknown"} · ${boardTimestampLabel(
                entry.updated_at || entry.created_at,
              )}`,
            ),
          );
          for (const label of boardEntryAudienceLabels(entry)) {
            const badge = createNode("span", "board-audience-badge", label);
            if (label === "For you") {
              badge.classList.add("for-you");
            }
            meta.appendChild(badge);
          }
          card.appendChild(meta);
          if (entry.parent_id) {
            const parent = findBoardEntry(state, entry.parent_id);
            const quote = createNode(
              "button",
              "board-reply-quote",
              parent
                ? `Reply to ${parent.author || "Unknown"}: ${boardEntryPreview(parent)}`
                : "Reply to earlier Board entry",
            );
            quote.type = "button";
            quote.addEventListener("click", () => focusBoardEntry(entry.parent_id));
            card.appendChild(quote);
          }
          card.appendChild(createNode("div", "board-message-body", entry.body));
          const messageActions = createNode("div", "board-message-actions");
          const replyButton = createNode("button", "board-reply-button", "Reply");
          replyButton.type = "button";
          replyButton.addEventListener("click", () => {
            state.replyParentId = entry.id;
            renderBoard(windowId);
            const input = body.querySelector(".board-textarea");
            input?.focus();
          });
          messageActions.appendChild(replyButton);
          card.appendChild(messageActions);
          timeline.appendChild(card);
        }

        if (scroller) {
          if (focusTarget && state.pendingFocusScroll) {
            focusTarget.scrollIntoView({ block: "center" });
            focusTarget.focus({ preventScroll: true });
            state.pendingFocusScroll = false;
            pendingBoardEntryFocusId = null;
          } else if (state.pendingSelfPostScroll) {
            forceBoardScrollToBottom(scroller);
            state.pendingSelfPostScroll = false;
            state.newEntriesAvailable = false;
          } else if (state.preserveBoardScrollPosition) {
            preserveBoardScrollPosition(scroller, previousScrollTop, previousScrollHeight);
            state.preserveBoardScrollPosition = false;
          } else if (shouldFollowBoardBottom) {
            forceBoardScrollToBottom(scroller);
            state.newEntriesAvailable = false;
          } else {
            scroller.scrollTop = previousScrollTop;
          }
        }

        composer.innerHTML = "";
        if (state.replyParentId) {
          const parent = findBoardEntry(state, state.replyParentId);
          const banner = createNode("div", "board-reply-banner");
          banner.appendChild(
            createNode(
              "span",
              "board-reply-banner-text",
              parent
                ? `Replying to ${parent.author || "Unknown"} - ${boardEntryPreview(parent)}`
                : "Replying to earlier Board entry",
            ),
          );
          const jump = createNode("button", "text-button", "Jump to original");
          jump.type = "button";
          jump.addEventListener("click", () => focusBoardEntry(state.replyParentId));
          const cancel = createNode("button", "icon-button", "×");
          cancel.type = "button";
          cancel.setAttribute("aria-label", "Cancel reply");
          cancel.addEventListener("click", () => {
            state.replyParentId = null;
            renderBoard(windowId);
          });
          banner.appendChild(jump);
          banner.appendChild(cancel);
          composer.appendChild(banner);
        }
        const bodyField = createNode("label", "board-composer-field");
        bodyField.appendChild(createNode("span", "mock-label", "Share a Board update"));
        const bodyInput = document.createElement("textarea");
        bodyInput.className = "board-textarea board-scroll-surface";
        bodyInput.value = state.composerBody;
        bodyInput.placeholder = "Share the current state, next action, or blocker";
        bodyInput.addEventListener("input", () => {
          state.composerBody = bodyInput.value;
        });
        bodyInput.addEventListener("keydown", (event) => {
          if (event.key === "Enter" && event.shiftKey && !event.isComposing) {
            event.preventDefault();
            if (!state.submitting) {
              submitBoardEntry(windowId);
            }
          }
        });
        bodyField.appendChild(bodyInput);
        composer.appendChild(bodyField);

        const actions = createNode("div", "board-composer-actions");
        const submit = createNode(
          "button",
          "wizard-button primary",
          state.submitting ? "Saving..." : "Post",
        );
        submit.type = "button";
        submit.disabled = state.submitting;
        submit.addEventListener("click", () => submitBoardEntry(windowId));
        actions.appendChild(submit);
        composer.appendChild(actions);
      }

      function createLaunchSection(title, copy) {
        const section = createNode("section", "launch-section");
        const header = createNode("div", "launch-section-header");
        const text = createNode("div");
        text.appendChild(createNode("div", "launch-section-title", title));
        if (copy) {
          text.appendChild(createNode("div", "launch-section-copy", copy));
        }
        header.appendChild(text);
        section.appendChild(header);
        return section;
      }

      function createLaunchField(label, wide = false) {
        const field = createNode(
          "div",
          wide ? "launch-field wide" : "launch-field",
        );
        field.appendChild(createNode("div", "launch-field-label", label));
        return field;
      }

      function createChoiceButton(option, selected, onSelect) {
        const button = createNode("button", "launch-choice-button");
        button.type = "button";
        // SPEC-2356 — choice buttons toggle between mutually-exclusive
        // options (which agent to launch / which preset). aria-pressed
        // exposes the toggled state so screen readers announce which
        // option is currently selected without relying on the visual
        // .selected class alone.
        button.setAttribute("aria-pressed", selected ? "true" : "false");
        if (selected) {
          button.classList.add("selected");
        }
        const title = createNode("span", "launch-choice-title");
        if (option.color) {
          button.dataset.agentColor = option.color;
          title.appendChild(createNode("span", "agent-dot"));
        }
        title.appendChild(document.createTextNode(option.label));
        button.appendChild(title);
        if (option.description) {
          button.appendChild(
            createNode("span", "launch-choice-detail", option.description),
          );
        }
        button.addEventListener("click", onSelect);
        return button;
      }

      function appendChoiceField(
        parent,
        label,
        options,
        selectedValue,
        onSelect,
        wide = false,
      ) {
        const field = createLaunchField(label, wide);
        const row = createNode("div", "launch-choice-row");
        for (const option of options) {
          row.appendChild(
            createChoiceButton(option, option.value === selectedValue, () =>
              onSelect(option.value),
            ),
          );
        }
        field.appendChild(row);
        parent.appendChild(field);
        return field;
      }

      function appendSelectField(
        parent,
        label,
        options,
        selectedValue,
        onChange,
        wide = false,
        emptyLabel = "Unavailable",
      ) {
        const field = createLaunchField(label, wide);
        const select = createNode("select", "launch-select");
        // SPEC-2356 — launch-field labels are non-<label> divs, so set
        // aria-label directly. Reuses the visible label text so screen
        // readers and visual users see the same name.
        select.setAttribute("aria-label", label);
        if (options.length === 0) {
          select.disabled = true;
          const option = document.createElement("option");
          option.value = "";
          option.textContent = emptyLabel;
          select.appendChild(option);
        } else {
          for (const item of options) {
            const option = document.createElement("option");
            option.value = item.value;
            option.textContent = item.label;
            select.appendChild(option);
          }
          const hasSelected = options.some((item) => item.value === selectedValue);
          select.value = hasSelected ? selectedValue : options[0].value;
          select.addEventListener("change", () => onChange(select.value));
        }
        field.appendChild(select);
        parent.appendChild(field);
        return field;
      }

      function appendCheckboxField(
        parent,
        label,
        copy,
        checked,
        onChange,
        wide = false,
      ) {
        const field = createLaunchField(label, wide);
        const checkboxLabel = createNode("label", "launch-inline-check");
        const input = document.createElement("input");
        input.type = "checkbox";
        input.checked = checked;
        input.addEventListener("change", () => onChange(input.checked));
        checkboxLabel.appendChild(input);
        checkboxLabel.appendChild(createNode("span", "", copy));
        field.appendChild(checkboxLabel);
        parent.appendChild(field);
        return field;
      }

      function runtimeTargetPayload(value) {
        return value === "docker" ? "Docker" : "Host";
      }

      function dockerLifecyclePayload(value) {
        switch (value) {
          case "connect":
            return "Connect";
          case "start":
            return "Start";
          case "restart":
            return "Restart";
          case "recreate":
            return "Recreate";
          case "create_and_start":
            return "CreateAndStart";
          default:
            return "Connect";
        }
      }

      function syncWizardDraftState() {
        if (!launchWizard) {
          wizardWasOpen = false;
          wizardBranchDraft = "";
          wizardBranchBackendValue = "";
          return;
        }

        if (!wizardWasOpen) {
          wizardWasOpen = true;
          wizardBranchDraft = launchWizard.branch_name || "";
          wizardBranchBackendValue = wizardBranchDraft;
          return;
        }

        if ((launchWizard.branch_name || "") !== wizardBranchBackendValue) {
          wizardBranchDraft = launchWizard.branch_name || "";
          wizardBranchBackendValue = wizardBranchDraft;
        }
      }

      function flushWizardBranchDraft() {
        if (!launchWizard || launchWizard.branch_mode !== "create_new") {
          return;
        }
        if (wizardBranchDraft === wizardBranchBackendValue) {
          return;
        }
        wizardBranchBackendValue = wizardBranchDraft;
        sendWizardAction({
          kind: "set_branch_name",
          value: wizardBranchDraft,
        });
      }

      function renderWizardSummary() {
        wizardSummary.innerHTML = "";
        for (const item of launchWizard.launch_summary || []) {
          const card = createNode("div", "wizard-summary-item");
          card.appendChild(createNode("div", "wizard-summary-label", item.label));
          card.appendChild(createNode("div", "wizard-summary-value", item.value));
          wizardSummary.appendChild(card);
        }
      }

      let wizardFocusReturn = null;
      let wizardFocusTrapRelease = null;

      function renderLaunchWizard() {
        if (!launchWizard) {
          const wasOpenBeforeClose = wizardModal.classList.contains("open");
          wizardModal.classList.remove("open");
          // SPEC-2356 — keep aria-hidden in lockstep with .open so screen
          // readers stop announcing the wizard when it slides closed.
          wizardModal.setAttribute("aria-hidden", "true");
          // SPEC-2356 — release the focus trap before restoring focus so
          // the trap doesn't intercept the focus move and pull it back in.
          if (wasOpenBeforeClose && typeof wizardFocusTrapRelease === "function") {
            wizardFocusTrapRelease();
            wizardFocusTrapRelease = null;
          }
          // SPEC-2356 — restore focus to the trigger that opened the wizard
          // so keyboard users land back on Start Work / Launch Agent / etc.
          if (wasOpenBeforeClose && wizardFocusReturn && typeof wizardFocusReturn.focus === "function") {
            try { wizardFocusReturn.focus({ preventScroll: true }); }
            catch { wizardFocusReturn.focus(); }
            wizardFocusReturn = null;
          }
          wizardModal.classList.remove("is-drawer");
          wizardDialog?.classList.remove("is-drawer-shell");
          wizardSummary.innerHTML = "";
          wizardBody.innerHTML = "";
          wizardError.hidden = true;
          wizardError.textContent = "";
          if (wizardTitle) wizardTitle.textContent = "Launch Agent";
          wizardSubmitButton.textContent = "Launch";
          wizardSubmitButton.disabled = false;
          syncWizardDraftState();
          return;
        }

        syncWizardDraftState();
        closeModal();
        const isStartWorkMode = launchWizard.show_branch_controls === false;
        wizardModal.classList.toggle("is-drawer", isStartWorkMode);
        wizardDialog?.classList.toggle("is-drawer-shell", isStartWorkMode);
        const wasOpenWizard = wizardModal.classList.contains("open");
        if (!wasOpenWizard) {
          // Capture trigger BEFORE flipping .open so render-driven focus
          // moves don't overwrite our save.
          wizardFocusReturn = document.activeElement;
        }
        wizardModal.classList.add("open");
        wizardModal.removeAttribute("aria-hidden");
        if (!wasOpenWizard && wizardDialog && typeof wizardDialog.focus === "function") {
          // SPEC-2356 — move focus into the dialog so screen readers
          // announce "Launch Agent dialog" and keyboard users land inside.
          try { wizardDialog.focus({ preventScroll: true }); }
          catch { wizardDialog.focus(); }
        }
        if (!wasOpenWizard && wizardDialog) {
          // SPEC-2356 — trap Tab inside the wizard while it's open so
          // keyboard users can't escape into background content.
          wizardFocusTrapRelease = createFocusTrap(wizardDialog, { document });
        }
        if (wizardTitle) wizardTitle.textContent = launchWizard.title || "Launch Agent";
        wizardMeta.textContent = launchWizard.show_branch_controls === false
          ? "Workspace launch"
          : `Selected branch · ${
            launchWizard.selected_branch_name || launchWizard.branch_name || "Workspace"
          }`;
        wizardSubmitButton.textContent = launchWizard.is_hydrating
          ? "Loading..."
          : launchWizard.branch_mode === "create_new"
            ? "Create and launch"
            : "Launch";
        wizardSubmitButton.disabled = Boolean(launchWizard.is_hydrating);

        if (launchWizard.error || launchWizard.hydration_error) {
          wizardError.hidden = false;
          wizardError.textContent =
            launchWizard.error || launchWizard.hydration_error;
        } else {
          wizardError.hidden = true;
          wizardError.textContent = "";
        }

        renderWizardSummary();
        wizardBody.innerHTML = "";
        const panel = createNode("div", "launch-panel");
        if (launchWizard.is_hydrating) {
          panel.appendChild(
            createNode(
              "div",
              "launch-note",
              "Loading branch workspace, recent sessions, and Docker options...",
            ),
          );
        }

        if (
          (launchWizard.quick_start_entries || []).length > 0 ||
          (launchWizard.live_sessions || []).length > 0
        ) {
          const section = createLaunchSection(
            "Quick start",
            "Reuse a recent launch profile or jump to a running window.",
          );

          if ((launchWizard.quick_start_entries || []).length > 0) {
            const quickStartGrid = createNode("div", "quick-start-grid");
            for (const entry of launchWizard.quick_start_entries) {
              const card = createNode("div", "quick-start-card");
              const head = createNode("div", "quick-start-head");
              head.appendChild(
                createNode("div", "quick-start-title", entry.tool_label),
              );
              head.appendChild(
                createNode("div", "quick-start-meta", entry.summary),
              );
              if (entry.resume_session_id) {
                head.appendChild(
                  createNode(
                    "div",
                    "quick-start-secondary",
                    `Resume ID · ${entry.resume_session_id}`,
                  ),
                );
              }
              card.appendChild(head);

              const actions = createNode("div", "quick-start-actions");
              if (entry.reuse_action_label) {
                const reuseButton = createNode(
                  "button",
                  "wizard-button",
                  entry.reuse_action_label,
                );
                reuseButton.type = "button";
                reuseButton.addEventListener("click", () => {
                  sendWizardAction({
                    kind: "apply_quick_start",
                    index: entry.index,
                    mode: "resume",
                  });
                });
                actions.appendChild(reuseButton);
              }

              const startNewButton = createNode(
                "button",
                "wizard-button primary",
                "Start new",
              );
              startNewButton.type = "button";
              startNewButton.addEventListener("click", () => {
                sendWizardAction({
                  kind: "apply_quick_start",
                  index: entry.index,
                  mode: "start_new",
                });
              });
              actions.appendChild(startNewButton);

              card.appendChild(actions);
              quickStartGrid.appendChild(card);
            }
            section.appendChild(quickStartGrid);
          }

          if ((launchWizard.live_sessions || []).length > 0) {
            const liveSection = createNode("div", "live-session-list");
            for (const session of launchWizard.live_sessions) {
              const button = createNode("button", "live-session-button");
              button.type = "button";
              button.appendChild(
                createNode("div", "live-session-name", session.name),
              );
              button.appendChild(
                createNode(
                  "div",
                  "live-session-status",
                  session.active ? "Active window" : "Running window",
                ),
              );
              if (session.detail) {
                button.appendChild(
                  createNode("div", "live-session-detail", session.detail),
                );
              }
              button.addEventListener("click", () => {
                sendWizardAction({
                  kind: "focus_existing_session",
                  index: session.index,
                });
              });
              liveSection.appendChild(button);
            }
            section.appendChild(liveSection);
          }

          panel.appendChild(section);
        }

        if (launchWizard.show_branch_controls !== false) {
          const section = createLaunchSection(
            "Branch",
            "Choose the selected branch or create a new branch from it.",
          );
          const grid = createNode("div", "launch-form-grid");
          appendChoiceField(
            grid,
            "Branch target",
            [
              {
                value: "use_selected",
                label: "Use selected",
                description:
                  launchWizard.selected_branch_name || "Launch on the selected branch",
              },
              {
                value: "create_new",
                label: "Create new",
                description: `Base · ${
                  launchWizard.selected_branch_name || "selected branch"
                }`,
              },
            ],
            launchWizard.branch_mode,
            (value) => {
              sendWizardAction({
                kind: "set_branch_mode",
                create_new: value === "create_new",
              });
            },
            true,
          );

          if (launchWizard.branch_mode === "create_new") {
            appendChoiceField(
              grid,
              "Branch type",
              launchWizard.branch_type_options || [],
              launchWizard.selected_branch_type,
              (value) => {
                flushWizardBranchDraft();
                sendWizardAction({
                  kind: "set_branch_type",
                  prefix: value,
                });
              },
              true,
            );
            const field = createLaunchField("Branch name", true);
            const input = createNode("input", "launch-input");
            input.type = "text";
            // SPEC-2356 — launch-field labels are <div>s (not <label>s)
            // so screen readers can't programmatically associate them
            // with the input. Set aria-label directly so the input
            // announces with its purpose ("Branch name, edit text").
            input.setAttribute("aria-label", "Branch name");
            input.value = wizardBranchDraft;
            input.placeholder = "feature/my-task";
            input.addEventListener("input", () => {
              wizardBranchDraft = input.value;
            });
            input.addEventListener("blur", () => {
              flushWizardBranchDraft();
            });
            field.appendChild(input);
            field.appendChild(
              createNode(
                "div",
                "launch-field-help",
                `Base branch · ${launchWizard.selected_branch_name || "selected"}`,
              ),
            );
            grid.appendChild(field);
          } else {
            const note = createLaunchField("Resolved target", true);
            note.appendChild(
              createNode(
                "div",
                "launch-note",
                launchWizard.selected_branch_name || launchWizard.branch_name,
              ),
            );
            grid.appendChild(note);
          }

          section.appendChild(grid);
          panel.appendChild(section);
        }

        {
          const section = createLaunchSection(
            "Launch",
            "Choose what to launch on the selected branch.",
          );
          const grid = createNode("div", "launch-form-grid");
          appendSelectField(
            grid,
            "Target",
            launchWizard.launch_target_options || [],
            launchWizard.selected_launch_target,
            (value) =>
              sendWizardAction({
                kind: "set_launch_target",
                target: value === "shell" ? "shell" : "agent",
              }),
          );
          if (launchWizard.show_agent_settings) {
            appendSelectField(
              grid,
              "Agent",
              launchWizard.agent_options || [],
              launchWizard.selected_agent_id,
              (value) =>
                sendWizardAction({
                  kind: "set_agent",
                  agent_id: value,
                }),
            );
            if ((launchWizard.model_options || []).length > 0) {
              appendSelectField(
                grid,
                "Model",
                launchWizard.model_options || [],
                launchWizard.selected_model,
                (value) =>
                  sendWizardAction({
                    kind: "set_model",
                    model: value,
                  }),
              );
            }
            if (launchWizard.show_reasoning) {
              appendSelectField(
                grid,
                "Reasoning",
                launchWizard.reasoning_options || [],
                launchWizard.selected_reasoning,
                (value) =>
                  sendWizardAction({
                    kind: "set_reasoning",
                    reasoning: value,
                  }),
              );
            }
            if (launchWizard.show_execution_mode) {
              appendSelectField(
                grid,
                "Execution mode",
                launchWizard.execution_mode_options || [],
                launchWizard.selected_execution_mode,
                (value) =>
                  sendWizardAction({
                    kind: "set_execution_mode",
                    mode: value,
                  }),
              );
            }
          } else {
            const note = createLaunchField("Shell", true);
            note.appendChild(
              createNode(
                "div",
                "launch-note",
                "Open a plain shell in the selected branch and runtime.",
              ),
            );
            grid.appendChild(note);
          }
          if (launchWizard.show_windows_shell) {
            appendSelectField(
              grid,
              "Shell",
              launchWizard.windows_shell_options || [],
              launchWizard.selected_windows_shell,
              (value) =>
                sendWizardAction({
                  kind: "set_windows_shell",
                  shell: value,
                }),
            );
          }
          section.appendChild(grid);
          panel.appendChild(section);
        }

        if (
          launchWizard.show_version ||
          launchWizard.show_skip_permissions ||
          launchWizard.show_codex_fast_mode
        ) {
          const section = createLaunchSection(
            "Launch settings",
            "Version, permissions, and tool-specific launch behavior.",
          );
          const grid = createNode("div", "launch-form-grid");
          if (launchWizard.show_version) {
            appendSelectField(
              grid,
              "Version",
              launchWizard.version_options || [],
              launchWizard.selected_version,
              (value) =>
                sendWizardAction({
                  kind: "set_version",
                  version: value,
                }),
            );
          }
          if (launchWizard.show_skip_permissions) {
            appendCheckboxField(
              grid,
              "Permissions",
              "Skip permission prompts",
              launchWizard.skip_permissions,
              (enabled) =>
                sendWizardAction({
                  kind: "set_skip_permissions",
                  enabled,
                }),
            );
          }
          if (launchWizard.show_codex_fast_mode) {
            appendCheckboxField(
              grid,
              "Codex fast mode",
              "Use the fast service tier",
              launchWizard.codex_fast_mode,
              (enabled) =>
                sendWizardAction({
                  kind: "set_codex_fast_mode",
                  enabled,
                }),
            );
          }
          section.appendChild(grid);
          panel.appendChild(section);
        }

        if (launchWizard.show_agent_settings) {
          const section = createLaunchSection(
            "Linked issue",
            "Optional: Link an issue to this launch session.",
          );
          const grid = createNode("div", "launch-form-grid");
          const field = createLaunchField("Issue number", false);
          const input = createNode("input", "launch-input");
          input.type = "number";
          input.min = "1";
          // SPEC-2356 — see createLaunchField comment: aria-label is the
          // programmatic name since launch-field labels are non-<label> divs.
          input.setAttribute("aria-label", "Issue number");
          input.value = launchWizard.linked_issue_number
            ? launchWizard.linked_issue_number.toString()
            : "";
          input.placeholder = "e.g., 1938";
          input.addEventListener("change", () => {
            const value = input.value.trim();
            if (value) {
              sendWizardAction({
                kind: "set_linked_issue",
                issue_number: parseInt(value, 10),
              });
            } else {
              sendWizardAction({ kind: "clear_linked_issue" });
            }
          });
          field.appendChild(input);
          grid.appendChild(field);
          section.appendChild(grid);
          panel.appendChild(section);
        }

        if (
          launchWizard.show_runtime_target ||
          (launchWizard.show_docker_service &&
            (launchWizard.docker_service_options || []).length > 0) ||
          (launchWizard.show_docker_lifecycle &&
            (launchWizard.docker_lifecycle_options || []).length > 0)
        ) {
          const section = createLaunchSection(
            "Runtime",
            "Choose where the session runs and how Docker services are used.",
          );
          const grid = createNode("div", "launch-form-grid");
          if (launchWizard.show_runtime_target) {
            appendSelectField(
              grid,
              "Runtime target",
              launchWizard.runtime_target_options || [],
              launchWizard.selected_runtime_target,
              (value) =>
                sendWizardAction({
                  kind: "set_runtime_target",
                  target: runtimeTargetPayload(value),
                }),
            );
          }
          if (
            launchWizard.show_docker_service &&
            (launchWizard.docker_service_options || []).length > 0
          ) {
            appendSelectField(
              grid,
              "Docker service",
              launchWizard.docker_service_options || [],
              launchWizard.selected_docker_service,
              (value) =>
                sendWizardAction({
                  kind: "set_docker_service",
                  service: value,
                }),
            );
          }
          if (
            launchWizard.show_docker_lifecycle &&
            (launchWizard.docker_lifecycle_options || []).length > 0
          ) {
            appendSelectField(
              grid,
              "Docker lifecycle",
              launchWizard.docker_lifecycle_options || [],
              launchWizard.selected_docker_lifecycle,
              (value) =>
                sendWizardAction({
                  kind: "set_docker_lifecycle",
                  intent: dockerLifecyclePayload(value),
                }),
            );
          }
          section.appendChild(grid);

          panel.appendChild(section);
        }

        wizardBody.appendChild(panel);
      }

      function renderFileTree(windowId) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const state = ensureFileTreeState(windowId);
        const list = element.querySelector(".file-tree-list");
        const footer = element.querySelector(".file-tree-footer");
        if (!list || !footer) {
          return;
        }
        list.innerHTML = "";
        footer.textContent = state.selectedPath || ".";

        if (state.error) {
          const errorRow = document.createElement("div");
          errorRow.className = "file-tree-empty workspace-empty-state";
          errorRow.textContent = state.error;
          list.appendChild(errorRow);
        }

        if (!state.loaded.has("")) {
          const loadingRow = document.createElement("div");
          loadingRow.className = "file-tree-empty workspace-empty-state";
          loadingRow.textContent = "Loading repository";
          list.appendChild(loadingRow);
          return;
        }

        function appendRows(parentPath, depth) {
          const entries = state.loaded.get(parentPath) || [];
          for (const entry of entries) {
            const row = document.createElement("div");
            row.className = "file-tree-row";
            // SPEC-2356 — make the row keyboard-navigable. tabindex=0
            // puts the row in the natural Tab order; role="button"
            // tells assistive tech the row is activatable. The keydown
            // handler below mirrors the click handler for Enter/Space.
            row.tabIndex = 0;
            row.setAttribute("role", "button");
            // SPEC-2356 — file tree rows have a selected state but are
            // <div>s, not buttons. aria-current="true" works on any
            // element and announces "current item" to screen readers.
            if (state.selectedPath === entry.path) {
              row.classList.add("selected");
              row.setAttribute("aria-current", "true");
            } else {
              row.removeAttribute("aria-current");
            }
            row.style.paddingLeft = `${12 + depth * 18}px`;

            const expanded = state.expanded.has(entry.path);
            const isDirectory = entry.kind === "directory";
            // SPEC-2356 — directory rows expose collapse state via
            // aria-expanded so screen readers announce "expanded" or
            // "collapsed" alongside the visual ▾/▸ caret. File rows
            // (non-directories) should not expose aria-expanded —
            // that would falsely signal the element is collapsible.
            if (isDirectory) {
              row.setAttribute("aria-expanded", expanded ? "true" : "false");
            } else {
              row.removeAttribute("aria-expanded");
            }
            row.innerHTML = `
              <span class="tree-caret">${isDirectory ? (expanded ? "▾" : "▸") : ""}</span>
              <span class="tree-icon ${isDirectory ? "dir" : "file"}">${isDirectory ? "▣" : "•"}</span>
              <span class="tree-name">${entry.name}</span>
            `;
            const activate = () => {
              state.selectedPath = entry.path;
              if (isDirectory) {
                if (state.expanded.has(entry.path)) {
                  state.expanded.delete(entry.path);
                } else {
                  state.expanded.add(entry.path);
                  if (!state.loaded.has(entry.path)) {
                    requestFileTree(windowId, entry.path);
                  }
                }
              }
              renderFileTree(windowId);
            };
            row.addEventListener("click", activate);
            // SPEC-2356 — keyboard activation: Enter and Space invoke the
            // same handler as click so keyboard users can navigate and
            // activate rows without a pointing device.
            row.addEventListener("keydown", (event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                activate();
              }
            });
            list.appendChild(row);

            if (isDirectory && state.expanded.has(entry.path)) {
              if (state.loaded.has(entry.path)) {
                appendRows(entry.path, depth + 1);
              } else {
                const loadingRow = document.createElement("div");
                loadingRow.className = "file-tree-empty workspace-empty-state";
                loadingRow.style.paddingLeft = `${30 + depth * 18}px`;
                loadingRow.textContent = "Loading";
                list.appendChild(loadingRow);
              }
            }
          }
        }

        appendRows("", 0);

        if (list.childElementCount === 0) {
          const emptyRow = document.createElement("div");
          emptyRow.className = "file-tree-empty workspace-empty-state";
          emptyRow.textContent = "No visible files";
          list.appendChild(emptyRow);
        }
      }

      function createBranchRow(windowId, branchName) {
        const row = document.createElement("div");
        row.className = "branch-row";
        row.dataset.branchName = branchName;
        // SPEC-2356 — make the row keyboard-navigable. tabindex=0 puts
        // the row in the natural Tab order; role="button" tells assistive
        // tech the row is activatable. The keydown handler below mirrors
        // the click handler for Enter/Space (matches the file tree
        // pattern from PR #2464).
        row.tabIndex = 0;
        row.setAttribute("role", "button");

        const toggle = document.createElement("button");
        toggle.type = "button";
        toggle.className = "branch-cleanup-toggle";
        toggle.addEventListener("click", (event) => {
          event.stopPropagation();
          toggleBranchCleanupSelection(windowId, branchName);
        });
        row.appendChild(toggle);

        const main = document.createElement("div");
        main.className = "branch-main";

        const nameContainer = document.createElement("div");
        nameContainer.className = "branch-name";
        const nameText = document.createElement("span");
        nameText.className = "branch-name-text";
        nameContainer.appendChild(nameText);
        main.appendChild(nameContainer);

        const upstream = document.createElement("div");
        upstream.className = "branch-upstream";
        main.appendChild(upstream);

        const date = document.createElement("div");
        date.className = "branch-date";
        main.appendChild(date);

        row.appendChild(main);

        const meta = document.createElement("div");
        meta.className = "branch-meta";
        const scope = document.createElement("span");
        scope.className = "branch-scope";
        meta.appendChild(scope);
        const cleanupBadge = document.createElement("span");
        cleanupBadge.className = "branch-cleanup-badge";
        meta.appendChild(cleanupBadge);
        const summary = document.createElement("span");
        summary.className = "branch-summary";
        meta.appendChild(summary);
        row.appendChild(meta);

        row._fields = {
          toggle,
          main,
          nameContainer,
          nameText,
          headBadge: null,
          upstream,
          date,
          cleanupDetail: null,
          scope,
          cleanupBadge,
          summary,
        };

        const select = () => {
          const state = ensureBranchListState(windowId);
          state.selectedBranchName = branchName;
          state.notice = "";
          renderBranches(windowId);
        };
        const activate = () => {
          select();
          send({
            kind: "open_launch_wizard",
            id: windowId,
            branch_name: branchName,
          });
        };
        row.addEventListener("click", select);
        row.addEventListener("dblclick", activate);
        // SPEC-2356 — keyboard activation parity:
        //   Enter/Space → select (same as click)
        //   Cmd/Ctrl+Enter → activate (same as dblclick — open wizard)
        // Without this, keyboard-only users could Tab to a branch row
        // (after the tabindex/role wiring above) but couldn't select
        // or open the launch wizard from it.
        row.addEventListener("keydown", (event) => {
          if (event.key !== "Enter" && event.key !== " ") return;
          event.preventDefault();
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            activate();
          } else {
            select();
          }
        });

        return row;
      }

      function updateBranchRow(row, entry, state) {
        const fields = row._fields;
        row.classList.toggle("selected", state.selectedBranchName === entry.name);
        row.classList.toggle("cleanup-selected", state.cleanupSelected.has(entry.name));

        fields.toggle.className = `branch-cleanup-toggle ${cleanupToggleClass(entry, state)}`;
        fields.toggle.textContent = cleanupToggleSymbol(entry, state);
        fields.toggle.title = cleanupToggleTitle(entry, state);

        fields.nameText.textContent = entry.name;

        if (entry.is_head) {
          if (!fields.headBadge) {
            const head = document.createElement("span");
            head.className = "branch-head";
            head.textContent = "HEAD";
            fields.nameContainer.appendChild(head);
            fields.headBadge = head;
          }
        } else if (fields.headBadge) {
          fields.headBadge.remove();
          fields.headBadge = null;
        }

        fields.upstream.textContent = entry.upstream || "No upstream";
        fields.date.textContent = entry.last_commit_date || "No commit date";

        const cleanupDetail = cleanupDetailText(entry, state);
        if (cleanupDetail) {
          if (!fields.cleanupDetail) {
            const detail = document.createElement("div");
            fields.main.appendChild(detail);
            fields.cleanupDetail = detail;
          }
          fields.cleanupDetail.className = `branch-cleanup-detail ${
            cleanupAvailabilityForRender(entry, state) === "blocked" ? "blocked" : ""
          }`.trim();
          fields.cleanupDetail.textContent = cleanupDetail;
        } else if (fields.cleanupDetail) {
          fields.cleanupDetail.remove();
          fields.cleanupDetail = null;
        }

        fields.scope.textContent = entry.scope;
        fields.cleanupBadge.className =
          `branch-cleanup-badge ${cleanupAvailabilityForRender(entry, state)}`;
        fields.cleanupBadge.textContent = cleanupBadgeText(entry, state);
        fields.summary.textContent =
          entry.ahead || entry.behind ? `↑${entry.ahead} ↓${entry.behind}` : "synced";
      }

      function setBranchListPlaceholder(list, text) {
        let placeholder = null;
        for (const child of Array.from(list.children)) {
          if (!placeholder && child.classList.contains("branch-empty")) {
            placeholder = child;
          } else {
            child.remove();
          }
        }
        if (!placeholder) {
          placeholder = document.createElement("div");
          placeholder.className = "branch-empty workspace-empty-state";
          list.appendChild(placeholder);
        }
        placeholder.textContent = text;
      }

      function renderBranches(windowId) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const state = ensureBranchListState(windowId);
        syncBranchSelectionState(state);
        const list = element.querySelector(".branch-list");
        const notice = element.querySelector(".branch-notice");
        const cleanupButton = element.querySelector("[data-action='open-branch-cleanup']");
        if (!list) {
          return;
        }

        for (const button of element.querySelectorAll("[data-branch-filter]")) {
          button.classList.toggle("active", button.dataset.branchFilter === state.filter);
        }
        if (cleanupButton) {
          const selectedCount = selectedBranchCleanupEntries(windowId).length;
          cleanupButton.disabled = selectedCount === 0;
          cleanupButton.textContent =
            selectedCount === 0 ? "Clean Up" : `Clean Up (${selectedCount})`;
        }
        if (notice) {
          const noticeText = state.notice || branchLoadingNoticeText(state);
          notice.hidden = !noticeText;
          notice.textContent = noticeText || "";
        }

        if (state.error) {
          setBranchListPlaceholder(list, state.error);
          renderBranchCleanupModal();
          return;
        }

        if (state.loading && state.entries.length === 0) {
          setBranchListPlaceholder(list, "Loading branches");
          renderBranchCleanupModal();
          return;
        }

        const visibleEntries = filteredBranchEntries(state);
        if (visibleEntries.length === 0) {
          setBranchListPlaceholder(
            list,
            state.entries.length === 0 ? "No branches" : "No branches in this filter",
          );
          renderBranchCleanupModal();
          return;
        }

        const existingRows = new Map();
        for (const child of Array.from(list.children)) {
          if (child.classList.contains("branch-row") && child.dataset.branchName) {
            existingRows.set(child.dataset.branchName, child);
          } else {
            child.remove();
          }
        }

        let prevSibling = null;
        const usedNames = new Set();
        for (const entry of visibleEntries) {
          let row = existingRows.get(entry.name);
          if (!row) {
            row = createBranchRow(windowId, entry.name);
          }
          updateBranchRow(row, entry, state);
          const targetPosition = prevSibling ? prevSibling.nextSibling : list.firstChild;
          if (row !== targetPosition) {
            list.insertBefore(row, targetPosition);
          }
          prevSibling = row;
          usedNames.add(entry.name);
        }

        for (const [name, row] of existingRows) {
          if (!usedNames.has(name)) {
            row.remove();
          }
        }

        renderBranchCleanupModal();
      }

      function knowledgeHeading(kind) {
        switch (kind) {
          case "issue":
            return "Cached issues";
          case "spec":
            return "Cached SPECs";
          case "pr":
            return "PR bridge";
          default:
            return "Knowledge Bridge";
        }
      }

      function knowledgeSearchPlaceholder(kind, listScope = "open") {
        switch (kind) {
          case "issue":
            return listScope === "closed"
              ? "Semantic search closed issues"
              : "Semantic search open issues";
          case "spec":
            return "Semantic search cached SPECs";
          case "pr":
            return "Search unavailable";
          default:
            return "Search";
        }
      }

      function filteredKnowledgeEntries(state) {
        const query = state.query.trim().toLowerCase();
        if (!query) {
          return state.entries;
        }
        return state.entries.filter((entry) =>
          [
            `#${entry.number}`,
            entry.title,
            entry.meta,
            ...(entry.labels || []),
          ]
            .join(" ")
            .toLowerCase()
            .includes(query),
        );
      }

      function switchKnowledgeListScope(windowId, nextScope) {
        const state = ensureKnowledgeBridgeState(
          windowId,
          knowledgeKindForPreset(workspaceWindowById(windowId)?.preset),
        );
        if (state.kind !== "issue" || state.listScope === nextScope || state.loading) {
          return;
        }
        if (state.pendingSearchTimer) {
          clearTimeout(state.pendingSearchTimer);
          state.pendingSearchTimer = null;
        }
        state.listScope = nextScope;
        state.entries = [];
        state.baseEntries = [];
        state.selectedNumber = null;
        state.detail = null;
        state.detailLoading = false;
        state.query = "";
        state.searching = false;
        state.refreshing = false;
        state.searchInFlight = false;
        state.inFlightSearchRequestId = 0;
        state.queuedSearchQuery = "";
        state.loadRequestId += 1;
        state.searchRequestId += 1;
        requestKnowledgeBridge(windowId, state.kind, false);
        renderKnowledgeBridge(windowId);
      }

      function kanbanEmptyMessage(state, phase) {
        if (state.searching) return "Searching";
        if (state.loading) return "Loading";
        if (phase === "backlog") return "No backlog items";
        return "Empty";
      }

      // SPEC-2017 US-8 — wire dragover / dragenter / dragleave / drop on
      // a Kanban column once. dragover preventDefault is required for
      // the drop event to fire; we also light up .is-drop-target as a
      // visual affordance. drop translates the column data-phase into
      // an `update_knowledge_bridge_phase` request, optimistically
      // moves the card DOM, and registers a pending entry so the card
      // shows a spinner until the response confirms.
      function wireKanbanColumnDropTarget(windowId, column) {
        column.addEventListener("dragover", (event) => {
          event.preventDefault();
          if (event.dataTransfer) {
            event.dataTransfer.dropEffect = "move";
          }
        });
        column.addEventListener("dragenter", (event) => {
          event.preventDefault();
          column.classList.add("is-drop-target");
        });
        column.addEventListener("dragleave", (event) => {
          // dragleave fires for child element transitions; only clear
          // the marker when leaving the column itself.
          if (event.target === column) {
            column.classList.remove("is-drop-target");
          }
        });
        column.addEventListener("drop", (event) => {
          event.preventDefault();
          column.classList.remove("is-drop-target");
          const raw = event.dataTransfer?.getData("text/plain");
          const issueNumber = raw ? Number.parseInt(raw, 10) : NaN;
          if (!Number.isFinite(issueNumber)) {
            return;
          }
          const state = ensureKnowledgeBridgeState(
            windowId,
            knowledgeKindForPreset(workspaceWindowById(windowId)?.preset),
          );
          const phaseKey = column.dataset.phase;
          if (!phaseKey) return;
          const targetPhase = phaseKey === "backlog" || phaseKey === "done"
            ? phaseKey === "done"
              ? "done"
              : null
            : phaseKey;
          // Optimistic UI: rewrite the entry's phase locally and
          // rerender so the card lands in the target column instantly.
          if (Array.isArray(state.entries)) {
            const index = state.entries.findIndex(
              (entry) => entry.number === issueNumber,
            );
            if (index >= 0) {
              state.entries[index] = {
                ...state.entries[index],
                phase: targetPhase,
                has_unknown_phase: false,
              };
            }
          }
          if (!state.pendingPhaseUpdates) {
            state.pendingPhaseUpdates = new Map();
          }
          state.pendingPhaseUpdates.set(
            issueNumber,
            sendUpdateKnowledgePhase(windowId, issueNumber, targetPhase),
          );
          renderKnowledgeBridge(windowId);
        });
      }

      function renderKanbanCard(windowId, state, entry) {
        const card = createNode("button", "kanban-card");
        card.type = "button";
        card.dataset.issueNumber = String(entry.number);
        // Plain (non-spec) Issues cannot be moved through phase columns
        // because they carry no canonical phase labels. We surface a
        // (plain) chip and disable HTML5 D&D so the user understands
        // the constraint at a glance.
        const isPlain = entry.is_spec === false;
        card.draggable = !isPlain;
        if (isPlain) {
          card.classList.add("kanban-card--plain");
        }
        if (state.selectedNumber === entry.number) {
          card.classList.add("is-selected");
          // SPEC-2356 — selected card announces aria-current="true" so
          // screen readers read which Kanban card is currently shown
          // in the detail pane (parallel to project tabs and the old
          // knowledge-row pattern).
          card.setAttribute("aria-current", "true");
        } else {
          card.removeAttribute("aria-current");
        }
        if (state.pendingPhaseUpdates && state.pendingPhaseUpdates.has(entry.number)) {
          card.classList.add("is-pending");
        }

        const head = createNode("div", "kanban-card-head");
        head.appendChild(
          createNode("span", "kanban-card-number", `#${entry.number}`),
        );
        const stateChip = createNode(
          "span",
          `kanban-card-chip kanban-card-chip--state-${entry.state}`,
          entry.state,
        );
        head.appendChild(stateChip);
        card.appendChild(head);

        card.appendChild(
          createNode("div", "kanban-card-title", entry.title),
        );

        const meta = createNode("div", "kanban-card-meta");
        if (isPlain) {
          meta.appendChild(
            createNode("span", "kanban-card-chip kanban-card-chip--plain", "(plain)"),
          );
        }
        if (entry.has_unknown_phase) {
          meta.appendChild(
            createNode(
              "span",
              "kanban-card-chip kanban-card-chip--warning",
              "Unknown phase",
            ),
          );
        }
        if (Number.isFinite(entry.match_score)) {
          meta.appendChild(
            createNode(
              "span",
              "kanban-card-chip",
              `${entry.match_score}% match`,
            ),
          );
        }
        if ((entry.linked_branch_count || 0) > 0) {
          meta.appendChild(
            createNode(
              "span",
              "kanban-card-chip",
              `${entry.linked_branch_count} branch${entry.linked_branch_count === 1 ? "" : "es"}`,
            ),
          );
        }
        if (meta.childElementCount > 0) {
          card.appendChild(meta);
        }

        card.addEventListener("click", () => {
          // SPEC-2017 US-9 — clicking a card opens the Drawer with the
          // freshest detail. We always request detail (cheap; cache-
          // backed) so reopening on the same card still pulls live
          // comment / linked-branch updates, and we always (re)open
          // the Drawer so a previously dismissed Drawer reappears.
          requestKnowledgeDetail(windowId, state.kind, entry.number);
          renderKnowledgeBridge(windowId);
          openKanbanDrawer({ windowId, kind: state.kind, number: entry.number });
        });

        // SPEC-2017 US-8 — D&D wire-up. Plain (is_spec=false) cards
        // skip these handlers entirely (draggable=false above) so they
        // can still be clicked but never picked up.
        if (!isPlain) {
          card.addEventListener("dragstart", (event) => {
            // Snapshot the original entry so a failed write-back can
            // restore it; the snapshot keeps the entire entry value
            // because labels / phase / state all change on success.
            state.dndSnapshot = {
              issueNumber: entry.number,
              entry: { ...entry },
              originPhase:
                entry.state === "closed" ? "done" : entry.phase || "backlog",
            };
            card.classList.add("is-dragging");
            if (event.dataTransfer) {
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData("text/plain", String(entry.number));
            }
          });
          card.addEventListener("dragend", () => {
            card.classList.remove("is-dragging");
          });
        }
        return card;
      }

      function renderKnowledgeBridge(windowId) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const state = ensureKnowledgeBridgeState(
          windowId,
          knowledgeKindForPreset(workspaceWindowById(windowId)?.preset),
        );
        const board = element.querySelector(".kanban-board");
        const detailPane = element.querySelector(".knowledge-detail-pane");
        const status = element.querySelector(".knowledge-status");
        const refreshButton = element.querySelector("[data-action='refresh-knowledge']");
        const searchInput = element.querySelector(".knowledge-search");
        const scopeButtons = element.querySelectorAll("[data-knowledge-scope]");
        const hideDoneToggle = element.querySelector("[data-action='kanban-hide-done']");
        if (!board || !detailPane || !status || !refreshButton || !searchInput) {
          return;
        }

        refreshButton.disabled = !state.refreshEnabled || state.loading;
        searchInput.placeholder = knowledgeSearchPlaceholder(
          state.kind,
          state.listScope,
        );
        for (const button of scopeButtons) {
          const active = button.dataset.knowledgeScope === state.listScope;
          button.classList.toggle("active", active);
          button.disabled = state.loading && !active;
        }
        if (hideDoneToggle) {
          hideDoneToggle.checked = state.hideDone === true;
        }
        board.dataset.hideDone = state.hideDone === true ? "true" : "false";

        status.className = "knowledge-status";
        status.textContent = "";
        if (state.error) {
          status.classList.add("visible", "error");
          status.textContent = state.error;
        } else if (state.searching) {
          status.classList.add("visible", "info");
          status.textContent = "Searching semantic index";
        } else if (state.loading && state.entries.length > 0) {
          status.classList.add("visible", "info");
          status.textContent = state.refreshing
            ? "Refreshing cached knowledge"
            : "Loading cache-backed data";
        } else if (state.loading && state.entries.length === 0) {
          status.classList.add("visible", "info");
          status.textContent = "Loading cache-backed data";
        } else if (state.entries.length === 0 && !state.searching) {
          status.classList.add("visible", "info");
          status.textContent = state.emptyMessage || "No cached items";
        }

        // SPEC-2017 — Kanban grouping. Each entry routes to a single
        // column: closed Issues land in "done" regardless of phase
        // label so the Done column unifies state="closed" with the
        // phase/done open Issues; otherwise we trust entry.phase, with
        // null falling back to "backlog" so plain Issues and unlabeled
        // SPECs are never lost. Unknown phase labels stay in their
        // backend-extracted column but flag has_unknown_phase so the
        // card can warn the user about malformed metadata.
        const visibleEntries = state.query.trim()
          ? state.entries
          : filteredKnowledgeEntries(state);
        const columnsByPhase = new Map();
        for (const column of board.querySelectorAll(".kanban-column[data-phase]")) {
          const body = column.querySelector("[data-role='body']");
          if (body) {
            body.innerHTML = "";
          }
          columnsByPhase.set(column.dataset.phase, column);
          if (column.dataset.kanbanWired !== "true") {
            wireKanbanColumnDropTarget(windowId, column);
            column.dataset.kanbanWired = "true";
          }
        }
        const counts = new Map();
        for (const entry of visibleEntries) {
          const phaseKey =
            entry.state === "closed" ? "done" : entry.phase || "backlog";
          const column = columnsByPhase.get(phaseKey) || columnsByPhase.get("backlog");
          if (!column) continue;
          const body = column.querySelector("[data-role='body']");
          if (!body) continue;
          const card = renderKanbanCard(windowId, state, entry);
          body.appendChild(card);
          counts.set(phaseKey, (counts.get(phaseKey) || 0) + 1);
        }
        for (const [phase, column] of columnsByPhase) {
          const countLabel = column.querySelector("[data-role='count']");
          if (countLabel) {
            countLabel.textContent = String(counts.get(phase) || 0);
          }
          const body = column.querySelector("[data-role='body']");
          if (body && body.childElementCount === 0) {
            const empty = createNode(
              "div",
              "kanban-column-empty",
              kanbanEmptyMessage(state, phase),
            );
            body.appendChild(empty);
          }
        }

        detailPane.innerHTML = "";
        const detail = state.detail;
        if (!detail) {
          detailPane.appendChild(
            createNode("div", "knowledge-detail-empty", "Select a cached item"),
          );
          return;
        }

        const header = createNode("div", "knowledge-detail-header");
        const head = createNode("div", "");
        const headRow = createNode("div", "knowledge-detail-head");
        headRow.appendChild(createNode("h3", "knowledge-detail-title", detail.title));
        headRow.appendChild(
          createNode("span", `knowledge-state-chip ${detail.state}`, detail.state),
        );
        head.appendChild(headRow);
        if (detail.subtitle) {
          head.appendChild(
            createNode("div", "knowledge-detail-subtitle", detail.subtitle),
          );
        }
        if ((detail.labels || []).length > 0) {
          const labelRow = createNode("div", "knowledge-label-row");
          for (const label of detail.labels) {
            labelRow.appendChild(createNode("span", "knowledge-chip", label));
          }
          head.appendChild(labelRow);
        }
        header.appendChild(head);

        const actions = createNode("div", "knowledge-detail-actions");
        if (detail.launch_issue_number !== null && detail.launch_issue_number !== undefined) {
          const launchButton = createNode("button", "wizard-button primary", "Launch Agent");
          launchButton.type = "button";
          launchButton.addEventListener("click", () =>
            openIssueLaunchWizard(windowId, detail.launch_issue_number),
          );
          actions.appendChild(launchButton);
        }
        if (actions.childElementCount > 0) {
          header.appendChild(actions);
        }
        detailPane.appendChild(header);

        const scroll = createNode("div", "knowledge-detail-scroll workspace-scroll");
        if (state.detailLoading) {
          scroll.appendChild(
            createNode("div", "knowledge-detail-empty", "Loading detail"),
          );
        }
        for (const section of detail.sections || []) {
          const card = createNode("section", "knowledge-section");
          card.appendChild(
            createNode("div", "knowledge-section-title", section.title),
          );
          card.appendChild(
            createNode("pre", "knowledge-section-body", section.body),
          );
          scroll.appendChild(card);
        }
        if (scroll.childElementCount === 0) {
          scroll.appendChild(
            createNode("div", "knowledge-detail-empty", "No cached detail available"),
          );
        }
        detailPane.appendChild(scroll);
      }

      function filteredBranchEntries(state) {
        if (state.filter === "all") {
          return state.entries;
        }
        return state.entries.filter((entry) =>
          state.filter === "local" ? entry.scope === "local" : entry.scope === "remote",
        );
      }

      function syncBranchSelectionState(state) {
        const validBranchNames = new Set(state.entries.map((entry) => entry.name));
        state.cleanupSelected = new Set(
          Array.from(state.cleanupSelected).filter((name) => validBranchNames.has(name)),
        );
        if (state.selectedBranchName && !validBranchNames.has(state.selectedBranchName)) {
          state.selectedBranchName = "";
        }
      }

      function selectedBranchCleanupEntries(windowId) {
        const state = ensureBranchListState(windowId);
        const entriesByName = new Map(state.entries.map((entry) => [entry.name, entry]));
        return Array.from(state.cleanupSelected)
          .map((name) => entriesByName.get(name))
          .filter((entry) => Boolean(entry?.cleanup_ready));
      }

      function branchCleanupFailureResults(state, message) {
        const selectedBranches = Array.from(state.cleanupSelected);
        if (selectedBranches.length === 0) {
          return [
            {
              branch: "Cleanup request",
              execution_branch: null,
              status: "failed",
              message,
            },
          ];
        }
        return selectedBranches.map((branch) => ({
          branch,
          execution_branch: null,
          status: "failed",
          message,
        }));
      }

      function failRunningBranchCleanup(windowId, message) {
        const state = ensureBranchListState(windowId);
        if (state.cleanupModal.stage !== "running") {
          return false;
        }
        state.cleanupModal.open = true;
        state.cleanupModal.stage = "result";
        state.cleanupModal.results = branchCleanupFailureResults(state, message);
        branchCleanupWindowId = windowId;
        return true;
      }

      function cleanupToggleClass(entry, state) {
        if (state.cleanupSelected.has(entry.name)) {
          return "selected";
        }
        return cleanupAvailabilityForRender(entry, state);
      }

      function cleanupToggleSymbol(entry, state) {
        if (state.cleanupSelected.has(entry.name)) {
          return "●";
        }
        if (!entry.cleanup_ready) {
          return "…";
        }
        switch (entry.cleanup.availability) {
          case "safe":
            return "✓";
          case "risky":
            return "·";
          default:
            return "–";
        }
      }

      function cleanupToggleTitle(entry, state) {
        if (state.cleanupSelected.has(entry.name)) {
          return "Selected for cleanup";
        }
        if (!entry.cleanup_ready) {
          return branchCleanupPendingText(state);
        }
        if (entry.cleanup.availability === "blocked") {
          return cleanupBlockedReasonText(entry.cleanup.blocked_reason);
        }
        if (entry.cleanup.risks?.length) {
          return cleanupRiskLabels(entry.cleanup.risks).join(", ");
        }
        return "Select for cleanup";
      }

      function cleanupDetailText(entry, state) {
        if (!entry.cleanup_ready) {
          return branchCleanupPendingText(state);
        }
        if (entry.cleanup.availability === "blocked") {
          return cleanupBlockedReasonText(entry.cleanup.blocked_reason);
        }
        if (entry.cleanup.risks?.length) {
          return cleanupRiskLabels(entry.cleanup.risks).join(", ");
        }
        return cleanupMergeTargetText(entry.cleanup.merge_target);
      }

      function cleanupBlockedReasonText(reason) {
        switch (reason) {
          case "protected_branch":
            return "Protected branches cannot be cleaned up";
          case "current_head":
            return "The current HEAD branch cannot be cleaned up";
          case "active_session":
            return "A running agent session is using this branch";
          case "remote_tracking_without_local":
            return "Remote-tracking branch without a local counterpart";
          case "non_workspace_branch":
            return "Only gwt-managed workspaces can be cleaned up";
          default:
            return "This branch cannot be cleaned up";
        }
      }

      function cleanupRiskLabels(risks) {
        return (risks || []).map((risk) => {
          switch (risk) {
            case "remote_tracking":
              return "remote-tracking";
            case "unmerged":
              return "unmerged";
            default:
              return "warning";
          }
        });
      }

      function cleanupMergeTargetText(target) {
        if (!target) {
          return "";
        }
        if (target.kind === "gone") {
            return "upstream is gone";
        }
        return target.reference ? `merged to ${target.reference}` : "";
      }

      function branchLoadingNoticeText(state) {
        if (!state.loading) {
          return "";
        }
        return state.entries.length === 0 ? "" : "Loading branch details";
      }

      function branchCleanupPendingText(state) {
        return state.loading ? "Loading cleanup status" : "Cleanup status unavailable";
      }

      function cleanupAvailabilityForRender(entry, state) {
        return entry.cleanup_ready ? entry.cleanup.availability : "loading";
      }

      function cleanupBadgeText(entry, state) {
        return entry.cleanup_ready ? entry.cleanup.availability : state.loading ? "loading" : "unknown";
      }

      function toggleBranchCleanupSelection(windowId, branchName) {
        const state = ensureBranchListState(windowId);
        const entry = state.entries.find((candidate) => candidate.name === branchName);
        if (!entry) {
          return;
        }
        if (!entry.cleanup_ready) {
          state.notice = branchCleanupPendingText(state);
          renderBranches(windowId);
          return;
        }
        if (entry.cleanup.availability === "blocked") {
          state.notice = cleanupBlockedReasonText(entry.cleanup.blocked_reason);
          renderBranches(windowId);
          return;
        }
        state.notice = "";
        if (state.cleanupSelected.has(branchName)) {
          state.cleanupSelected.delete(branchName);
        } else {
          state.cleanupSelected.add(branchName);
        }
        renderBranches(windowId);
      }

      function openBranchCleanupModal(windowId) {
        const state = ensureBranchListState(windowId);
        if (selectedBranchCleanupEntries(windowId).length === 0) {
          state.notice = "Select at least one branch for cleanup";
          renderBranches(windowId);
          return;
        }
        state.notice = "";
        state.cleanupModal.open = true;
        state.cleanupModal.stage = "confirm";
        state.cleanupModal.deleteRemote = false;
        state.cleanupModal.results = [];
        branchCleanupWindowId = windowId;
        renderBranches(windowId);
      }

      function closeBranchCleanupModal(windowId = branchCleanupWindowId) {
        if (!windowId) {
          branchCleanupWindowId = null;
          renderBranchCleanupModal();
          return;
        }
        const state = ensureBranchListState(windowId);
        if (state.cleanupModal.stage === "running") {
          return;
        }
        state.cleanupModal.open = false;
        state.cleanupModal.stage = "confirm";
        state.cleanupModal.deleteRemote = false;
        state.cleanupModal.results = [];
        if (branchCleanupWindowId === windowId) {
          branchCleanupWindowId = null;
        }
        renderBranchCleanupModal();
      }

      function runBranchCleanup(windowId) {
        const state = ensureBranchListState(windowId);
        const branches = Array.from(state.cleanupSelected);
        if (branches.length === 0) {
          state.notice = "Select at least one branch for cleanup";
          renderBranches(windowId);
          return;
        }
        state.notice = "";
        state.cleanupModal.stage = "running";
        state.cleanupModal.results = [];
        renderBranchCleanupModal();
        if (windowId === WORKSPACE_CLEANUP_WINDOW_ID) {
          send({
            kind: "run_workspace_cleanup",
            branch: branches[0],
            delete_remote: state.cleanupModal.deleteRemote,
          });
          return;
        }
        send({
          kind: "run_branch_cleanup",
          id: windowId,
          branches,
          delete_remote: state.cleanupModal.deleteRemote,
        });
      }

      function branchCleanupResultSummary(results) {
        const counts = { success: 0, partial: 0, failed: 0 };
        for (const result of results || []) {
          counts[result.status] = (counts[result.status] || 0) + 1;
        }
        return `success ${counts.success} · partial ${counts.partial} · failed ${counts.failed}`;
      }

      function renderBranchCleanupModal() {
        const windowId = branchCleanupWindowId;
        const state = windowId ? ensureBranchListState(windowId) : null;
        const selectedEntries = windowId
          ? selectedBranchCleanupEntries(windowId)
          : [];
        renderBranchCleanupModalView({
          modalEl: branchCleanupModal,
          dialogEl: branchCleanupDialog,
          windowId,
          state,
          selectedEntries,
          createNode,
          resultSummary: branchCleanupResultSummary,
          mergeTargetText: cleanupMergeTargetText,
          riskLabels: cleanupRiskLabels,
          onCancel: () => closeBranchCleanupModal(windowId),
          onSubmit: () => runBranchCleanup(windowId),
          onDeleteRemoteToggle: (checked) => {
            if (state) {
              state.cleanupModal.deleteRemote = checked;
            }
          },
        });
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
          onSkip: () => {
            const tabId = migrationModalState.tabId;
            migrationModalState.open = false;
            migrationModalState.stage = "confirm";
            migrationModalState.message = "";
            migrationModalState.recovery = "";
            renderMigrationModal();
            if (tabId) {
              send({ kind: "skip_migration", tab_id: tabId });
            }
          },
          onQuit: () => {
            const tabId = migrationModalState.tabId;
            migrationModalState.open = false;
            renderMigrationModal();
            if (tabId) {
              send({ kind: "quit_migration", tab_id: tabId });
            }
          },
        });
      }

      function mountWindowBody(windowData, element) {
        const body = element.querySelector(".window-body");
        body.innerHTML = "";
        const surface = presetSurface(windowData.preset);
        element.classList.remove(
          "surface-terminal",
          "surface-file-tree",
          "surface-branches",
          "surface-board",
          "surface-logs",
          "surface-knowledge",
          "surface-mock",
        );
        element.classList.add(`surface-${surface}`);

        if (surface === "terminal") {
          body.innerHTML = `
            <div class="terminal-root"></div>
            <div class="terminal-overlay"></div>
          `;
          const terminalRoot = body.querySelector(".terminal-root");
          const overlay = body.querySelector(".terminal-overlay");
          const spinner = document.createElement("div");
          spinner.className = "overlay-spinner";
          spinner.textContent = "⣾";
          const message = document.createElement("div");
          message.className = "overlay-message";
          message.textContent = "Launching...";
          const copyButton = document.createElement("button");
          copyButton.type = "button";
          copyButton.className = "overlay-copy-button";
          copyButton.textContent = "Copy";
          copyButton.addEventListener("click", (event) => {
            event.preventDefault();
            event.stopPropagation();
            void copyTerminalOverlayMessage(windowData.id);
          });
          overlay.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            socketTransport.send({ kind: "focus_window", id: windowData.id });
          });
          overlay.appendChild(spinner);
          overlay.appendChild(message);
          overlay.appendChild(copyButton);
          updateTerminalOverlayCopyState(overlay);
          terminalRoot.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            socketTransport.send({ kind: "focus_window", id: windowData.id });
          });
          terminalRoot.addEventListener("click", () => {
            const runtime = terminalMap.get(windowData.id);
            runtime?.terminal.focus();
          });
          frontendUnits.terminalHost.createRuntime(windowData.id, terminalRoot);
          return;
        }

        if (surface === "file-tree") {
          body.innerHTML = `
            <div class="file-tree-root">
              <div class="file-tree-toolbar workspace-toolbar">
                <div class="file-tree-path">Repository</div>
                <button class="icon-button" data-action="refresh-tree" aria-label="Refresh tree">↻</button>
              </div>
              <div class="file-tree-scroll workspace-scroll">
                <div class="file-tree-list"></div>
              </div>
              <div class="file-tree-footer">.</div>
            </div>
          `;
          body.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            socketTransport.send({ kind: "focus_window", id: windowData.id });
          });
          body
            .querySelector("[data-action='refresh-tree']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
                windowData.id,
              );
              state.loaded.clear();
              state.expanded.clear();
              state.loading.clear();
              state.error = "";
              frontendUnits.branchesFileTreeSurface.requestFileTree(
                windowData.id,
                "",
              );
              frontendUnits.branchesFileTreeSurface.renderFileTree(windowData.id);
            });
          const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
            windowData.id,
          );
          if (!state.loaded.has("")) {
            frontendUnits.branchesFileTreeSurface.requestFileTree(windowData.id, "");
          }
          frontendUnits.branchesFileTreeSurface.renderFileTree(windowData.id);
          return;
        }

        if (surface === "branches") {
          body.innerHTML = `
            <div class="branch-list-root">
              <div class="branch-toolbar workspace-toolbar is-stacked">
                <div class="branch-toolbar-main workspace-toolbar-main">
                  <div class="branch-heading">Repository branches · double-click to launch</div>
                  <div class="branch-filter-group">
                    <button class="branch-filter-button" type="button" data-branch-filter="local">Local</button>
                    <button class="branch-filter-button" type="button" data-branch-filter="remote">Remote</button>
                    <button class="branch-filter-button" type="button" data-branch-filter="all">All</button>
                  </div>
                </div>
                <div class="branch-toolbar-actions workspace-toolbar-actions">
                  <button class="wizard-button branch-cleanup-trigger" type="button" data-action="open-branch-cleanup">Clean Up</button>
                  <button class="icon-button" data-action="refresh-branches" aria-label="Refresh branches">↻</button>
                </div>
              </div>
              <div class="branch-notice" hidden></div>
              <div class="branch-scroll workspace-scroll">
                <div class="branch-list"></div>
              </div>
            </div>
          `;
          body.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            socketTransport.send({ kind: "focus_window", id: windowData.id });
          });
          body
            .querySelector("[data-action='refresh-branches']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              const state = frontendUnits.branchesFileTreeSurface.ensureBranchListState(
                windowData.id,
              );
              state.error = "";
              state.notice = "";
              frontendUnits.branchesFileTreeSurface.requestBranches(windowData.id);
              frontendUnits.branchesFileTreeSurface.renderBranches(windowData.id);
            });
          for (const button of body.querySelectorAll("[data-branch-filter]")) {
            button.addEventListener("click", (event) => {
              event.stopPropagation();
              const state = frontendUnits.branchesFileTreeSurface.ensureBranchListState(
                windowData.id,
              );
              state.filter = button.dataset.branchFilter;
              frontendUnits.branchesFileTreeSurface.renderBranches(windowData.id);
            });
          }
          body
            .querySelector("[data-action='open-branch-cleanup']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              frontendUnits.branchesFileTreeSurface.openBranchCleanupModal(
                windowData.id,
              );
            });
          const state = frontendUnits.branchesFileTreeSurface.ensureBranchListState(
            windowData.id,
          );
          if (state.entries.length === 0 && !state.loading && !state.error) {
            frontendUnits.branchesFileTreeSurface.requestBranches(windowData.id);
          }
          frontendUnits.branchesFileTreeSurface.renderBranches(windowData.id);
          return;
        }

        if (surface === "profile") {
          body.innerHTML = `
            <div class="profile-root">
              <div class="workspace-toolbar is-stacked">
                <div class="workspace-toolbar-main">
                  <div class="knowledge-heading">Profiles</div>
                  <div class="profile-status"></div>
                </div>
                <div class="workspace-toolbar-actions">
                  <button class="wizard-button" type="button" data-action="new-profile">New profile</button>
                  <button class="icon-button" data-action="refresh-profile" aria-label="Refresh profiles">↻</button>
                </div>
              </div>
              <div class="profile-layout workspace-split">
                <div class="profile-list-pane">
                  <div class="profile-list"></div>
                </div>
                <div class="profile-editor-pane"></div>
              </div>
            </div>
          `;
          body.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            socketTransport.send({ kind: "focus_window", id: windowData.id });
          });
          body
            .querySelector("[data-action='refresh-profile']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              const state = frontendUnits.profileSurface.ensureProfileState(windowData.id);
              state.error = "";
              frontendUnits.profileSurface.requestProfile(windowData.id);
              frontendUnits.profileSurface.renderProfile(windowData.id, true);
            });
          body
            .querySelector("[data-action='new-profile']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              frontendUnits.profileSurface.createProfile(windowData.id);
            });
          const state = frontendUnits.profileSurface.ensureProfileState(windowData.id);
          if (!state.snapshot && !state.loading && !state.error) {
            frontendUnits.profileSurface.requestProfile(windowData.id);
          }
          frontendUnits.profileSurface.renderProfile(windowData.id);
          return;
        }

        if (surface === "board") {
          body.innerHTML = `
            <div class="board-root">
              <div class="workspace-toolbar is-stacked">
                <div class="workspace-toolbar-main">
                  <div class="knowledge-heading">Board chat</div>
                  <div class="board-status"></div>
                </div>
                <div class="workspace-toolbar-actions">
                  <button class="text-button board-for-you-filter" data-action="toggle-board-for-you" type="button" aria-pressed="false">For you</button>
                  <button class="icon-button" data-action="refresh-board" aria-label="Refresh board">↻</button>
                </div>
              </div>
              <div class="board-chat-shell">
                <div class="board-timeline-scroll board-scroll-surface workspace-scroll">
                  <div class="board-timeline"></div>
                </div>
                <div class="board-composer-bar">
                  <div class="board-composer-pane"></div>
                </div>
              </div>
            </div>
          `;
          body.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            socketTransport.send({ kind: "focus_window", id: windowData.id });
          });
          body
            .querySelector("[data-action='refresh-board']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              const state = frontendUnits.boardSurface.ensureBoardState(windowData.id);
              state.error = "";
              frontendUnits.boardSurface.requestBoard(windowData.id);
              frontendUnits.boardSurface.renderBoard(windowData.id);
            });
          body
            .querySelector("[data-action='toggle-board-for-you']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              const state = frontendUnits.boardSurface.ensureBoardState(windowData.id);
              state.audienceFilter = state.audienceFilter === "for_you" ? "all" : "for_you";
              if (state.audienceFilter === "for_you") {
                state.forYouUnread = 0;
              }
              frontendUnits.boardSurface.renderBoard(windowData.id);
            });
          const state = frontendUnits.boardSurface.ensureBoardState(windowData.id);
          if (state.entries.length === 0 && !state.loading && !state.error) {
            frontendUnits.boardSurface.requestBoard(windowData.id);
          }
          frontendUnits.boardSurface.renderBoard(windowData.id);
          return;
        }

        if (surface === "logs") {
          body.innerHTML = `
            <div class="logs-root">
              <div class="workspace-toolbar is-stacked">
                <div class="workspace-toolbar-main">
                  <div class="knowledge-heading">Structured logs</div>
                  <div class="logs-status"></div>
                </div>
                <div class="workspace-toolbar-actions">
                  <button class="text-button logs-unread-button" type="button" hidden>0 unread alerts</button>
                  <button class="icon-button" data-action="refresh-logs" aria-label="Refresh logs">↻</button>
                </div>
              </div>
              <div class="logs-filter-bar">
                <label class="logs-filter-field">
                  <span>Severity</span>
                  <select class="logs-severity-select">
                    <option value="debug">Debug+</option>
                    <option value="info">Info+</option>
                    <option value="warn">Warn+</option>
                    <option value="error">Error</option>
                  </select>
                </label>
                <label class="logs-filter-field">
                  <span>Search</span>
                  <input class="logs-search-input" type="search" placeholder="Filter message, source, or fields" />
                </label>
              </div>
              <div class="logs-layout workspace-split">
                <div class="logs-timeline"></div>
                <div class="logs-detail-pane"></div>
              </div>
            </div>
          `;
          body.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            socketTransport.send({ kind: "focus_window", id: windowData.id });
          });
          const state = frontendUnits.logsSurface.ensureLogState(windowData.id);
          body
            .querySelector("[data-action='refresh-logs']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              state.error = "";
              frontendUnits.logsSurface.requestLogs(windowData.id);
              frontendUnits.logsSurface.renderLogs(windowData.id);
            });
          body
            .querySelector(".logs-unread-button")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              frontendUnits.logsSurface.jumpToUnread(windowData.id);
            });
          body
            .querySelector(".logs-severity-select")
            .addEventListener("change", (event) => {
              state.severity = event.target.value;
              frontendUnits.logsSurface.renderLogs(windowData.id);
            });
          body
            .querySelector(".logs-search-input")
            .addEventListener("input", (event) => {
              state.query = event.target.value;
              frontendUnits.logsSurface.renderLogs(windowData.id);
            });
          if (state.entries.length === 0 && !state.loading && !state.error) {
            frontendUnits.logsSurface.requestLogs(windowData.id);
          }
          frontendUnits.logsSurface.renderLogs(windowData.id);
          return;
        }

        if (surface === "knowledge") {
          const knowledgeKind = knowledgeKindForPreset(windowData.preset);
          // SPEC-2017 — Knowledge Bridge surface is a 6-column Kanban Board:
          // Backlog / Draft / Planning / Implementation / Review / Done.
          // The columns are hard-coded so the source carries every
          // canonical data-phase literal (asserted by kanban-structure
          // tests) and so the renderer can simply locate columns via
          // .kanban-column[data-phase="..."]. The right-hand detail
          // pane survives Phase 1 unchanged; SPEC-2017 Phase 3 replaces
          // it with the SPEC-2356 Drawer pattern.
          body.innerHTML = `
            <div class="knowledge-root kanban-root">
              <div class="workspace-toolbar kanban-toolbar is-stacked">
                <div class="workspace-toolbar-main">
                  <div class="knowledge-heading">${knowledgeHeading(knowledgeKind)}</div>
                  ${
                    knowledgeKind === "issue"
                      ? `<div class="branch-filter-group">
                  <button class="branch-filter-button" type="button" data-knowledge-scope="open">Open</button>
                  <button class="branch-filter-button" type="button" data-knowledge-scope="closed">Closed</button>
                </div>`
                      : ""
                  }
                  <input class="knowledge-search" type="search" placeholder="${knowledgeSearchPlaceholder(knowledgeKind)}" />
                  <label class="kanban-hide-done-toggle" for="kanban-hide-done-${windowData.id}">
                    <input
                      type="checkbox"
                      id="kanban-hide-done-${windowData.id}"
                      class="kanban-hide-done"
                      data-action="kanban-hide-done"
                    />
                    <span>Hide done</span>
                  </label>
                </div>
                <div class="workspace-toolbar-actions">
                  <button class="icon-button" data-action="refresh-knowledge" aria-label="Refresh cached knowledge">↻</button>
                </div>
              </div>
              <div class="knowledge-status"></div>
              <div class="knowledge-split workspace-split kanban-shell">
                <div class="knowledge-list-pane kanban-list-pane">
                  <div class="kanban-board" role="list" aria-label="Knowledge Bridge Kanban Board">
                    <div class="kanban-column" data-phase="backlog" aria-label="Backlog column">
                      <div class="kanban-column-header">
                        <span class="kanban-column-name">Backlog</span>
                        <span class="kanban-column-count" data-role="count">0</span>
                      </div>
                      <div class="kanban-column-body" data-role="body"></div>
                    </div>
                    <div class="kanban-column" data-phase="draft" aria-label="Draft column">
                      <div class="kanban-column-header">
                        <span class="kanban-column-name">Draft</span>
                        <span class="kanban-column-count" data-role="count">0</span>
                      </div>
                      <div class="kanban-column-body" data-role="body"></div>
                    </div>
                    <div class="kanban-column" data-phase="planning" aria-label="Planning column">
                      <div class="kanban-column-header">
                        <span class="kanban-column-name">Planning</span>
                        <span class="kanban-column-count" data-role="count">0</span>
                      </div>
                      <div class="kanban-column-body" data-role="body"></div>
                    </div>
                    <div class="kanban-column" data-phase="implementation" aria-label="Implementation column">
                      <div class="kanban-column-header">
                        <span class="kanban-column-name">Implementation</span>
                        <span class="kanban-column-count" data-role="count">0</span>
                      </div>
                      <div class="kanban-column-body" data-role="body"></div>
                    </div>
                    <div class="kanban-column" data-phase="review" aria-label="Review column">
                      <div class="kanban-column-header">
                        <span class="kanban-column-name">Review</span>
                        <span class="kanban-column-count" data-role="count">0</span>
                      </div>
                      <div class="kanban-column-body" data-role="body"></div>
                    </div>
                    <div class="kanban-column" data-phase="done" aria-label="Done column">
                      <div class="kanban-column-header">
                        <span class="kanban-column-name">Done</span>
                        <span class="kanban-column-count" data-role="count">0</span>
                      </div>
                      <div class="kanban-column-body" data-role="body"></div>
                    </div>
                  </div>
                </div>
                <div class="knowledge-detail-pane"></div>
              </div>
            </div>
          `;
          body.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            socketTransport.send({ kind: "focus_window", id: windowData.id });
          });
          const state = frontendUnits.knowledgeSettingsSurface.ensureKnowledgeBridgeState(
            windowData.id,
            knowledgeKind,
          );
          const search = body.querySelector(".knowledge-search");
          search.value = state.query;
          search.addEventListener("input", () => {
            state.query = search.value;
            frontendUnits.knowledgeSettingsSurface.scheduleKnowledgeSearch(
              windowData.id,
              knowledgeKind,
            );
          });
          for (const button of body.querySelectorAll("[data-knowledge-scope]")) {
            button.addEventListener("click", (event) => {
              event.stopPropagation();
              frontendUnits.knowledgeSettingsSurface.switchKnowledgeListScope(
                windowData.id,
                button.dataset.knowledgeScope,
              );
            });
          }
          body
            .querySelector("[data-action='refresh-knowledge']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              frontendUnits.knowledgeSettingsSurface.requestKnowledgeBridge(
                windowData.id,
                knowledgeKind,
                true,
              );
              frontendUnits.knowledgeSettingsSurface.renderKnowledgeBridge(
                windowData.id,
              );
            });
          // SPEC-2017 — Hide done toggle persists via localStorage so
          // reloads honour the user preference. The hidden state hides
          // the Done column entirely (CSS-driven via data-hide-done on
          // the board) and updates state in place without reloading.
          const hideDoneToggle = body.querySelector("[data-action='kanban-hide-done']");
          if (hideDoneToggle) {
            hideDoneToggle.checked = state.hideDone === true;
            hideDoneToggle.addEventListener("change", (event) => {
              event.stopPropagation();
              state.hideDone = hideDoneToggle.checked === true;
              frontendUnits.knowledgeSettingsSurface.persistKanbanHideDone(
                state.hideDone,
              );
              frontendUnits.knowledgeSettingsSurface.renderKnowledgeBridge(
                windowData.id,
              );
            });
          }
          if (!state.detail && !state.loading) {
            frontendUnits.knowledgeSettingsSurface.requestKnowledgeBridge(
              windowData.id,
              knowledgeKind,
              false,
            );
          }
          frontendUnits.knowledgeSettingsSurface.renderKnowledgeBridge(
            windowData.id,
          );
          return;
        }

        if (windowData.preset === "settings") {
          frontendUnits.knowledgeSettingsSurface.renderSettingsWindow(
            body,
            windowData,
          );
          return;
        }

        const mock = mockContentForPreset(windowData.preset);
        body.innerHTML = `
          <div class="mock-root">
            <div class="mock-toolbar">
              <div class="mock-heading">${mock.heading}</div>
              <span class="mock-chip">${windowData.title}</span>
            </div>
            <div class="mock-scroll"></div>
          </div>
        `;
        const scroll = body.querySelector(".mock-scroll");
        body.addEventListener("mousedown", () => {
          focusWindowLocally(windowData.id);
          socketTransport.send({ kind: "focus_window", id: windowData.id });
        });
        for (const [label, value] of mock.rows) {
          const section = document.createElement("div");
          section.className = "mock-section";
          section.innerHTML = `
            <div class="mock-label">${label}</div>
            <div class="mock-row">
              <span>${value}</span>
              <span class="mock-chip">ready</span>
            </div>
          `;
          scroll.appendChild(section);
        }
      }

      // All DOM nodes are built via createElement + textContent to avoid
      // innerHTML with mixed trust. Secrets in env tables are redacted by
      // the backend before reaching this layer (see redact_secrets_in_agent).
      const customAgentsState = {
        agents: [],
        loading: false,
        statusMessage: "",
        statusKind: "",
      };
      // SPEC-1933 US-4: System tab state. `language` is the raw stored value
      // (auto/en/ja); the backend `system_settings` reply seeds it.
      const systemSettingsState = {
        language: "auto",
        loaded: false,
        statusMessage: "",
        statusKind: "",
      };
      const settingsWindowBodies = new Set();
      let pendingAddFromPreset = null;

      function createDiv(className) {
        const el = document.createElement("div");
        if (className) el.className = className;
        return el;
      }

      function purgeDetachedSettingsBodies() {
        for (const body of Array.from(settingsWindowBodies)) {
          if (!body.isConnected) settingsWindowBodies.delete(body);
        }
      }

      // SPEC-1933 Phase: System Settings (Output Language).
      // Build a tabbed Settings surface (System | Custom Agents) using
      // Operator Design tokens. Existing renderSettingsAgentList continues
      // to populate the Custom Agents panel via [data-role='settings-scroll'].
      function renderSettingsWindow(body, windowData) {
        // Sweep detached bodies up-front so repeated open/close cycles do
        // not accumulate references.
        purgeDetachedSettingsBodies();
        while (body.firstChild) body.removeChild(body.firstChild);

        const root = createDiv("settings-root");

        const toolbar = document.createElement("header");
        toolbar.className = "settings-toolbar";
        const heading = document.createElement("h2");
        heading.className = "settings-heading";
        heading.textContent = windowData.title || "Settings";

        const tabs = document.createElement("nav");
        tabs.className = "settings-tabs";
        tabs.setAttribute("role", "tablist");
        tabs.appendChild(buildSettingsTab("system", "System", true));
        tabs.appendChild(buildSettingsTab("custom-agents", "Custom Agents", false));

        toolbar.appendChild(heading);
        toolbar.appendChild(tabs);

        const bodyEl = createDiv("settings-body");

        const panelSystem = document.createElement("section");
        panelSystem.className = "settings-panel";
        panelSystem.setAttribute("role", "tabpanel");
        panelSystem.dataset.settingsPanel = "system";

        const panelAgents = document.createElement("section");
        panelAgents.className = "settings-panel hidden";
        panelAgents.setAttribute("role", "tabpanel");
        panelAgents.dataset.settingsPanel = "custom-agents";
        // Existing renderSettingsAgentList queries this attribute to inject
        // the Add button and agent rows.
        panelAgents.dataset.role = "settings-scroll";

        bodyEl.appendChild(panelSystem);
        bodyEl.appendChild(panelAgents);

        root.appendChild(toolbar);
        root.appendChild(bodyEl);
        body.appendChild(root);

        tabs.addEventListener("click", (e) => {
          const btn = e.target.closest("[data-settings-tab]");
          if (!btn) return;
          switchSettingsTab(body, btn.dataset.settingsTab);
        });
        tabs.addEventListener("keydown", (e) => {
          if (e.key !== "Enter" && e.key !== " ") return;
          const btn = e.target.closest("[data-settings-tab]");
          if (!btn) return;
          e.preventDefault();
          switchSettingsTab(body, btn.dataset.settingsTab);
        });

        body.addEventListener("mousedown", () => {
          focusWindowLocally(windowData.id);
          send({ kind: "focus_window", id: windowData.id });
        });
        settingsWindowBodies.add(body);

        renderSystemPanel(panelSystem);
        // Always request fresh system settings on open so the dropdown
        // reflects the on-disk config, even if the user changed it from a
        // different gwt instance.
        send({ kind: "get_system_settings" });

        renderSettingsAgentList();
        if (!customAgentsState.loading && customAgentsState.agents.length === 0) {
          customAgentsState.loading = true;
          send({ kind: "list_custom_agents" });
        }
      }

      function buildSettingsTab(id, label, selected) {
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = selected ? "settings-tab active" : "settings-tab";
        btn.setAttribute("role", "tab");
        btn.setAttribute("aria-selected", String(selected));
        btn.dataset.settingsTab = id;
        btn.textContent = label;
        return btn;
      }

      function switchSettingsTab(body, target) {
        const tabs = body.querySelectorAll(".settings-tab");
        tabs.forEach((tab) => {
          const isSelected = tab.dataset.settingsTab === target;
          tab.setAttribute("aria-selected", String(isSelected));
          tab.classList.toggle("active", isSelected);
        });
        const panels = body.querySelectorAll(".settings-panel");
        panels.forEach((panel) => {
          panel.classList.toggle(
            "hidden",
            panel.dataset.settingsPanel !== target,
          );
        });
      }

      function renderSystemPanel(panel) {
        while (panel.firstChild) panel.removeChild(panel.firstChild);

        const section = createDiv("settings-section");

        const label = document.createElement("label");
        label.className = "settings-label";
        label.setAttribute("for", "settings-system-language");
        label.textContent = "Output Language";
        section.appendChild(label);

        const select = document.createElement("select");
        select.className = "settings-select";
        select.id = "settings-system-language";
        for (const opt of [
          { value: "auto", text: "Auto (OS locale)" },
          { value: "en", text: "English" },
          { value: "ja", text: "日本語" },
        ]) {
          const option = document.createElement("option");
          option.value = opt.value;
          option.textContent = opt.text;
          select.appendChild(option);
        }
        select.value = systemSettingsState.language || "auto";
        select.addEventListener("change", (e) => {
          const next = e.target.value;
          systemSettingsState.language = next;
          systemSettingsState.statusMessage = "Saving…";
          systemSettingsState.statusKind = "info";
          renderSystemPanelStatus(panel);
          send({ kind: "update_system_settings", language: next });
        });
        section.appendChild(select);

        const help = document.createElement("p");
        help.className = "settings-help";
        help.textContent =
          "Used for narrative outputs (Workspace summaries and Board post bodies). " +
          "Settings UI text and gwtd subcommands stay English.";
        section.appendChild(help);

        const status = document.createElement("p");
        status.className = "settings-status";
        status.dataset.role = "system-settings-status";
        section.appendChild(status);

        panel.appendChild(section);
        renderSystemPanelStatus(panel);
      }

      function renderSystemPanelStatus(panel) {
        const status = panel.querySelector(
          "[data-role='system-settings-status']",
        );
        if (!status) return;
        status.textContent = systemSettingsState.statusMessage || "";
        if (systemSettingsState.statusKind) {
          status.dataset.kind = systemSettingsState.statusKind;
        } else {
          delete status.dataset.kind;
        }
      }

      function renderSystemPanelInAllSettingsWindows() {
        for (const body of Array.from(settingsWindowBodies)) {
          if (!body.isConnected) {
            settingsWindowBodies.delete(body);
            continue;
          }
          const panel = body.querySelector(
            "[data-settings-panel='system']",
          );
          if (panel) renderSystemPanel(panel);
        }
      }

      function renderSettingsAgentList() {
        for (const body of Array.from(settingsWindowBodies)) {
          if (!body.isConnected) {
            settingsWindowBodies.delete(body);
            continue;
          }
          const scroll = body.querySelector("[data-role='settings-scroll']");
          if (!scroll) continue;
          while (scroll.firstChild) scroll.removeChild(scroll.firstChild);

          const addBtn = document.createElement("button");
          addBtn.className = "wizard-button";
          addBtn.style.margin = "8px 0";
          addBtn.textContent = "＋ Add Claude Code (OpenAI-compat backend)";
          addBtn.addEventListener("click", (e) => {
            e.stopPropagation();
            startAddClaudeCodeOpenaiCompatFlow();
          });
          scroll.appendChild(addBtn);

          if (customAgentsState.statusMessage) {
            const section = createDiv("mock-section");
            const label = createDiv("mock-label");
            label.textContent = "Status";
            label.style.color =
              customAgentsState.statusKind === "error"
                ? "#ff6b6b"
                : customAgentsState.statusKind === "success"
                  ? "#7abf7a"
                  : "#999";
            section.appendChild(label);
            const row = createDiv("mock-row");
            const text = document.createElement("span");
            text.textContent = customAgentsState.statusMessage;
            row.appendChild(text);
            section.appendChild(row);
            scroll.appendChild(section);
          }

          if (customAgentsState.loading && customAgentsState.agents.length === 0) {
            const section = createDiv("mock-section");
            const row = createDiv("mock-row");
            const text = document.createElement("span");
            text.textContent = "Loading custom agents…";
            row.appendChild(text);
            section.appendChild(row);
            scroll.appendChild(section);
            continue;
          }

          if (customAgentsState.agents.length === 0) {
            const section = createDiv("mock-section");
            const row = createDiv("mock-row");
            const text = document.createElement("span");
            text.textContent = "No custom agents configured yet.";
            row.appendChild(text);
            section.appendChild(row);
            scroll.appendChild(section);
            continue;
          }

          for (const agent of customAgentsState.agents) {
            const section = createDiv("mock-section");
            const label = createDiv("mock-label");
            label.textContent = agent.display_name || agent.id;
            section.appendChild(label);
            const row = createDiv("mock-row");
            const text = document.createElement("span");
            const envCount = Object.keys(agent.env || {}).length;
            const baseUrl =
              agent.env && agent.env.ANTHROPIC_BASE_URL
                ? ` · ${agent.env.ANTHROPIC_BASE_URL}`
                : "";
            text.textContent = `${agent.id} · ${agent.command} · ${envCount} env entries${baseUrl}`;
            row.appendChild(text);
            const delBtn = document.createElement("button");
            delBtn.className = "icon-button";
            delBtn.setAttribute("aria-label", "Delete agent");
            delBtn.textContent = "×";
            delBtn.addEventListener("click", (e) => {
              e.stopPropagation();
              if (window.confirm(`Delete custom agent "${agent.id}"?`)) {
                send({ kind: "delete_custom_agent", agent_id: agent.id });
              }
            });
            row.appendChild(delBtn);
            section.appendChild(row);
            scroll.appendChild(section);
          }
        }
      }

      function startAddClaudeCodeOpenaiCompatFlow() {
        const baseUrl = window.prompt(
          "Upstream base_url (http:// or https://)\n\nExample: http://192.168.100.166:32768",
          "http://",
        );
        if (!baseUrl) return;
        const apiKey = window.prompt(
          "API key (forwarded as Bearer on /v1/models probe and ANTHROPIC_API_KEY at launch):",
        );
        if (!apiKey) return;
        setSettingsStatus("Probing /v1/models…", "info");
        pendingAddFromPreset = { baseUrl, apiKey };
        send({ kind: "test_backend_connection", base_url: baseUrl, api_key: apiKey });
      }

      function setSettingsStatus(message, kind) {
        customAgentsState.statusMessage = message;
        customAgentsState.statusKind = kind;
        renderSettingsAgentList();
      }

      function completeAddFromPreset(discoveredModels) {
        if (!pendingAddFromPreset) return;
        if (!discoveredModels || discoveredModels.length === 0) {
          setSettingsStatus("Upstream /v1/models returned no entries.", "error");
          pendingAddFromPreset = null;
          return;
        }
        const modelList = discoveredModels.join("\n");
        const model = window.prompt(
          `Discovered ${discoveredModels.length} model(s). Choose default_model (copy one ID):\n\n${modelList}`,
          discoveredModels[0],
        );
        if (!model) {
          pendingAddFromPreset = null;
          setSettingsStatus("Cancelled.", "info");
          return;
        }
        const id = window.prompt("Custom agent id (alphanumeric + `-`):", "claude-code-openai");
        if (!id) {
          pendingAddFromPreset = null;
          setSettingsStatus("Cancelled.", "info");
          return;
        }
        const displayName = window.prompt("Display name:", "Claude Code (OpenAI-compat)");
        if (!displayName) {
          pendingAddFromPreset = null;
          setSettingsStatus("Cancelled.", "info");
          return;
        }
        setSettingsStatus("Saving preset…", "info");
        send({
          kind: "add_custom_agent_from_preset",
          input: {
            id,
            display_name: displayName,
            base_url: pendingAddFromPreset.baseUrl,
            api_key: pendingAddFromPreset.apiKey,
            default_model: model,
          },
        });
        pendingAddFromPreset = null;
      }

      function ensureWindow(windowData) {
        let element = windowMap.get(windowData.id);
        if (!element) {
          element = document.createElement("div");
          element.className = "workspace-window";
          element.dataset.id = windowData.id;
          element.innerHTML = `
            <div class="titlebar">
              <div class="title">
                <span class="title-text"></span>
                <span class="window-role-badge"></span>
                <span class="status-chip running">
                  <span class="status-dot"></span>
                  <span class="status-label">Running</span>
                </span>
              </div>
              <div class="window-actions">
                <button class="icon-button" data-action="minimize" aria-label="Minimize window">▁</button>
                <button class="icon-button" data-action="maximize" aria-label="Maximize window">◻</button>
                <button class="icon-button" data-action="close" aria-label="Close window">×</button>
              </div>
            </div>
            <div class="window-tab-strip" aria-label="Window tabs"></div>
            <div class="window-body"></div>
            <div class="resize-handle"></div>
          `;
          stage.appendChild(element);
          windowMap.set(windowData.id, element);

          const titlebar = element.querySelector(".titlebar");
          const minimizeButton = element.querySelector("[data-action='minimize']");
          const maximizeButton = element.querySelector("[data-action='maximize']");
          const closeButton = element.querySelector("[data-action='close']");
          const resizeHandle = element.querySelector(".resize-handle");

          minimizeButton.addEventListener("click", (event) => {
            event.stopPropagation();
            toggleMinimizeWindow(windowData.id);
          });

          maximizeButton.addEventListener("click", (event) => {
            event.stopPropagation();
            toggleMaximizeWindow(windowData.id);
          });

          closeButton.addEventListener("click", (event) => {
            event.stopPropagation();
            send({ kind: "close_window", id: windowData.id });
          });

          titlebar.addEventListener("pointerdown", (event) => {
            if (event.target.closest("button")) {
              return;
            }
            const currentWindow = workspaceWindowById(windowData.id);
            focusWindowRemotely(windowData.id);
            dragState = {
              id: windowData.id,
              pointerId: event.pointerId,
              startX: event.clientX,
              startY: event.clientY,
              left: parseNumber(element.style.left),
              top: parseNumber(element.style.top),
              moved: false,
              allowMove: !currentWindow?.maximized,
              dockTargetId: null,
            };
            titlebar.setPointerCapture(event.pointerId);
          });
          titlebar.addEventListener("dragover", (event) => {
            if (!windowTabDragState || windowTabDragState.id === windowData.id) {
              return;
            }
            event.preventDefault();
            if (event.dataTransfer) {
              event.dataTransfer.dropEffect = "move";
            }
          });
          titlebar.addEventListener("drop", (event) => {
            if (!windowTabDragState || windowTabDragState.id === windowData.id) {
              return;
            }
            event.preventDefault();
            windowTabDragState.docked = true;
            send({
              kind: "dock_window_tab",
              id: windowTabDragState.id,
              target_id: windowData.id,
            });
          });

          resizeHandle.addEventListener("pointerdown", (event) => {
            focusWindowRemotely(windowData.id);
            resizeState = {
              id: windowData.id,
              pointerId: event.pointerId,
              startX: event.clientX,
              startY: event.clientY,
              width: parseNumber(element.style.width),
              height: parseNumber(element.style.height),
              fitFrame: null,
            };
            // SPEC-2356 Phase 9 (T-136): suppress hover-reveal peek strip
            // hits while resize is active so pointer movements that cross
            // the screen edge do not steal focus mid-resize.
            document.documentElement.dataset.opResizeActive = "true";
            resizeHandle.setPointerCapture(event.pointerId);
          });
          resizeHandle.addEventListener("lostpointercapture", (event) => {
            finishWindowResize(event.pointerId);
          });
        }

        if (!element.dataset.preset || element.dataset.preset !== windowData.preset) {
          element.dataset.preset = windowData.preset;
          mountWindowBody(windowData, element);
        }

        element.querySelector(".title-text").textContent = windowDisplayTitle(windowData);
        const titleText = element.querySelector(".title-text");
        titleText.title = windowTitleTooltip(windowData);
        element.querySelector(".window-role-badge").textContent = presetRoleLabel(windowData.preset);
        renderWindowTabs(windowData, element);
        if (windowData.agent_color) {
          element.dataset.agentColor = windowData.agent_color;
        } else {
          delete element.dataset.agentColor;
        }
        const wasMinimized = element.classList.contains("minimized");
        const previousWidth = parseFloat(element.style.width || "0");
        const previousHeight = parseFloat(element.style.height || "0");
        const dimensionsChanged =
          previousWidth !== windowData.geometry.width ||
          previousHeight !== windowData.geometry.height;
        const shouldPersistTerminalGeometry =
          (wasMinimized && !windowData.minimized) || dimensionsChanged;
        element.classList.toggle("minimized", Boolean(windowData.minimized));
        element.classList.toggle("maximized", Boolean(windowData.maximized));
        element.classList.toggle("tabbed", windowTabsFor(windowData).length > 1);
        element.style.left = `${windowData.geometry.x}px`;
        element.style.top = `${windowData.geometry.y}px`;
        element.style.width = `${windowData.geometry.width}px`;
        element.style.height = `${windowData.minimized ? 38 : windowData.geometry.height}px`;
        element.style.zIndex = String(windowData.z_index);
        element.querySelector(".resize-handle").hidden =
          Boolean(windowData.minimized) || Boolean(windowData.maximized);
        applyStatus(windowData.id, windowData.status, detailMap.get(windowData.id));
        if (presetSurface(windowData.preset) === "terminal" && !windowData.minimized) {
          requestAnimationFrame(() =>
            fitTerminal(windowData.id, shouldPersistTerminalGeometry),
          );
        }
      }

      function renderWorkspace(workspace) {
        viewport = {
          x: workspace.viewport.x,
          y: workspace.viewport.y,
          zoom: workspace.viewport.zoom,
        };
        applyViewport();

        const ids = new Set(workspace.windows.map((windowData) => windowData.id));
        for (const [windowId, element] of windowMap.entries()) {
          if (ids.has(windowId)) {
            continue;
          }
          const runtime = terminalMap.get(windowId);
          if (runtime && runtime.viewportRefreshFrame !== null) {
            cancelAnimationFrame(runtime.viewportRefreshFrame);
          }
          runtime?.cleanup?.();
          runtime?.terminal.dispose();
          terminalMap.delete(windowId);
          decoderMap.delete(windowId);
          detailMap.delete(windowId);
          windowRuntimeStateMap.delete(windowId);
          pendingOutputMap.delete(windowId);
          pendingSnapshotMap.delete(windowId);
          const profileState = profileStateMap.get(windowId);
          if (profileState) {
            clearProfileSaveTimer(profileState);
          }
          fileTreeStateMap.delete(windowId);
          branchListStateMap.delete(windowId);
          profileStateMap.delete(windowId);
          boardStateMap.delete(windowId);
          logStateMap.delete(windowId);
          clearKnowledgeBridgeState(windowId);
          if (branchCleanupWindowId === windowId) {
            branchCleanupWindowId = null;
            renderBranchCleanupModal();
          }
          element.remove();
          windowMap.delete(windowId);
        }

        for (const windowData of workspace.windows) {
          ensureWindow(windowData);
          const element = windowMap.get(windowData.id);
          if (element) {
            element.hidden = !visibleWindowData(windowData);
          }
        }

        requestAnimationFrame(syncMaximizedWindowsToViewport);

        const topmostId = topmostWindowId(workspace);
        if (topmostId && ids.has(topmostId)) {
          focusWindowLocally(topmostId);
          const runtime = terminalMap.get(topmostId);
          if (runtime) {
            runtime.terminal.focus();
          }
        } else {
          focusedId = null;
        }
      }

      const socketTransport = Object.freeze({
        send,
        setConnectionState,
        websocketUrl,
        handleOpen: handleSocketOpen,
        handleMessage: handleSocketMessage,
        handleClose: handleSocketClose,
        installEventHandlers: installSocketEventHandlers,
        connect: connectSocket,
      });

      const projectWorkspaceShell = Object.freeze({
        emptyWorkspace,
        activeProjectTab,
        activeWorkspace,
        workspaceWindowById,
        sendOpenProjectDialog,
        requestWindowList,
        renderWindowList,
        windowDisplayTitle,
        toggleWindowList,
        renderProjectTabs,
        renderRecentProjects,
        renderProjectPicker,
        renderProjectOnboarding,
        renderAppState,
      });

      const workspaceWindowManager = Object.freeze({
        focusWindowLocally,
        focusWindowRemotely,
        mountWindowBody,
        ensureWindow,
        renderWorkspace,
      });

      const terminalHost = Object.freeze({
        fit: fitTerminal,
        applyStatus,
        createRuntime: createTerminalRuntime,
        writeOutput,
        replaceTerminalSnapshot,
      });

      const launchWizardSurface = Object.freeze({
        openIssueLaunchWizard,
        sendAction: sendWizardAction,
        syncDraftState: syncWizardDraftState,
        flushBranchDraft: flushWizardBranchDraft,
        render: renderLaunchWizard,
      });

      const branchesFileTreeSurface = Object.freeze({
        ensureFileTreeState,
        ensureBranchListState,
        requestFileTree,
        requestBranches,
        renderFileTree,
        renderBranches,
        openBranchCleanupModal,
        closeBranchCleanupModal,
        renderBranchCleanupModal,
      });

      const profileSurface = Object.freeze({
        ensureProfileState,
        requestProfile,
        renderProfile,
        createProfile,
        setActiveProfile,
        flushProfileSave,
        deleteProfile,
        syncDraftFromSelection: syncProfileDraftFromSelection,
      });

      const boardSurface = Object.freeze({
        ensureBoardState,
        requestBoard,
        requestOlderBoardEntries,
        renderBoard,
        submitBoardEntry,
        handleRuntimeHookEvent: handleBoardHookEvent,
      });

      const logsSurface = Object.freeze({
        ensureLogState,
        requestLogs,
        renderLogs,
        appendLiveEntry: appendLiveLogEntry,
        jumpToUnread: jumpToUnreadLogs,
      });

      const knowledgeSettingsSurface = Object.freeze({
        ensureKnowledgeBridgeState,
        requestKnowledgeBridge,
        scheduleKnowledgeSearch,
        requestKnowledgeDetail,
        knowledgeEventScopeMatches,
        knowledgeDetailRequestMatches,
        switchKnowledgeListScope,
        renderKnowledgeBridge,
        renderSettingsWindow,
        renderSettingsAgentList,
        setSettingsStatus,
        completeAddFromPreset,
        persistKanbanHideDone: writeKanbanHideDonePreference,
      });

      const frontendUnits = Object.freeze({
        socketTransport,
        projectWorkspaceShell,
        workspaceWindowManager,
        terminalHost,
        launchWizardSurface,
        branchesFileTreeSurface,
        profileSurface,
        boardSurface,
        logsSurface,
        knowledgeSettingsSurface,
      });

      function receive(event) {
        switch (event.kind) {
          case "workspace_state":
            projectError = "";
            frontendUnits.projectWorkspaceShell.renderAppState(event.workspace);
            break;
          case "active_work_projection":
            activeWorkProjection = event.projection || null;
            renderActiveWorkOverview();
            renderWorkspaceOverview();
            recomputeOperatorTelemetry();
            break;
          case "window_list":
            windowListEntries = event.windows || [];
            frontendUnits.projectWorkspaceShell.renderWindowList();
            break;
          case "terminal_output":
            frontendUnits.terminalHost.writeOutput(event.id, event.data_base64);
            break;
          case "terminal_snapshot":
            frontendUnits.terminalHost.replaceTerminalSnapshot(event.id, event.data_base64);
            break;
          case "terminal_status":
            frontendUnits.terminalHost.applyStatus(
              event.id,
              event.status,
              event.detail,
            );
            break;
          case "window_state":
            frontendUnits.terminalHost.applyStatus(event.window_id, event.state);
            break;
          case "launch_progress": {
            const element = windowMap.get(event.id);
            if (element) {
              const messageEl = element.querySelector(".terminal-overlay .overlay-message");
              if (messageEl) {
                messageEl.textContent = event.message;
                updateTerminalOverlayCopyState(messageEl.closest(".terminal-overlay"));
              }
            }
            break;
          }
          case "project_index_status":
            setIndexStatus(event.project_root, event.status);
            break;
          case "file_tree_entries": {
            const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
              event.id,
            );
            state.loaded.set(event.path, event.entries);
            state.loading.delete(event.path);
            state.error = "";
            frontendUnits.branchesFileTreeSurface.renderFileTree(event.id);
            break;
          }
          case "file_tree_error": {
            const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
              event.id,
            );
            state.loading.delete(event.path);
            state.error = event.message;
            frontendUnits.branchesFileTreeSurface.renderFileTree(event.id);
            break;
          }
          case "branch_entries": {
            const state = frontendUnits.branchesFileTreeSurface.ensureBranchListState(
              event.id,
            );
            state.entries = event.entries;
            const phase = String(event.phase || "hydrated").toLowerCase();
            state.phase = phase;
            state.loading = phase !== "hydrated";
            state.receivedFreshEntries = true;
            state.error = "";
            state.notice = "";
            syncBranchSelectionState(state);
            frontendUnits.branchesFileTreeSurface.renderBranches(event.id);
            // SPEC-2356 — feed git layer count into the Operator Status Strip.
            try {
              const branchesCount = Array.isArray(event.entries) ? event.entries.length : 0;
              window.__operatorShell?.applyTelemetryCounts?.({
                branches: branchesCount,
                git: branchesCount,
              });
            } catch (e) {
              console.warn("operator branch telemetry failed", e);
            }
            break;
          }
          case "profile_snapshot": {
            const state = frontendUnits.profileSurface.ensureProfileState(event.id);
            state.snapshot = event.snapshot || null;
            state.loading = false;
            state.saving = Boolean(state.saveTimer);
            state.error = "";
            state.selectedProfile = event.snapshot?.selected_profile || null;
            frontendUnits.profileSurface.renderProfile(event.id);
            break;
          }
          case "board_entries": {
            const state = frontendUnits.boardSurface.ensureBoardState(event.id);
            const incomingEntries = event.entries || [];
            const existingEntryIds = new Set(state.entries.map((entry) => entry.id));
            const incomingEntryIds = new Set(incomingEntries.map((entry) => entry.id));
            const retainedHistory = state.entries.some(
              (entry) => Boolean(entry.id) && !incomingEntryIds.has(entry.id),
            );
            const addedEntry = incomingEntries.some(
              (entry) => Boolean(entry.id) && !existingEntryIds.has(entry.id),
            );
            const addressedEntry = incomingEntries.find(
              (entry) =>
                Boolean(entry.id) &&
                !existingEntryIds.has(entry.id) &&
                boardEntryMentionsSelf(entry),
            );
            const pendingSubmit = state.pendingSubmit;
            const completedSubmit = Boolean(pendingSubmit)
              && incomingEntries.some((entry) => {
                const parentId = entry.parent_id || null;
                return Boolean(entry.id)
                  && !pendingSubmit.existingEntryIds.has(entry.id)
                  && parentId === pendingSubmit.parentId
                  && String(entry.author_kind || "").toLowerCase() === "user"
                  && String(entry.body || "").trim() === pendingSubmit.body;
              });
            state.entries = mergeBoardEntries(state.entries, incomingEntries);
            state.hasMoreBefore = retainedHistory
              ? state.hasMoreBefore
              : Boolean(event.has_more_before);
            state.oldestEntryId = state.entries[0]?.id || null;
            if (
              state.replyParentId &&
              !state.entries.some((entry) => entry.id === state.replyParentId)
            ) {
              state.replyParentId = null;
            }
            if (completedSubmit) {
              if (state.composerBody.trim() === pendingSubmit.body) {
                state.composerBody = "";
              }
              state.replyParentId = null;
              state.pendingSubmit = null;
              state.submitting = false;
              state.pendingSelfPostScroll = true;
            } else if (addedEntry && !state.shouldFollowBoardBottom) {
              state.newEntriesAvailable = true;
            }
            if (
              addressedEntry &&
              addressedEntry.id !== state.lastNotifiedMentionEntryId
            ) {
              state.forYouUnread += 1;
              state.lastNotifiedMentionEntryId = addressedEntry.id;
              showBoardMentionNotification(addressedEntry, event.id);
            }
            state.loading = false;
            state.error = "";
            frontendUnits.boardSurface.renderBoard(event.id);
            break;
          }
          case "board_history_page": {
            const state = frontendUnits.boardSurface.ensureBoardState(event.id);
            const existingEntryIds = new Set(state.entries.map((entry) => entry.id));
            const olderEntries = (event.entries || []).filter(
              (entry) => !entry.id || !existingEntryIds.has(entry.id),
            );
            state.entries = olderEntries.concat(state.entries);
            state.hasMoreBefore = Boolean(event.has_more_before);
            state.oldestEntryId = state.entries[0]?.id || null;
            state.loadingOlder = false;
            state.preserveBoardScrollPosition = olderEntries.length > 0;
            state.error = "";
            frontendUnits.boardSurface.renderBoard(event.id);
            if (
              state.focusEntryId &&
              !state.entries.some((entry) => entry.id === state.focusEntryId) &&
              state.hasMoreBefore
            ) {
              frontendUnits.boardSurface.requestOlderBoardEntries(event.id);
            }
            break;
          }
          case "log_entries": {
            const state = frontendUnits.logsSurface.ensureLogState(event.id);
            state.entries = event.entries || [];
            state.loading = false;
            state.error = "";
            state.unreadAlerts = 0;
            state.unreadEntryId = null;
            if (!state.selectedEntryId || !state.entries.some((entry) => entry.id === state.selectedEntryId)) {
              state.selectedEntryId =
                state.entries.length > 0 ? state.entries[state.entries.length - 1].id : null;
            }
            frontendUnits.logsSurface.renderLogs(event.id);
            break;
          }
          case "log_entry_appended":
            frontendUnits.logsSurface.appendLiveEntry(event.entry);
            break;
          case "knowledge_entries": {
            const state = frontendUnits.knowledgeSettingsSurface.ensureKnowledgeBridgeState(
              event.id,
              event.knowledge_kind,
            );
            if (
              (event.request_id && event.request_id !== state.loadRequestId) ||
              !frontendUnits.knowledgeSettingsSurface.knowledgeEventScopeMatches(state, event)
            ) {
              break;
            }
            const queuedQuery = state.query.trim();
            const incomingEntries = event.entries || [];
            const keepSelectedNumber =
              state.selectedNumber &&
              incomingEntries.some((entry) => entry.number === state.selectedNumber);
            state.baseEntries = incomingEntries;
            state.baseEmptyMessage = event.empty_message || "";
            if (!queuedQuery) {
              state.entries = state.baseEntries.slice();
              state.emptyMessage = state.baseEmptyMessage;
              state.searching = false;
            }
            state.selectedNumber = keepSelectedNumber
              ? state.selectedNumber
              : event.selected_number ?? null;
            state.refreshEnabled = Boolean(event.refresh_enabled);
            state.loading = false;
            state.refreshing = false;
            state.error = "";
            if (queuedQuery) {
              frontendUnits.knowledgeSettingsSurface.scheduleKnowledgeSearch(
                event.id,
                event.knowledge_kind,
              );
              break;
            }
            frontendUnits.knowledgeSettingsSurface.renderKnowledgeBridge(event.id);
            break;
          }
          case "knowledge_search_results": {
            const state = frontendUnits.knowledgeSettingsSurface.ensureKnowledgeBridgeState(
              event.id,
              event.knowledge_kind,
            );
            const isInFlightResponse =
              event.request_id === state.inFlightSearchRequestId;
            if (isInFlightResponse) {
              state.searchInFlight = false;
              state.inFlightSearchRequestId = 0;
            }
            if (
              !frontendUnits.knowledgeSettingsSurface.knowledgeEventScopeMatches(state, event)
            ) {
              break;
            }
            if (
              event.request_id !== state.searchRequestId ||
              event.query !== state.query.trim()
            ) {
              const nextQuery = state.queuedSearchQuery || state.query.trim();
              state.queuedSearchQuery = "";
              if (isInFlightResponse && nextQuery) {
                frontendUnits.knowledgeSettingsSurface.scheduleKnowledgeSearch(
                  event.id,
                  event.knowledge_kind,
                );
              }
              break;
            }
            state.entries = event.entries || [];
            state.selectedNumber = event.selected_number ?? null;
            state.emptyMessage = event.empty_message || "";
            state.refreshEnabled = Boolean(event.refresh_enabled);
            state.error = "";
            const nextQuery = state.queuedSearchQuery;
            state.queuedSearchQuery = "";
            if (nextQuery && nextQuery !== event.query) {
              frontendUnits.knowledgeSettingsSurface.scheduleKnowledgeSearch(
                event.id,
                event.knowledge_kind,
              );
              break;
            }
            state.searching = false;
            if (state.selectedNumber) {
              state.detailLoading = true;
              frontendUnits.knowledgeSettingsSurface.requestKnowledgeDetail(
                event.id,
                event.knowledge_kind,
                state.selectedNumber,
              );
            } else {
              state.detail = null;
            }
            frontendUnits.knowledgeSettingsSurface.renderKnowledgeBridge(event.id);
            break;
          }
          case "knowledge_detail": {
            const state = frontendUnits.knowledgeSettingsSurface.ensureKnowledgeBridgeState(
              event.id,
              event.knowledge_kind,
            );
            if (
              !frontendUnits.knowledgeSettingsSurface.knowledgeDetailRequestMatches(state, event) ||
              !frontendUnits.knowledgeSettingsSurface.knowledgeEventScopeMatches(state, event)
            ) {
              break;
            }
            const matchesLoadRequest =
              !event.request_id || event.request_id === state.loadRequestId;
            state.detail = event.detail;
            state.selectedNumber = event.detail?.number ?? state.selectedNumber ?? null;
            if (matchesLoadRequest) {
              state.loading = false;
              state.refreshing = false;
            }
            state.detailLoading = false;
            frontendUnits.knowledgeSettingsSurface.renderKnowledgeBridge(event.id);
            // SPEC-2017 US-9 — refresh the Drawer body when the detail
            // is for the entry the Drawer is currently showing. This
            // also handles the swap-on-different-card case (T-034):
            // requestKnowledgeDetail was just dispatched for the new
            // number, so the new detail will arrive here and overwrite
            // the body without re-mounting the Drawer.
            const drawer = document.getElementById("kanban-drawer");
            if (
              drawer &&
              drawer.dataset.open === "true" &&
              kanbanDrawerActiveContext &&
              kanbanDrawerActiveContext.windowId === event.id
            ) {
              kanbanDrawerActiveContext = {
                ...kanbanDrawerActiveContext,
                number: event.detail?.number ?? kanbanDrawerActiveContext.number,
              };
              renderKanbanDrawerBody();
            }
            break;
          }
          case "branch_cleanup_result": {
            const state = frontendUnits.branchesFileTreeSurface.ensureBranchListState(
              event.id,
            );
            state.cleanupSelected.clear();
            state.cleanupModal.open = true;
            state.cleanupModal.stage = "result";
            state.cleanupModal.results = event.results || [];
            branchCleanupWindowId = event.id;
            if (event.id === WORKSPACE_CLEANUP_WINDOW_ID) {
              frontendUnits.branchesFileTreeSurface.renderBranchCleanupModal();
              renderWorkspaceOverview();
              break;
            }
            frontendUnits.branchesFileTreeSurface.renderBranches(event.id);
            break;
          }
          case "branch_error": {
            const state = frontendUnits.branchesFileTreeSurface.ensureBranchListState(
              event.id,
            );
            state.loading = false;
            if (state.cleanupModal.stage === "running") {
              failRunningBranchCleanup(event.id, event.message);
              if (event.id === WORKSPACE_CLEANUP_WINDOW_ID) {
                frontendUnits.branchesFileTreeSurface.renderBranchCleanupModal();
                break;
              }
              frontendUnits.branchesFileTreeSurface.renderBranches(event.id);
              break;
            }
            if (state.receivedFreshEntries) {
              state.notice = event.message;
              state.error = "";
            } else {
              state.error = event.message;
            }
            frontendUnits.branchesFileTreeSurface.renderBranches(event.id);
            break;
          }
          case "profile_error": {
            const state = frontendUnits.profileSurface.ensureProfileState(event.id);
            state.loading = false;
            state.saving = Boolean(state.saveTimer);
            state.error = event.message;
            frontendUnits.profileSurface.renderProfile(event.id, true);
            break;
          }
          case "board_error": {
            const state = frontendUnits.boardSurface.ensureBoardState(event.id);
            state.loading = false;
            state.loadingOlder = false;
            state.submitting = false;
            state.pendingSubmit = null;
            state.error = event.message;
            frontendUnits.boardSurface.renderBoard(event.id);
            break;
          }
          case "log_error": {
            const state = frontendUnits.logsSurface.ensureLogState(event.id);
            state.loading = false;
            state.error = event.message;
            frontendUnits.logsSurface.renderLogs(event.id);
            break;
          }
          case "knowledge_bridge_phase_updated": {
            // SPEC-2017 US-8 — phase write-back response. On Ok we
            // overwrite the optimistic card with fresh_entry and clear
            // the pending marker so the spinner stops; on Error we
            // rollback from dndSnapshot and surface a toast.
            const state = frontendUnits.knowledgeSettingsSurface.ensureKnowledgeBridgeState(
              event.id,
              knowledgeKindForPreset(workspaceWindowById(event.id)?.preset),
            );
            if (state.pendingPhaseUpdates) {
              state.pendingPhaseUpdates.delete(event.issue_number);
            }
            if (event.result?.kind === "ok") {
              const fresh = event.result.fresh_entry;
              if (fresh && Array.isArray(state.entries)) {
                const index = state.entries.findIndex(
                  (entry) => entry.number === fresh.number,
                );
                if (index >= 0) {
                  state.entries[index] = fresh;
                }
              }
              state.dndSnapshot = null;
            } else {
              const message =
                event.result?.message || "Failed to update phase. Reverting.";
              if (
                state.dndSnapshot &&
                state.dndSnapshot.issueNumber === event.issue_number &&
                Array.isArray(state.entries)
              ) {
                const index = state.entries.findIndex(
                  (entry) => entry.number === event.issue_number,
                );
                if (index >= 0 && state.dndSnapshot.entry) {
                  // Restore the card data captured at dragstart so the
                  // labels / phase / state mirror the pre-drop reality.
                  state.entries[index] = state.dndSnapshot.entry;
                }
                state.dndSnapshot = null;
              }
              state.error = message;
            }
            frontendUnits.knowledgeSettingsSurface.renderKnowledgeBridge(event.id);
            break;
          }
          case "knowledge_error": {
            const state = frontendUnits.knowledgeSettingsSurface.ensureKnowledgeBridgeState(
              event.id,
              event.knowledge_kind,
            );
            const isSearchError =
              typeof event.request_id === "number" && typeof event.query === "string";
            if (
              isSearchError &&
              (event.request_id !== state.inFlightSearchRequestId ||
                event.query !== state.query.trim() ||
                !frontendUnits.knowledgeSettingsSurface.knowledgeEventScopeMatches(state, event))
            ) {
              if (event.request_id === state.inFlightSearchRequestId) {
                state.searchInFlight = false;
                state.inFlightSearchRequestId = 0;
                const nextQuery = state.queuedSearchQuery || state.query.trim();
                state.queuedSearchQuery = "";
                if (nextQuery) {
                  frontendUnits.knowledgeSettingsSurface.scheduleKnowledgeSearch(
                    event.id,
                    event.knowledge_kind,
                  );
                }
              }
              break;
            }
            if (
              !isSearchError &&
              (!frontendUnits.knowledgeSettingsSurface.knowledgeDetailRequestMatches(state, event) ||
                !frontendUnits.knowledgeSettingsSurface.knowledgeEventScopeMatches(state, event))
            ) {
              break;
            }
            const matchesLoadRequest =
              !event.request_id || event.request_id === state.loadRequestId;
            if (matchesLoadRequest) {
              state.loading = false;
              state.refreshing = false;
            }
            state.searching = false;
            state.searchInFlight = false;
            state.inFlightSearchRequestId = 0;
            state.queuedSearchQuery = "";
            state.detailLoading = false;
            state.error = event.message;
            frontendUnits.knowledgeSettingsSurface.renderKnowledgeBridge(event.id);
            break;
          }
          case "project_open_error":
            projectError = event.message;
            frontendUnits.projectWorkspaceShell.renderProjectPicker();
            frontendUnits.projectWorkspaceShell.renderProjectOnboarding(
              frontendUnits.projectWorkspaceShell.activeProjectTab(),
            );
            break;
          case "launch_wizard_state":
            launchWizard = event.wizard;
            frontendUnits.launchWizardSurface.render();
            break;
          case "runtime_hook_event":
            frontendUnits.boardSurface.handleRuntimeHookEvent(event);
            break;
          case "update_state":
            if (event.state === "available") {
              updateCtaController.handleUpdateState(event);
            }
            break;
          case "update_apply_error":
            updateCtaController.showError(event.message || "Failed to start the update.");
            break;
          case "custom_agent_list":
            customAgentsState.agents = event.agents || [];
            customAgentsState.loading = false;
            frontendUnits.knowledgeSettingsSurface.renderSettingsAgentList();
            break;
          case "custom_agent_saved":
            if (event.agent) {
              const idx = customAgentsState.agents.findIndex(
                (a) => a.id === event.agent.id,
              );
              if (idx >= 0) {
                customAgentsState.agents[idx] = event.agent;
              } else {
                customAgentsState.agents.push(event.agent);
              }
            }
            customAgentsState.loading = false;
            setSettingsStatus(
              `Saved custom agent "${event.agent ? event.agent.id : "?"}".`,
              "success",
            );
            break;
          case "custom_agent_deleted":
            customAgentsState.agents = customAgentsState.agents.filter(
              (a) => a.id !== event.agent_id,
            );
            setSettingsStatus(`Deleted custom agent "${event.agent_id}".`, "success");
            break;
          case "system_settings":
            // SPEC-1933 US-4: backend echoed the on-disk language value.
            systemSettingsState.language = event.language || "auto";
            systemSettingsState.loaded = true;
            // Don't clobber an in-flight "Saving…" status; only seed when no
            // pending feedback is shown.
            if (!systemSettingsState.statusMessage || systemSettingsState.statusKind === "info") {
              systemSettingsState.statusMessage = "";
              systemSettingsState.statusKind = "";
            }
            renderSystemPanelInAllSettingsWindows();
            break;
          case "system_settings_updated":
            systemSettingsState.language = event.language || systemSettingsState.language;
            systemSettingsState.statusMessage = `Saved language: ${event.language}.`;
            systemSettingsState.statusKind = "success";
            renderSystemPanelInAllSettingsWindows();
            break;
          case "system_settings_error":
            systemSettingsState.statusMessage = event.message || "Failed to update system settings.";
            systemSettingsState.statusKind = "error";
            renderSystemPanelInAllSettingsWindows();
            break;
          case "backend_connection_result":
            frontendUnits.knowledgeSettingsSurface.setSettingsStatus(
              `/v1/models returned ${event.models.length} model(s).`,
              "success",
            );
            frontendUnits.knowledgeSettingsSurface.completeAddFromPreset(
              event.models,
            );
            break;
          case "custom_agent_preset_list":
            // Reserved for a future "Add from preset" picker — the current
            // UI hardcodes the one preset.
            break;
          case "custom_agent_error":
            customAgentsState.loading = false;
            pendingAddFromPreset = null;
            frontendUnits.knowledgeSettingsSurface.setSettingsStatus(
              `Error [${event.code}]: ${event.message}`,
              "error",
            );
            break;
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
        }
      }

      window.addEventListener("pointermove", (event) => {
        if (panState && panState.pointerId === event.pointerId) {
          viewport.x = panState.x + event.clientX - panState.startX;
          viewport.y = panState.y + event.clientY - panState.startY;
          applyViewport();
          return;
        }

        if (dragState && dragState.pointerId === event.pointerId) {
          const element = windowMap.get(dragState.id);
          if (!element) {
            return;
          }
          const deltaX = (event.clientX - dragState.startX) / viewport.zoom;
          const deltaY = (event.clientY - dragState.startY) / viewport.zoom;
          if (Math.abs(deltaX) > 2 || Math.abs(deltaY) > 2) {
            dragState.moved = true;
          }
          if (!dragState.allowMove) {
            return;
          }
          element.style.left = `${dragState.left + deltaX}px`;
          element.style.top = `${dragState.top + deltaY}px`;
          updateTitlebarDockPreview(event);
          return;
        }

        if (resizeState && resizeState.pointerId === event.pointerId) {
          const element = windowMap.get(resizeState.id);
          if (!element) {
            return;
          }
          element.style.width = `${clamp(
            resizeState.width + (event.clientX - resizeState.startX) / viewport.zoom,
            420,
          )}px`;
          element.style.height = `${clamp(
            resizeState.height + (event.clientY - resizeState.startY) / viewport.zoom,
            260,
          )}px`;
          scheduleTerminalResizeFit(resizeState.id);
        }
      });

      window.addEventListener("pointerup", (event) => {
        if (panState && panState.pointerId === event.pointerId) {
          canvas.classList.remove("panning");
          send({
            kind: "update_viewport",
            viewport,
          });
          panState = null;
        }

        if (dragState && dragState.pointerId === event.pointerId) {
          if (dragState.moved) {
            dragState.dockTargetId = dragState.allowMove
              ? titlebarDockTargetAt(event, dragState.id)
              : null;
            clearTitlebarDockPreview();
            if (dragState.dockTargetId) {
              send({
                kind: "dock_window_tab",
                id: dragState.id,
                target_id: dragState.dockTargetId,
              });
            } else {
              const runtime = terminalMap.get(dragState.id);
              sendGeometry(
                dragState.id,
                runtime?.terminal.cols || 80,
                runtime?.terminal.rows || 24,
              );
            }
          } else {
            clearTitlebarDockPreview();
            handleTitlebarClick(dragState.id);
          }
          dragState = null;
        }

        if (resizeState && resizeState.pointerId === event.pointerId) {
          finishWindowResize(event.pointerId);
        }
      });

      window.addEventListener("pointercancel", (event) => {
        if (dragState && dragState.pointerId === event.pointerId) {
          clearTitlebarDockPreview();
          dragState = null;
        }
        finishWindowResize(event.pointerId);
      });

      canvas.addEventListener("contextmenu", (event) => {
        event.preventDefault();
      });

      // Middle button pan: capture phase so it works over windows too.
      document.addEventListener(
        "pointerdown",
        (event) => {
          if (event.button !== 1) {
            return;
          }
          if (!canvas.contains(event.target)) {
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          panState = {
            pointerId: event.pointerId,
            startX: event.clientX,
            startY: event.clientY,
            x: viewport.x,
            y: viewport.y,
          };
          canvas.classList.add("panning");
          canvas.setPointerCapture(event.pointerId);
        },
        { capture: true },
      );

      // Left button pan: only on empty canvas area (not over windows).
      canvas.addEventListener("pointerdown", (event) => {
        if (event.button !== 0) {
          return;
        }
        if (event.target !== canvas && event.target !== stage) {
          return;
        }
        panState = {
          pointerId: event.pointerId,
          startX: event.clientX,
          startY: event.clientY,
          x: viewport.x,
          y: viewport.y,
        };
        canvas.classList.add("panning");
        canvas.setPointerCapture(event.pointerId);
      });

      function eventTargetElement(target) {
        if (target instanceof Element) {
          return target;
        }
        if (target && target.parentElement instanceof Element) {
          return target.parentElement;
        }
        return null;
      }

      function handleCanvasWheelEvent(event) {
        const targetElement = eventTargetElement(event.target);
        if (!targetElement || !canvas.contains(targetElement)) {
          return;
        }
        // SPEC-2008 FR-032: terminal-only opt-out. xterm.js owns wheel inside
        // `.surface-terminal`; every other workspace-window forwards plain
        // wheel to the DOM so panel scroll regions (Knowledge / Profile /
        // Logs / Board / Issue / SPEC / Settings ...) and modal
        // content scroll natively without registering a per-class whitelist.
        if (
          !event.ctrlKey &&
          !event.metaKey &&
          targetElement.closest(".surface-terminal")
        ) {
          return;
        }
        if (
          !event.ctrlKey &&
          !event.metaKey &&
          targetElement.closest(".workspace-window")
        ) {
          return;
        }
        if (event.ctrlKey || event.metaKey) {
          // Ctrl/Cmd + wheel = zoom
          event.preventDefault();
          event.stopPropagation();
          const rect = canvas.getBoundingClientRect();
          const anchorX = event.clientX - rect.left;
          const anchorY = event.clientY - rect.top;
          const factor = event.deltaY < 0 ? 1.1 : 0.9;
          zoomCanvasAt(anchorX, anchorY, viewport.zoom * factor);
        } else {
          // Plain wheel (trackpad two-finger scroll) = pan
          event.preventDefault();
          event.stopPropagation();
          viewport.x -= event.deltaX;
          viewport.y -= event.deltaY;
          applyViewport();
          persistViewport();
        }
      }

      function installCanvasWheelRouting() {
        // Capture phase so wheel events on child elements (windows, terminals)
        // are intercepted before they reach xterm.js or other consumers.
        document.addEventListener("wheel", handleCanvasWheelEvent, { capture: true, passive: false });
      }

      installCanvasWheelRouting();

      window.addEventListener(
        "keydown",
        (event) => {
          if (!shouldHandleFocusShortcut(event)) {
            return;
          }
          event.preventDefault();
          cycleFocus(event.key === "ArrowRight" ? "forward" : "backward");
        },
        true,
      );

      openProjectButton.addEventListener(
        "click",
        frontendUnits.projectWorkspaceShell.sendOpenProjectDialog,
      );
      pickerOpenProjectButton.addEventListener(
        "click",
        frontendUnits.projectWorkspaceShell.sendOpenProjectDialog,
      );
      onboardingOpenProjectButton.addEventListener(
        "click",
        frontendUnits.projectWorkspaceShell.sendOpenProjectDialog,
      );
      addButton.addEventListener("click", () => {
        if (addButton.disabled) {
          return;
        }
        openModal();
      });
      tileButton.addEventListener("click", () => arrangeWindows("tile"));
      stackButton.addEventListener("click", () => arrangeWindows("stack"));
      alignButton.addEventListener("click", () => arrangeWindows("align"));
      windowListButton.addEventListener(
        "click",
        frontendUnits.projectWorkspaceShell.toggleWindowList,
      );
      zoomOutButton.addEventListener("click", () => zoomCanvasByFactor(0.9));
      zoomResetButton.addEventListener("click", resetCanvasZoom);
      zoomInButton.addEventListener("click", () => zoomCanvasByFactor(1.1));
      closeModalButton.addEventListener("click", closeModal);
      modal.addEventListener("click", (event) => {
        if (event.target === modal) {
          closeModal();
        }
      });
      wizardCloseButton.addEventListener("click", () => {
        frontendUnits.launchWizardSurface.sendAction({ kind: "cancel" });
      });
      wizardCancelButton.addEventListener("click", () => {
        frontendUnits.launchWizardSurface.sendAction({ kind: "cancel" });
      });
      wizardSubmitButton.addEventListener("click", () => {
        frontendUnits.launchWizardSurface.flushBranchDraft();
        frontendUnits.launchWizardSurface.sendAction({ kind: "submit" });
      });
      wizardModal.addEventListener("click", (event) => {
        if (event.target === wizardModal) {
          frontendUnits.launchWizardSurface.sendAction({ kind: "cancel" });
        }
      });
      branchCleanupModal.addEventListener("click", (event) => {
        if (event.target === branchCleanupModal) {
          frontendUnits.branchesFileTreeSurface.closeBranchCleanupModal();
        }
      });
      // SPEC-2017 US-9 — wire the Kanban Drawer close affordances:
      // the explicit × button and the backdrop click both close the
      // modal. The Esc handler is registered globally a few blocks
      // below.
      const kanbanDrawerCloseButton = document.getElementById(
        "kanban-drawer-close",
      );
      if (kanbanDrawerCloseButton) {
        kanbanDrawerCloseButton.addEventListener("click", closeKanbanDrawer);
      }
      const kanbanDrawerBackdrop = document.getElementById(
        "kanban-drawer-backdrop",
      );
      if (kanbanDrawerBackdrop) {
        kanbanDrawerBackdrop.addEventListener("click", closeKanbanDrawer);
      }
      const workspaceOverviewCloseButton = document.getElementById(
        "workspace-overview-close",
      );
      if (workspaceOverviewCloseButton) {
        workspaceOverviewCloseButton.addEventListener("click", closeWorkspaceOverview);
      }
      const workspaceOverviewBackdrop = document.getElementById(
        "workspace-overview-drawer-backdrop",
      );
      if (workspaceOverviewBackdrop) {
        workspaceOverviewBackdrop.addEventListener("click", closeWorkspaceOverview);
      }
      if (workspaceOverviewEntry) {
        workspaceOverviewEntry.addEventListener("click", openWorkspaceOverview);
      }
      if (projectWorkspaceOverviewButton) {
        projectWorkspaceOverviewButton.addEventListener("click", openWorkspaceOverview);
      }
      // SPEC-2356 — keyboard equivalent for clicking the modal backdrop.
      // Without this, Esc only worked for the Hotkey overlay and Command
      // Palette; users were trapped in branch-cleanup / migration / wizard
      // with pointer escape only. The window list dropdown also gets Esc
      // close so the operator chrome offers consistent keyboard escape.
      document.addEventListener("keydown", (event) => {
        if (event.key !== "Escape") return;
        if (branchCleanupModal.classList.contains("open")) {
          // Reuse the same close path as backdrop click and explicit
          // Cancel button so all three pathways behave identically.
          frontendUnits.branchesFileTreeSurface.closeBranchCleanupModal();
          event.preventDefault();
          return;
        }
        if (wizardModal.classList.contains("open")) {
          // Wizard cancel is the explicit cancellation path; map Esc to
          // the same action so the modal isn't a keyboard trap.
          frontendUnits.launchWizardSurface.sendAction({ kind: "cancel" });
          event.preventDefault();
          return;
        }
        if (modal.classList.contains("open")) {
          // SPEC-2356 — preset (Add Window) modal also needs Esc-close.
          // closeModal() handles both the .open class flip and the focus
          // restore via the WeakMap-style closure variables.
          closeModal();
          event.preventDefault();
          return;
        }
        if (migrationModal && migrationModal.classList.contains("open")) {
          // Migration "skip" is the cancellation path; map Esc to the
          // same intent so the modal isn't a keyboard trap. Must use
          // tab_id (not id) to match the backend protocol.
          const tabId = migrationModalState.tabId;
          migrationModalState.open = false;
          migrationModalState.stage = "confirm";
          migrationModalState.message = "";
          migrationModalState.recovery = "";
          renderMigrationModal();
          if (tabId) {
            send({ kind: "skip_migration", tab_id: tabId });
          }
          event.preventDefault();
          return;
        }
        // SPEC-2017 US-9 — Esc dismisses the Kanban Drawer. Checked
        // before the windowList dropdown because the Drawer is a
        // modal surface and outranks the dropdown affordance.
        const kanbanDrawer = document.getElementById("kanban-drawer");
        if (kanbanDrawer && kanbanDrawer.dataset.open === "true") {
          closeKanbanDrawer();
          event.preventDefault();
          return;
        }
        const workspaceOverviewDrawer = document.getElementById("workspace-overview-drawer");
        if (
          workspaceOverviewDrawer &&
          workspaceOverviewDrawer.dataset.open === "true"
        ) {
          closeWorkspaceOverview();
          event.preventDefault();
          return;
        }
        if (windowListOpen) {
          // Close the Windows dropdown and return focus to its trigger
          // button (matches the modal pattern of restoring focus to the
          // element that opened the dropdown).
          windowListOpen = false;
          frontendUnits.projectWorkspaceShell.renderWindowList();
          if (windowListButton && typeof windowListButton.focus === "function") {
            try { windowListButton.focus({ preventScroll: true }); }
            catch { windowListButton.focus(); }
          }
          event.preventDefault();
        }
      });
      window.addEventListener("resize", () => {
        frontendUnits.projectWorkspaceShell.renderWindowList();
        syncMaximizedWindowsToViewport();
      });
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
        frontendUnits.projectWorkspaceShell.renderWindowList();
      });

      for (const button of modal.querySelectorAll("[data-preset]")) {
        button.addEventListener("click", () => {
          frontendUnits.socketTransport.send({
            kind: "create_window",
            preset: button.dataset.preset,
            bounds: visibleBounds(),
          });
          closeModal();
        });
      }

      // SPEC-2356 — bridge Command Palette + hotkey commands into existing
      // surface dispatch. Each command either focuses an existing window or
      // creates a new one through the same socket transport the preset
      // buttons use, so they share the legacy invariants.
      function focusOrSpawnPreset(preset) {
        const allWindows = activeWorkspace().windows || [];
        const existing = allWindows.find(
          (w) => w.preset === preset && !w.minimized,
        );
        if (existing) {
          frontendUnits.socketTransport.send({
            kind: "focus_window",
            id: existing.id,
          });
          return;
        }
        frontendUnits.socketTransport.send({
          kind: "create_window",
          preset,
          bounds: visibleBounds(),
        });
      }

      document.addEventListener("op:command", (event) => {
        const id = event.detail?.id;
        if (!id) return;
        switch (id) {
          case "open-board":
            focusOrSpawnPreset("board");
            return;
          case "open-git":
            focusOrSpawnPreset("branches");
            return;
          case "open-logs":
            focusOrSpawnPreset("logs");
            return;
          case "open-branches":
            focusOrSpawnPreset("branches");
            return;
          case "open-files":
            focusOrSpawnPreset("file_tree");
            return;
          case "spawn-shell":
            focusOrSpawnPreset("shell");
            return;
          case "start-work":
          case "spawn-agent":
            frontendUnits.socketTransport.send({
              kind: "open_start_work",
            });
            return;
          case "theme-cycle": {
            const tm = window.__operatorShell?.themeManager;
            if (!tm) return;
            const cycle = { auto: "dark", dark: "light", light: "auto" };
            tm.setTheme(cycle[tm.getPreference()] ?? "auto");
            return;
          }
          case "open-help": {
            const overlay = document.getElementById("op-hotkey-overlay");
            if (overlay) {
              overlay.dataset.open = overlay.dataset.open === "true" ? "" : "true";
              overlay.setAttribute(
                "aria-hidden",
                overlay.dataset.open === "true" ? "false" : "true",
              );
            }
            return;
          }
          default:
            console.debug("op:command unknown id", id);
        }
      });

      frontendUnits.projectWorkspaceShell.renderAppState(appState);
      renderActiveWorkOverview();
      frontendUnits.socketTransport.connect();
