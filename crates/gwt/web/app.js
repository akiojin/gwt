      import { Terminal } from "https://cdn.jsdelivr.net/npm/@xterm/xterm@6.0.0/+esm";
      import { FitAddon } from "https://cdn.jsdelivr.net/npm/@xterm/addon-fit@0.11.0/+esm";

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
      const windowListButton = document.getElementById("window-list-button");
      const windowListPanel = document.getElementById("window-list-panel");
      const zoomOutButton = document.getElementById("zoom-out-button");
      const zoomResetButton = document.getElementById("zoom-reset-button");
      const zoomInButton = document.getElementById("zoom-in-button");
      const modal = document.getElementById("preset-modal");
      const closeModalButton = document.getElementById("close-modal");
      const wizardModal = document.getElementById("wizard-modal");
      const wizardMeta = document.getElementById("wizard-meta");
      const wizardSummary = document.getElementById("wizard-summary");
      const wizardBody = document.getElementById("wizard-body");
      const wizardError = document.getElementById("wizard-error");
      const wizardCloseButton = document.getElementById("wizard-close-button");
      const wizardCancelButton = document.getElementById("wizard-cancel-button");
      const wizardSubmitButton = document.getElementById("wizard-submit-button");
      const branchCleanupModal = document.getElementById("branch-cleanup-modal");
      const branchCleanupDialog = branchCleanupModal.querySelector(".modal");
      const connectionDot = document.getElementById("connection-dot");
      const connectionLabel = document.getElementById("connection-label");
      const appVersionLabel = document.getElementById("app-version");

      const decoderMap = new Map();
      const pendingOutputMap = new Map();
      const pendingSnapshotMap = new Map();
      const detailMap = new Map();
      const terminalMap = new Map();
      const windowMap = new Map();
      const fileTreeStateMap = new Map();
      const branchListStateMap = new Map();
      const knowledgeBridgeStateMap = new Map();
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
            "wizardAdvancedOpen",
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

      // Diagnostic counter for intermittent key-input drops (bugfix/input-key).
      // Incremented on every `terminal.onData` firing so layer-by-layer counts
      // can be diffed against backend `gwt_input_trace` logs.
      let inputTraceSeq = 0;

      let socket = null;
      let reconnectTimer = null;
      let focusedId = null;
      let dragState = null;
      let panState = null;
      let resizeState = null;
      let viewport = { x: 0, y: 0, zoom: 1 };
      let viewportRasterTimer = null;
      let launchWizard = null;
      let wizardWasOpen = false;
      let wizardAdvancedOpen = false;
      let wizardBranchDraft = "";
      let wizardBranchBackendValue = "";
      let branchCleanupWindowId = null;
      let windowListOpen = false;
      let windowListEntries = [];
      let titlebarClickState = null;
      let appState = {
        app_version: "",
        tabs: [],
        active_tab_id: null,
        recent_projects: [],
      };
      let versionState = { current: "", latest: "" };
      let projectError = "";
      const BRANCH_CLEANUP_TIMEOUT_MS = 30000;
      const TERMINAL_SELECTION_DRAG_THRESHOLD = 4;

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

      function setConnectionState(connected) {
        connectionDot.classList.toggle("connected", connected);
        connectionLabel.textContent = connected ? "Connected" : "Reconnecting";
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

      function windowStateLabel(windowData) {
        if (windowData.minimized) {
          return "Minimized";
        }
        if (windowData.maximized) {
          return "Maximized";
        }
        return "Normal";
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
        const entries =
          windowListEntries.length > 0 ? windowListEntries : activeWorkspace().windows || [];
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
          row.innerHTML = `
            <div class="window-list-copy">
              <div class="window-list-title">${entry.title}</div>
              <div class="window-list-meta">${presetLabel(entry.preset)} · ${windowStateLabel(entry)}</div>
            </div>
            <span class="status-chip ${entry.status}">
              <span class="status-dot"></span>
              <span class="status-label">${windowStateLabel(entry)}</span>
            </span>
          `;
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
          if (tab.id === appState.active_tab_id) {
            button.classList.add("active");
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
          empty.className = "file-tree-empty";
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
        appState = nextState || {
          app_version: "",
          tabs: [],
          active_tab_id: null,
          recent_projects: [],
        };
        setVersionState(appState.app_version, versionState.latest);
        renderProjectTabs();
        renderProjectPicker();
        updateActionAvailability();
        const tab = activeProjectTab();
        renderProjectOnboarding(tab);
        renderWorkspace(tab?.workspace || emptyWorkspace());
        renderWindowList();
      }

      function openModal() {
        modal.classList.add("open");
      }

      function closeModal() {
        modal.classList.remove("open");
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

      function applyViewport() {
        stage.style.transform = `translate(${viewport.x}px, ${viewport.y}px) scale(${viewport.zoom})`;
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
        if (!workspace.windows || workspace.windows.length === 0) {
          return null;
        }
        return workspace.windows.reduce((topmost, candidate) => {
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
          fitTerminal(windowId, false);
        });
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

      function applyStatus(windowId, status, detail) {
        if (detail) {
          detailMap.set(windowId, detail);
        } else if (status === "running" || status === "ready") {
          detailMap.delete(windowId);
        }
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const chip = element.querySelector(".status-chip");
        const label = element.querySelector(".status-label");
        const overlay = element.querySelector(".terminal-overlay");
        chip.classList.remove("starting", "running", "ready", "exited", "error");
        chip.classList.add(status);
        label.textContent = status;
        const effectiveDetail = detailMap.get(windowId);
        if (overlay) {
          const messageEl = overlay.querySelector(".overlay-message");
          if (messageEl) {
            messageEl.textContent = effectiveDetail || "";
          } else {
            overlay.textContent = effectiveDetail || "";
          }
          overlay.classList.toggle(
            "visible",
            status === "starting" || status === "error" || status === "exited",
          );
          if (status === "starting") {
            startSpinnerAnimation(overlay);
          }
        }
      }

      function startSpinnerAnimation(overlay) {
        const spinner = overlay.querySelector(".overlay-spinner");
        if (!spinner) return;
        const chars = ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];
        let index = 0;
        const interval = setInterval(() => {
          spinner.textContent = chars[index % chars.length];
          index++;
        }, 100);
        const cleanup = () => {
          clearInterval(interval);
          overlay.removeEventListener("visibilitychange", cleanup);
        };
        const observer = new MutationObserver(() => {
          if (!overlay.classList.contains("visible")) {
            cleanup();
          }
        });
        observer.observe(overlay, { attributes: true });
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

      async function writeClipboardText(text, restoreFocus = null) {
        if (!text) {
          return false;
        }
        if (navigator.clipboard?.writeText) {
          try {
            await navigator.clipboard.writeText(text);
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

      function createTerminalRuntime(windowId, terminalContainer) {
        if (terminalMap.has(windowId)) {
          return terminalMap.get(windowId);
        }
        const terminal = new Terminal({
          cursorBlink: true,
          convertEol: true,
          theme: {
            background: "#020617",
            foreground: "#e2e8f0",
            cursor: "#f8fafc",
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
          },
          fontFamily:
            "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
          fontSize: 13,
          lineHeight: 1.2,
          scrollback: 5000,
        });
        const fitAddon = new FitAddon();
        terminal.loadAddon(fitAddon);
        terminal.open(terminalContainer);
        const cleanup = installTerminalCopyHandlers(windowId, terminalContainer, terminal);
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
          case "memo":
            return {
              heading: "Daily Notes",
              rows: [
                ["Workspace", "Floating windows"],
                ["Pinned", "2 notes"],
                ["Draft", "Canvas review"],
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
              timeoutId: null,
            },
          });
        }
        return branchListStateMap.get(windowId);
      }

      function ensureKnowledgeBridgeState(windowId, knowledgeKind) {
        if (!knowledgeBridgeStateMap.has(windowId)) {
          knowledgeBridgeStateMap.set(windowId, {
            kind: knowledgeKind,
            entries: [],
            selectedNumber: null,
            detail: null,
            query: "",
            loading: false,
            detailLoading: false,
            error: "",
            emptyMessage: "",
            refreshEnabled: true,
          });
        }
        const state = knowledgeBridgeStateMap.get(windowId);
        state.kind = knowledgeKind || state.kind;
        return state;
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

      function requestKnowledgeBridge(windowId, knowledgeKind, refresh = false) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        if (state.loading) {
          return;
        }
        state.loading = true;
        state.error = "";
        send({
          kind: "load_knowledge_bridge",
          id: windowId,
          knowledge_kind: knowledgeKind,
          selected_number: state.selectedNumber ?? null,
          refresh,
        });
      }

      function requestKnowledgeDetail(windowId, knowledgeKind, number) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        state.selectedNumber = number;
        state.detailLoading = true;
        send({
          kind: "select_knowledge_bridge_entry",
          id: windowId,
          knowledge_kind: knowledgeKind,
          number,
        });
      }

      function openIssueLaunchWizard(windowId, issueNumber) {
        send({
          kind: "open_issue_launch_wizard",
          id: windowId,
          issue_number: issueNumber,
        });
      }

      function sendWizardAction(action) {
        const payload = {
          kind: "launch_wizard_action",
          action,
        };
        if (action.kind === "submit") {
          payload.bounds = visibleBounds();
        }
        send(payload);
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
        if (selected) {
          button.classList.add("selected");
        }
        button.appendChild(createNode("span", "launch-choice-title", option.label));
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
          wizardAdvancedOpen = false;
          wizardBranchDraft = "";
          wizardBranchBackendValue = "";
          return;
        }

        if (!wizardWasOpen) {
          wizardWasOpen = true;
          wizardAdvancedOpen =
            launchWizard.selected_runtime_target === "docker" ||
            Boolean(launchWizard.selected_docker_service);
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

      function renderLaunchWizard() {
        if (!launchWizard) {
          wizardModal.classList.remove("open");
          wizardSummary.innerHTML = "";
          wizardBody.innerHTML = "";
          wizardError.hidden = true;
          wizardError.textContent = "";
          wizardSubmitButton.textContent = "Launch";
          wizardSubmitButton.disabled = false;
          syncWizardDraftState();
          return;
        }

        syncWizardDraftState();
        closeModal();
        wizardModal.classList.add("open");
        wizardMeta.textContent = `Selected branch · ${
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

        {
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

        {
          const section = createLaunchSection(
            "Advanced",
            "Runtime target, versions, and launch flags.",
          );
          const toggleRow = createNode("div", "launch-toggle-row");
          toggleRow.appendChild(
            createNode(
              "div",
              "launch-note",
              wizardAdvancedOpen
                ? "Docker, version, permissions, and tool-specific flags."
                : "Show runtime and launch flags.",
            ),
          );
          const toggleButton = createNode(
            "button",
            "wizard-button",
            wizardAdvancedOpen ? "Hide advanced" : "Show advanced",
          );
          toggleButton.type = "button";
          toggleButton.addEventListener("click", () => {
            wizardAdvancedOpen = !wizardAdvancedOpen;
            renderLaunchWizard();
          });
          toggleRow.appendChild(toggleButton);
          section.appendChild(toggleRow);

          if (wizardAdvancedOpen) {
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
          }

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
          errorRow.className = "file-tree-empty";
          errorRow.textContent = state.error;
          list.appendChild(errorRow);
        }

        if (!state.loaded.has("")) {
          const loadingRow = document.createElement("div");
          loadingRow.className = "file-tree-empty";
          loadingRow.textContent = "Loading repository";
          list.appendChild(loadingRow);
          return;
        }

        function appendRows(parentPath, depth) {
          const entries = state.loaded.get(parentPath) || [];
          for (const entry of entries) {
            const row = document.createElement("div");
            row.className = "file-tree-row";
            if (state.selectedPath === entry.path) {
              row.classList.add("selected");
            }
            row.style.paddingLeft = `${12 + depth * 18}px`;

            const expanded = state.expanded.has(entry.path);
            const isDirectory = entry.kind === "directory";
            row.innerHTML = `
              <span class="tree-caret">${isDirectory ? (expanded ? "▾" : "▸") : ""}</span>
              <span class="tree-icon ${isDirectory ? "dir" : "file"}">${isDirectory ? "▣" : "•"}</span>
              <span class="tree-name">${entry.name}</span>
            `;
            row.addEventListener("click", () => {
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
            });
            list.appendChild(row);

            if (isDirectory && state.expanded.has(entry.path)) {
              if (state.loaded.has(entry.path)) {
                appendRows(entry.path, depth + 1);
              } else {
                const loadingRow = document.createElement("div");
                loadingRow.className = "file-tree-empty";
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
          emptyRow.className = "file-tree-empty";
          emptyRow.textContent = "No visible files";
          list.appendChild(emptyRow);
        }
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

        list.innerHTML = "";

        if (state.error) {
          const errorRow = document.createElement("div");
          errorRow.className = "branch-empty";
          errorRow.textContent = state.error;
          list.appendChild(errorRow);
          renderBranchCleanupModal();
          return;
        }

        if (state.loading && state.entries.length === 0) {
          const loadingRow = document.createElement("div");
          loadingRow.className = "branch-empty";
          loadingRow.textContent = "Loading branches";
          list.appendChild(loadingRow);
          renderBranchCleanupModal();
          return;
        }

        const visibleEntries = filteredBranchEntries(state);
        if (visibleEntries.length === 0) {
          const emptyRow = document.createElement("div");
          emptyRow.className = "branch-empty";
          emptyRow.textContent = state.entries.length === 0 ? "No branches" : "No branches in this filter";
          list.appendChild(emptyRow);
          renderBranchCleanupModal();
          return;
        }

        for (const entry of visibleEntries) {
          const row = document.createElement("div");
          row.className = "branch-row";
          if (state.selectedBranchName === entry.name) {
            row.classList.add("selected");
          }
          if (state.cleanupSelected.has(entry.name)) {
            row.classList.add("cleanup-selected");
          }

          const toggle = document.createElement("button");
          toggle.type = "button";
          toggle.className = `branch-cleanup-toggle ${cleanupToggleClass(entry, state)}`;
          toggle.textContent = cleanupToggleSymbol(entry, state);
          toggle.title = cleanupToggleTitle(entry, state);
          toggle.addEventListener("click", (event) => {
            event.stopPropagation();
            toggleBranchCleanupSelection(windowId, entry.name);
          });
          row.appendChild(toggle);

          const main = document.createElement("div");
          main.className = "branch-main";
          const name = document.createElement("div");
          name.className = "branch-name";
          const nameText = document.createElement("span");
          nameText.className = "branch-name-text";
          nameText.textContent = entry.name;
          name.appendChild(nameText);
          if (entry.is_head) {
            const head = document.createElement("span");
            head.className = "branch-head";
            head.textContent = "HEAD";
            name.appendChild(head);
          }
          main.appendChild(name);

          const upstream = document.createElement("div");
          upstream.className = "branch-upstream";
          upstream.textContent = entry.upstream || "No upstream";
          main.appendChild(upstream);

          const date = document.createElement("div");
          date.className = "branch-date";
          date.textContent = entry.last_commit_date || "No commit date";
          main.appendChild(date);

          const cleanupDetail = cleanupDetailText(entry, state);
          if (cleanupDetail) {
            const detail = document.createElement("div");
            detail.className = `branch-cleanup-detail ${
              cleanupAvailabilityForRender(entry, state) === "blocked" ? "blocked" : ""
            }`.trim();
            detail.textContent = cleanupDetail;
            main.appendChild(detail);
          }
          row.appendChild(main);

          const meta = document.createElement("div");
          meta.className = "branch-meta";
          const scope = document.createElement("span");
          scope.className = "branch-scope";
          scope.textContent = entry.scope;
          meta.appendChild(scope);

          const cleanupBadge = document.createElement("span");
          cleanupBadge.className = `branch-cleanup-badge ${cleanupAvailabilityForRender(entry, state)}`;
          cleanupBadge.textContent = cleanupBadgeText(entry, state);
          meta.appendChild(cleanupBadge);

          const summary = document.createElement("span");
          summary.className = "branch-summary";
          summary.textContent =
            entry.ahead || entry.behind ? `↑${entry.ahead} ↓${entry.behind}` : "synced";
          meta.appendChild(summary);
          row.appendChild(meta);

          row.addEventListener("click", () => {
            state.selectedBranchName = entry.name;
            state.notice = "";
            renderBranches(windowId);
          });
          row.addEventListener("dblclick", () => {
            state.selectedBranchName = entry.name;
            state.notice = "";
            renderBranches(windowId);
            send({
              kind: "open_launch_wizard",
              id: windowId,
              branch_name: entry.name,
            });
          });
          list.appendChild(row);
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

      function knowledgeSearchPlaceholder(kind) {
        switch (kind) {
          case "issue":
            return "Search cached issues";
          case "spec":
            return "Search cached SPECs";
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

      function renderKnowledgeBridge(windowId) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const state = ensureKnowledgeBridgeState(
          windowId,
          knowledgeKindForPreset(workspaceWindowById(windowId)?.preset),
        );
        const list = element.querySelector(".knowledge-list");
        const detailPane = element.querySelector(".knowledge-detail-pane");
        const status = element.querySelector(".knowledge-status");
        const refreshButton = element.querySelector("[data-action='refresh-knowledge']");
        const searchInput = element.querySelector(".knowledge-search");
        if (!list || !detailPane || !status || !refreshButton || !searchInput) {
          return;
        }

        refreshButton.disabled = !state.refreshEnabled || state.loading;
        searchInput.placeholder = knowledgeSearchPlaceholder(state.kind);

        status.className = "knowledge-status";
        status.textContent = "";
        if (state.error) {
          status.classList.add("visible", "error");
          status.textContent = state.error;
        } else if (state.loading && state.entries.length === 0) {
          status.classList.add("visible", "info");
          status.textContent = "Loading cache-backed data";
        } else if (state.emptyMessage && state.entries.length === 0) {
          status.classList.add("visible", "info");
          status.textContent = state.emptyMessage;
        }

        list.innerHTML = "";
        const visibleEntries = filteredKnowledgeEntries(state);
        if (visibleEntries.length === 0) {
          const empty = createNode("div", "knowledge-empty");
          if (state.entries.length === 0) {
            empty.textContent = state.emptyMessage || "No cached items";
          } else {
            empty.textContent = "No matching cached items";
          }
          list.appendChild(empty);
        } else {
          for (const entry of visibleEntries) {
            const row = createNode("button", "knowledge-row");
            row.type = "button";
            if (state.selectedNumber === entry.number) {
              row.classList.add("selected");
            }
            const main = createNode("div", "knowledge-row-main");
            const titleWrap = createNode("div", "");
            titleWrap.appendChild(
              createNode("div", "knowledge-row-number", `#${entry.number}`),
            );
            titleWrap.appendChild(
              createNode("div", "knowledge-row-title", entry.title),
            );
            main.appendChild(titleWrap);
            const stateChip = createNode(
              "span",
              `knowledge-state-chip ${entry.state}`,
              entry.state,
            );
            main.appendChild(stateChip);
            row.appendChild(main);

            const meta = createNode("div", "knowledge-row-meta");
            meta.appendChild(createNode("span", "knowledge-meta-copy", entry.meta));
            if ((entry.linked_branch_count || 0) > 0) {
              meta.appendChild(
                createNode(
                  "span",
                  "knowledge-chip",
                  `${entry.linked_branch_count} linked branch${entry.linked_branch_count === 1 ? "" : "es"}`,
                ),
              );
            }
            for (const label of entry.labels || []) {
              meta.appendChild(createNode("span", "knowledge-chip", label));
            }
            row.appendChild(meta);
            row.addEventListener("click", () => {
              if (state.selectedNumber === entry.number && !state.detailLoading) {
                return;
              }
              requestKnowledgeDetail(windowId, state.kind, entry.number);
              renderKnowledgeBridge(windowId);
            });
            list.appendChild(row);
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

        const scroll = createNode("div", "knowledge-detail-scroll");
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

      function clearBranchCleanupTimeout(state) {
        if (state.cleanupModal.timeoutId !== null) {
          clearTimeout(state.cleanupModal.timeoutId);
          state.cleanupModal.timeoutId = null;
        }
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
        clearBranchCleanupTimeout(state);
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
        clearBranchCleanupTimeout(state);
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
        clearBranchCleanupTimeout(state);
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
        clearBranchCleanupTimeout(state);
        state.cleanupModal.timeoutId = window.setTimeout(() => {
          if (
            failRunningBranchCleanup(
              windowId,
              "Branch cleanup timed out. Check the branch list and try again.",
            )
          ) {
            renderBranches(windowId);
          }
        }, BRANCH_CLEANUP_TIMEOUT_MS);
        renderBranchCleanupModal();
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
        if (!windowId) {
          branchCleanupModal.classList.remove("open");
          branchCleanupDialog.innerHTML = "";
          return;
        }
        const state = ensureBranchListState(windowId);
        if (!state.cleanupModal.open) {
          branchCleanupModal.classList.remove("open");
          branchCleanupDialog.innerHTML = "";
          return;
        }

        const selectedEntries = selectedBranchCleanupEntries(windowId);
        const supportsRemoteDelete = selectedEntries.some((entry) => Boolean(entry.cleanup.upstream));
        branchCleanupModal.classList.add("open");
        branchCleanupDialog.innerHTML = "";

        if (state.cleanupModal.stage === "running") {
          const title = createNode("h2", "", "Cleaning up branches");
          const copy = createNode(
            "div",
            "branch-cleanup-running",
            `Running cleanup for ${Math.max(selectedEntries.length, 1)} branch${Math.max(
              selectedEntries.length,
              1,
            ) === 1 ? "" : "es"}.`,
          );
          branchCleanupDialog.appendChild(title);
          branchCleanupDialog.appendChild(copy);
          return;
        }

        if (state.cleanupModal.stage === "result") {
          const title = createNode("h2", "", "Cleanup result");
          branchCleanupDialog.appendChild(title);
          branchCleanupDialog.appendChild(
            createNode(
              "div",
              "branch-cleanup-results-summary",
              branchCleanupResultSummary(state.cleanupModal.results),
            ),
          );
          const resultList = createNode("div", "branch-cleanup-list");
          for (const result of state.cleanupModal.results || []) {
            const item = createNode("div", "branch-cleanup-item");
            const header = createNode("div", "branch-cleanup-item-header");
            header.appendChild(
              createNode("div", "branch-cleanup-item-title", result.branch),
            );
            const status = createNode(
              "span",
              `branch-cleanup-badge ${result.status === "partial" ? "risky" : result.status === "failed" ? "blocked" : "safe"}`,
              result.status,
            );
            header.appendChild(status);
            item.appendChild(header);
            item.appendChild(
              createNode("div", "branch-cleanup-item-copy", result.message),
            );
            if (result.execution_branch && result.execution_branch !== result.branch) {
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
          branchCleanupDialog.appendChild(resultList);
          const footer = createNode("div", "modal-footer");
          const close = createNode("button", "wizard-button primary", "Close");
          close.type = "button";
          close.addEventListener("click", () => closeBranchCleanupModal(windowId));
          footer.appendChild(close);
          branchCleanupDialog.appendChild(footer);
          return;
        }

        branchCleanupDialog.appendChild(createNode("h2", "", "Clean up branches"));
        branchCleanupDialog.appendChild(
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
            createNode("span", `branch-cleanup-badge ${entry.cleanup.availability}`, entry.cleanup.availability),
          );
          item.appendChild(header);
          const target = cleanupMergeTargetText(entry.cleanup.merge_target) || "not merged";
          item.appendChild(createNode("div", "branch-cleanup-item-copy", target));
          if (entry.cleanup.execution_branch && entry.cleanup.execution_branch !== entry.name) {
            item.appendChild(
              createNode(
                "div",
                "branch-cleanup-item-copy",
                `Executed as ${entry.cleanup.execution_branch}`,
              ),
            );
          }
          const risks = cleanupRiskLabels(entry.cleanup.risks);
          if (risks.length > 0) {
            item.appendChild(
              createNode("div", "branch-cleanup-item-copy", risks.join(", ")),
            );
          }
          list.appendChild(item);
        }
        branchCleanupDialog.appendChild(list);
        if (supportsRemoteDelete) {
          const toggleRow = createNode("label", "branch-cleanup-toggle-row");
          const checkbox = document.createElement("input");
          checkbox.type = "checkbox";
          checkbox.checked = Boolean(state.cleanupModal.deleteRemote);
          checkbox.addEventListener("change", () => {
            state.cleanupModal.deleteRemote = checkbox.checked;
          });
          toggleRow.appendChild(checkbox);
          toggleRow.appendChild(
            createNode("span", "", "Also delete matching remote branches"),
          );
          branchCleanupDialog.appendChild(toggleRow);
        }
        const footer = createNode("div", "modal-footer");
        const cancel = createNode("button", "wizard-button", "Cancel");
        cancel.type = "button";
        cancel.addEventListener("click", () => closeBranchCleanupModal(windowId));
        footer.appendChild(cancel);
        const submit = createNode("button", "wizard-button primary", "Run cleanup");
        submit.type = "button";
        submit.addEventListener("click", () => runBranchCleanup(windowId));
        footer.appendChild(submit);
        branchCleanupDialog.appendChild(footer);
      }

      function mountWindowBody(windowData, element) {
        const body = element.querySelector(".window-body");
        body.innerHTML = "";
        const surface = presetSurface(windowData.preset);
        element.classList.remove(
          "surface-terminal",
          "surface-file-tree",
          "surface-branches",
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
          overlay.appendChild(spinner);
          overlay.appendChild(message);
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
              <div class="file-tree-toolbar">
                <div class="file-tree-path">Repository</div>
                <button class="icon-button" data-action="refresh-tree" aria-label="Refresh tree">↻</button>
              </div>
              <div class="file-tree-scroll">
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
              <div class="branch-toolbar">
                <div class="branch-toolbar-main">
                  <div class="branch-heading">Repository branches · double-click to launch</div>
                  <div class="branch-filter-group">
                    <button class="branch-filter-button" type="button" data-branch-filter="local">Local</button>
                    <button class="branch-filter-button" type="button" data-branch-filter="remote">Remote</button>
                    <button class="branch-filter-button" type="button" data-branch-filter="all">All</button>
                  </div>
                </div>
                <div class="branch-toolbar-actions">
                  <button class="wizard-button branch-cleanup-trigger" type="button" data-action="open-branch-cleanup">Clean Up</button>
                  <button class="icon-button" data-action="refresh-branches" aria-label="Refresh branches">↻</button>
                </div>
              </div>
              <div class="branch-notice" hidden></div>
              <div class="branch-scroll">
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

        if (surface === "knowledge") {
          const knowledgeKind = knowledgeKindForPreset(windowData.preset);
          body.innerHTML = `
            <div class="knowledge-root">
              <div class="knowledge-toolbar">
                <div class="knowledge-toolbar-main">
                  <div class="knowledge-heading">${knowledgeHeading(knowledgeKind)}</div>
                  <input class="knowledge-search" type="search" placeholder="${knowledgeSearchPlaceholder(knowledgeKind)}" />
                </div>
                <div class="knowledge-toolbar-actions">
                  <button class="icon-button" data-action="refresh-knowledge" aria-label="Refresh cached knowledge">↻</button>
                </div>
              </div>
              <div class="knowledge-status"></div>
              <div class="knowledge-split">
                <div class="knowledge-list-pane">
                  <div class="knowledge-list"></div>
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
            frontendUnits.knowledgeSettingsSurface.renderKnowledgeBridge(
              windowData.id,
            );
          });
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

      function renderSettingsWindow(body, windowData) {
        // Sweep detached bodies up-front so repeated open/close cycles do
        // not accumulate references.
        purgeDetachedSettingsBodies();
        while (body.firstChild) body.removeChild(body.firstChild);
        const root = createDiv("mock-root");
        const toolbar = createDiv("mock-toolbar");
        const heading = createDiv("mock-heading");
        heading.textContent = "Custom Agents";
        const chip = document.createElement("span");
        chip.className = "mock-chip";
        chip.textContent = windowData.title || "Settings";
        toolbar.appendChild(heading);
        toolbar.appendChild(chip);
        const scroll = createDiv("mock-scroll");
        scroll.dataset.role = "settings-scroll";
        root.appendChild(toolbar);
        root.appendChild(scroll);
        body.appendChild(root);

        body.addEventListener("mousedown", () => {
          focusWindowLocally(windowData.id);
          send({ kind: "focus_window", id: windowData.id });
        });
        settingsWindowBodies.add(body);
        renderSettingsAgentList();
        if (!customAgentsState.loading && customAgentsState.agents.length === 0) {
          customAgentsState.loading = true;
          send({ kind: "list_custom_agents" });
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
                <span class="status-chip starting">
                  <span class="status-dot"></span>
                  <span class="status-label">starting</span>
                </span>
              </div>
              <div class="window-actions">
                <button class="icon-button" data-action="minimize" aria-label="Minimize window">▁</button>
                <button class="icon-button" data-action="maximize" aria-label="Maximize window">◻</button>
                <button class="icon-button" data-action="close" aria-label="Close window">×</button>
              </div>
            </div>
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
            };
            titlebar.setPointerCapture(event.pointerId);
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
            };
            resizeHandle.setPointerCapture(event.pointerId);
          });
        }

        if (!element.dataset.preset || element.dataset.preset !== windowData.preset) {
          element.dataset.preset = windowData.preset;
          mountWindowBody(windowData, element);
        }

        element.querySelector(".title-text").textContent = windowData.title;
        const wasMinimized = element.classList.contains("minimized");
        const shouldPersistTerminalGeometry = wasMinimized && !windowData.minimized;
        element.classList.toggle("minimized", Boolean(windowData.minimized));
        element.classList.toggle("maximized", Boolean(windowData.maximized));
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
          pendingOutputMap.delete(windowId);
          pendingSnapshotMap.delete(windowId);
          fileTreeStateMap.delete(windowId);
          branchListStateMap.delete(windowId);
          knowledgeBridgeStateMap.delete(windowId);
          if (branchCleanupWindowId === windowId) {
            branchCleanupWindowId = null;
            renderBranchCleanupModal();
          }
          element.remove();
          windowMap.delete(windowId);
        }

        for (const windowData of workspace.windows) {
          ensureWindow(windowData);
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

      const knowledgeSettingsSurface = Object.freeze({
        ensureKnowledgeBridgeState,
        requestKnowledgeBridge,
        requestKnowledgeDetail,
        renderKnowledgeBridge,
        renderSettingsWindow,
        renderSettingsAgentList,
        setSettingsStatus,
        completeAddFromPreset,
      });

      const frontendUnits = Object.freeze({
        socketTransport,
        projectWorkspaceShell,
        workspaceWindowManager,
        terminalHost,
        launchWizardSurface,
        branchesFileTreeSurface,
        knowledgeSettingsSurface,
      });

      function receive(event) {
        switch (event.kind) {
          case "workspace_state":
            projectError = "";
            frontendUnits.projectWorkspaceShell.renderAppState(event.workspace);
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
          case "launch_progress": {
            const element = windowMap.get(event.id);
            if (element) {
              const messageEl = element.querySelector(".terminal-overlay .overlay-message");
              if (messageEl) {
                messageEl.textContent = event.message;
              }
            }
            break;
          }
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
            break;
          }
          case "knowledge_entries": {
            const state = frontendUnits.knowledgeSettingsSurface.ensureKnowledgeBridgeState(
              event.id,
              event.knowledge_kind,
            );
            state.entries = event.entries || [];
            state.selectedNumber = event.selected_number ?? null;
            state.emptyMessage = event.empty_message || "";
            state.refreshEnabled = Boolean(event.refresh_enabled);
            state.loading = false;
            state.error = "";
            frontendUnits.knowledgeSettingsSurface.renderKnowledgeBridge(event.id);
            break;
          }
          case "knowledge_detail": {
            const state = frontendUnits.knowledgeSettingsSurface.ensureKnowledgeBridgeState(
              event.id,
              event.knowledge_kind,
            );
            state.detail = event.detail;
            state.selectedNumber = event.detail?.number ?? state.selectedNumber ?? null;
            state.loading = false;
            state.detailLoading = false;
            frontendUnits.knowledgeSettingsSurface.renderKnowledgeBridge(event.id);
            break;
          }
          case "branch_cleanup_result": {
            const state = frontendUnits.branchesFileTreeSurface.ensureBranchListState(
              event.id,
            );
            clearBranchCleanupTimeout(state);
            state.cleanupSelected.clear();
            state.cleanupModal.open = true;
            state.cleanupModal.stage = "result";
            state.cleanupModal.results = event.results || [];
            branchCleanupWindowId = event.id;
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
          case "knowledge_error": {
            const state = frontendUnits.knowledgeSettingsSurface.ensureKnowledgeBridgeState(
              event.id,
              event.knowledge_kind,
            );
            state.loading = false;
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
          case "update_state":
            if (event.state === "available") {
              setVersionState(event.current, event.latest);
              showUpdateToast(event.latest);
            }
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
        }
      }

      let updateToastTimer = null;

      function showUpdateToast(version) {
        let toast = document.getElementById("update-toast");
        if (!toast) {
          toast = document.createElement("div");
          toast.id = "update-toast";
          toast.className = "update-toast";
          document.body.appendChild(toast);
        }
        toast.textContent = `\u{1F4E6} Update available: v${version} \u2014 click to apply`;
        toast.style.opacity = "1";
        toast.onclick = () => {
          if (window.confirm(`Apply update to v${version} now?\n\ngwt will restart automatically.`)) {
            send({ kind: "apply_update" });
          }
        };
        clearTimeout(updateToastTimer);
        updateToastTimer = setTimeout(() => {
          toast.style.opacity = "0";
          setTimeout(() => toast.remove(), 300);
          updateToastTimer = null;
        }, 8000);
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
          fitTerminal(resizeState.id, false);
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
            const runtime = terminalMap.get(dragState.id);
            sendGeometry(
              dragState.id,
              runtime?.terminal.cols || 80,
              runtime?.terminal.rows || 24,
            );
          } else {
            handleTitlebarClick(dragState.id);
          }
          dragState = null;
        }

        if (resizeState && resizeState.pointerId === event.pointerId) {
          const runtime = terminalMap.get(resizeState.id);
          fitTerminal(resizeState.id, false);
          sendGeometry(
            resizeState.id,
            runtime?.terminal.cols || 80,
            runtime?.terminal.rows || 24,
          );
          resizeState = null;
        }
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

      function findNativeWheelScrollSurface(target) {
        const element = eventTargetElement(target);
        if (!element) {
          return null;
        }
        return element.closest(".branch-scroll, .file-tree-scroll");
      }

      function handleCanvasWheelEvent(event) {
        const targetElement = eventTargetElement(event.target);
        if (!targetElement || !canvas.contains(targetElement)) {
          return;
        }
        // Let terminal windows handle their own scroll (xterm.js scrollback)
        if (
          !event.ctrlKey &&
          !event.metaKey &&
          targetElement.closest(".surface-terminal")
        ) {
          return;
        }
        const nativeWheelScrollSurface = findNativeWheelScrollSurface(event.target);
        if (!event.ctrlKey && !event.metaKey && nativeWheelScrollSurface) {
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

      frontendUnits.projectWorkspaceShell.renderAppState(appState);
      frontendUnits.socketTransport.connect();
