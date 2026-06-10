      import { Terminal } from "/assets/xterm/xterm.mjs";
      import { FitAddon } from "/assets/xterm/addon-fit.mjs";
      import { renderBranchCleanupModal as renderBranchCleanupModalView } from "/branch-cleanup-modal.js";
      import { renderMigrationModal as renderMigrationModalView } from "/migration-modal.js";
      import { renderProjectCloneModal as renderProjectCloneModalView } from "/project-clone-modal.js";
      import { initOperatorShell, applyTelemetryCounts, applyProviderUsage } from "/operator-shell.js";
      import { createFocusTrap } from "/focus-trap.js";
      import {
        TITLEBAR_DOCK_HIT_HEIGHT,
        clientPointFromDragEvent,
        detachGeometryFromClientPoint,
        findTitlebarDockTarget,
        resolveDragReleasePoint,
      } from "/window-docking.js";
      import {
        applyBoardMentionNotificationFocus,
        boardEntryAudienceLabels,
        boardEntryMentionsSelf,
        boardEntryOriginActionLabel,
        boardEntryOriginLabel,
        boardEntryOriginSessionId,
        boardEntryPreview,
        findBoardEntry,
        groupBoardLanes,
        GENERAL_LANE_KEY,
        mentionsForBoardSubmit,
        visibleBoardEntries,
      } from "/board-surface.js";
      import { createWorkspaceKanbanSurface as createWorkspaceOverviewSurface } from "/workspace-kanban-surface.js";
      import { createWorkspaceResumePickerController } from "/workspace-resume-picker-modal.js";
      import { createUpdateCtaController } from "/update-cta.js";
      import { createReleaseNotesWindow } from "/release-notes-window.js";
      import { createConsoleWindow } from "/console-window.js";
      import { createTerminalContextMenuController } from "/terminal-context-menu.js";
      import { classifyTerminalCopyKeyEvent } from "/terminal-copy-shortcut.js";
      import { createTerminalWheelScrollController } from "/terminal-wheel-scroll.js";
      import {
        renderProjectTabs as renderProjectTabsView,
        updateProjectTabDot as updateProjectTabDotView,
      } from "/project-tabs-renderer.js";
      import { renderCloseProjectTabConfirmModal } from "/close-project-tab-confirm-modal.js";
      import { renderIndexSettingsPanel } from "/index-settings-panel.js";
      import { renderCustomAgentEnvEditor } from "/custom-agent-env-editor.js";
      import {
        buildChoiceOrSelectField,
        buildReasoningField,
        buildToggleField,
      } from "/launch-controls.js";
      import {
        applyVisibilityTransition,
        attachContainerResizeReflow,
        attachHostResizeReflow,
        classifyProjectWindowVisibility,
        createTerminalFitScheduler,
        createTerminalViewportRefreshScheduler,
        elementHasLayoutBox,
        gateTerminalInputForReadiness,
        rearmRefreshOnVisible,
        runTerminalActivationSequence,
        viewportEligibleForRefresh,
      } from "/terminal-viewport-reflow.js";
      import {
        beginLocalGeometryEdit,
        clearLocalGeometryEdit,
        commitLocalGeometryEdit,
        createGeometrySyncState,
        localGeometryBaseRevision,
        maximizedGeometry,
        resizeGeometryFromPointerState,
        shouldApplyWorkspaceGeometry,
        syncResizeStatePointerEvent,
        workspaceGeometryRevision,
      } from "/window-geometry-sync.js";
      import { createSocketReceiveDispatcher } from "/socket-receive-dispatcher.js";
      import { createTerminalOutputBatcher } from "/terminal-output-buffer.js";
      import { createInteractionGuard } from "/interaction-guard.js";
      import { createCanvasWheelGestureClassifier } from "/canvas-wheel-gesture.js";
      import { createViewportPersistThrottle } from "/viewport-persist-throttle.js";
      import { createViewportSyncState } from "/viewport-sync.js";
      import { shouldSkipTerminalFocusActivation } from "/clone-modal-focus-guard.js";
      import { createUiTraceProfiler } from "/ui-trace-profiler.js";
      import { UI_TRACE_EVENT, createUiTraceWiring } from "/ui-trace-wiring.js";
      // SPEC-3015 — window runtime state normalization extracted from app.js;
      // backed by the generated protocol enum contract (/protocol-enums.js).
      import {
        mapAgentTelemetryState,
        normalizeWindowRuntimeState,
        presetSupportsWaitingStatus,
        windowRuntimeLabel,
      } from "/window-runtime-state.js";

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
        applyProviderUsage: (snapshot) => applyProviderUsage(document, snapshot),
      };

      const uiTraceProfiler = createUiTraceProfiler();

      const canvas = document.getElementById("canvas");
      const stage = document.getElementById("canvas-stage");
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
      const addButton = document.getElementById("add-button");
      const tileButton = document.getElementById("tile-button");
      const stackButton = document.getElementById("stack-button");
      const alignButton = document.getElementById("align-button");
      const windowListButton = document.getElementById("window-list-button");
      const windowListPanel = document.getElementById("window-list-panel");
      const worldGrid = document.getElementById("canvas-world-grid");
      const workspaceOverviewEntry = document.getElementById("op-workspace-overview-entry");
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
      const wizardBackButton = document.getElementById("wizard-back-button");
      const wizardCancelButton = document.getElementById("wizard-cancel-button");
      const wizardSubmitButton = document.getElementById("wizard-submit-button");
      const cloneProjectModal = document.getElementById("clone-project-modal");
      const cloneProjectDialog = cloneProjectModal
        ? cloneProjectModal.querySelector(".modal-shell")
        : null;
      const branchCleanupModal = document.getElementById("branch-cleanup-modal");
      const branchCleanupDialog = branchCleanupModal.querySelector(".modal-shell");
      const migrationModal = document.getElementById("migration-modal");
      const migrationDialog = migrationModal
        ? migrationModal.querySelector(".modal-shell")
        : null;
      const connectionDot = document.getElementById("connection-dot");
      const connectionLabel = document.getElementById("connection-label");
      const appVersionLabel = document.getElementById("app-version");

      const decoderMap = new Map();
      const pendingOutputMap = new Map();
      const pendingSnapshotMap = new Map();
      const detailMap = new Map();
      const windowRuntimeStateMap = new Map();
      const terminalMap = new Map();
      let terminalFitScheduler = null;
      let terminalViewportRefreshScheduler = null;
      const terminalOutputBatcher = createTerminalOutputBatcher({
        mergeChunks: (chunks, windowId) => {
          const decoder = decoderMap.get(windowId);
          if (!decoder) {
            return "";
          }
          return chunks
            .map((chunk) => decoder.decode(decodeBase64(chunk), { stream: true }))
            .join("");
        },
        write: (windowId, text, onWritten) => {
          const runtime = terminalMap.get(windowId);
          if (!runtime) {
            return;
          }
          runtime.terminal.write(text, onWritten);
        },
        canWrite: canRefreshTerminalViewport,
        onFlush: (windowId) => {
          scheduleTerminalViewportRefresh(windowId);
        },
      });
      const windowMap = new Map();
      const renderedWindowElementKeys = new Map();
      const renderedRuntimeStatusKeys = new Map();
      const fileTreeStateMap = new Map();
      const branchListStateMap = new Map();
      const profileStateMap = new Map();
      const boardStateMap = new Map();
      const logStateMap = new Map();
      const knowledgeBridgeStateMap = new Map();
      const indexSearchStateMap = new Map();
      const pendingIndexOpenTargetsByPreset = new Map();
      const KNOWLEDGE_AUTO_REFRESH_INTERVAL_MS = 60000;
      let nextKnowledgeLoadRequestId = 1;
      let nextKnowledgeSearchRequestId = 1;
      let startupAutoResumeReadySent = false;
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
            "launchWizardOpenError",
            "wizardWasOpen",
            "wizardBranchDraft",
            "wizardBranchBackendValue",
          ]),
          mutatedBy: Object.freeze([
            "openIssueLaunchWizard",
            "closeLaunchWizardLocal",
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
      // Issue #2694 Phase C — per-connection dispatcher so queued messages
      // from a closed socket cannot flush into the next reconnect session
      // (replaying stale terminal_output / workspace_state). `generation`
      // identifies the active WebSocket cycle and is incremented on every
      // open/close transition; the dispatcher's receive() callback gates on
      // it so any inbound work scheduled before the swap is silently
      // dropped after the swap completes.
      let socketReceiveDispatcher = null;
      let socketReceiveDispatcherGeneration = 0;
      let reconnectTimer = null;
      let focusedId = null;
      let dragState = null;
      let windowTabDragState = null;
      let panState = null;
      let resizeState = null;
      const geometrySyncState = createGeometrySyncState();
      let viewport = { x: 0, y: 0, zoom: 1 };
      const viewportSyncState = createViewportSyncState({
        initialViewport: viewport,
      });
      const canvasWheelGestureClassifier = createCanvasWheelGestureClassifier({
        idleMs: 300,
      });
      let viewportRasterTimer = null;
      let viewportDomApplied = false;
      let launchWizard = null;
      let launchWizardOpenError = null;
      let launchWizardOpening = null;
      let activeWorkProjection = null;
      let pendingBoardEntryFocusId = null;
      let wizardWasOpen = false;
      let wizardBranchDraft = "";
      let wizardBranchBackendValue = "";
      let launchWizardPendingAction = null;
      // Issue #2698 PR 1 (B7) — defer destructive wizard re-renders
      // while the user has a native <select> dropdown open. The OS
      // dropdown overlay is anchored to the original DOM node; if
      // renderLaunchWizard() swaps `wizardBody` mid-interaction, the
      // user's selection commit lands on a destroyed element and is
      // silently lost. `wizardInteractionGuard` coalesces inbound
      // `launch_wizard_state` / `launch_wizard_open_error` messages
      // while active and replays the latest pending event on release.
      const wizardInteractionGuard = createInteractionGuard({
        onFlush: (deferred) => {
          if (!deferred || typeof deferred !== "object") {
            return;
          }
          if (deferred.kind === "launch_wizard_state") {
            clearLaunchWizardPendingAction();
            clearLaunchWizardOpening();
            if (deferred.wizard) {
              launchWizardOpenError = null;
            }
            launchWizard = deferred.wizard;
          } else if (deferred.kind === "launch_wizard_open_error") {
            clearLaunchWizardPendingAction();
            clearLaunchWizardOpening();
            launchWizard = null;
            launchWizardOpenError = {
              title: deferred.title || "Launch Agent",
              message: deferred.message || "Unable to open Launch Wizard",
            };
          }
          frontendUnits.launchWizardSurface.render();
        },
      });
      // Issue #2698 PR 4 — same guard applied to the System Settings
      // Output Language `<select>`. Backend echoes `system_settings`
      // and `system_settings_updated` events; if either arrives while
      // the user has the dropdown open, `renderSystemPanel()` does a
      // `while (panel.firstChild) panel.removeChild(panel.firstChild)`
      // pass that destroys the live `<select>` and breaks the user's
      // commit. Delegated listeners scope to `select.settings-select`
      // so the guard covers every Settings window without per-window
      // wiring.
      function applyAutostartStatus(event, statusMessage = "") {
        systemSettingsState.autostartEnabled = event.enabled === true;
        systemSettingsState.autostartPreviousEnabled =
          systemSettingsState.autostartEnabled;
        systemSettingsState.autostartMechanism = event.mechanism || "";
        systemSettingsState.autostartInstallPath = event.install_path || "";
        systemSettingsState.autostartLoaded = true;
        systemSettingsState.autostartPending = false;
        systemSettingsState.statusMessage = statusMessage;
        systemSettingsState.statusKind = statusMessage ? "success" : "";
      }

      function applyAutostartError(event) {
        systemSettingsState.autostartEnabled =
          systemSettingsState.autostartPreviousEnabled === true;
        systemSettingsState.autostartPending = false;
        systemSettingsState.statusMessage =
          event.message || "Failed to update login launch setting.";
        systemSettingsState.statusKind = "error";
      }

      const systemSettingsInteractionGuard = createInteractionGuard({
        onFlush: (deferred) => {
          if (!deferred || typeof deferred !== "object") {
            return;
          }
          if (deferred.kind === "system_settings") {
            systemSettingsState.language = deferred.language || "auto";
            systemSettingsState.codexTrustManagedHooks =
              deferred.codex_trust_managed_hooks !== false;
            systemSettingsState.boardProvider =
              deferred.board_provider || systemSettingsState.boardProvider || "local";
            systemSettingsState.loaded = true;
            if (
              !systemSettingsState.statusMessage
              || systemSettingsState.statusKind === "info"
            ) {
              systemSettingsState.statusMessage = "";
              systemSettingsState.statusKind = "";
            }
          } else if (deferred.kind === "system_settings_updated") {
            systemSettingsState.language = deferred.language
              || systemSettingsState.language;
            systemSettingsState.codexTrustManagedHooks =
              deferred.codex_trust_managed_hooks !== false;
            systemSettingsState.boardProvider =
              deferred.board_provider || systemSettingsState.boardProvider || "local";
            systemSettingsState.statusMessage = "Saved system settings.";
            systemSettingsState.statusKind = "success";
          } else if (deferred.kind === "system_settings_error") {
            systemSettingsState.statusMessage = deferred.message
              || "Failed to update system settings.";
            systemSettingsState.statusKind = "error";
          } else if (deferred.kind === "autostart_status") {
            applyAutostartStatus(
              deferred,
              deferred.from_update ? "Saved login launch setting." : "",
            );
          } else if (deferred.kind === "autostart_error") {
            applyAutostartError(deferred);
          }
          renderSystemPanelInAllSettingsWindows();
        },
      });
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
      let renderedProjectTabsKey = "";
      let renderedRecentProjectsKey = "";
      let renderedRecentProjectsMenuKey = "";
      let renderedWorkspaceWindowsKey = "";
      let renderedWindowListKey = "";
      let renderedAppVersionLabel = null;
      let renderedProjectPickerKey = "";
      let renderedProjectOnboardingKey = "";
      let renderedActionAvailabilityKey = "";
      let renderedOperatorTelemetryKey = "";
      let maximizedViewportSyncFrame = null;

      function projectTabsRenderKey(state) {
        const tabs = state?.tabs || [];
        const parts = [];
        appendRenderKeyPart(parts, "active_tab_id");
        appendRenderKeyPart(parts, state?.active_tab_id || null);
        appendRenderKeyPart(parts, "tabs");
        appendRenderKeyPart(parts, tabs.length);
        for (const tab of tabs) {
          appendRenderKeyPart(parts, "id");
          appendRenderKeyPart(parts, tab?.id || "");
          appendRenderKeyPart(parts, "title");
          appendRenderKeyPart(parts, tab?.title || "");
          appendRenderKeyPart(parts, "project_root");
          appendRenderKeyPart(parts, tab?.project_root || "");
        }
        return parts.join("");
      }

      function recentProjectsRenderKey(state) {
        return JSON.stringify(
          (state?.recent_projects || []).map((project) => ({
            title: project?.title || "",
            kind: project?.kind || "",
            path: project?.path || "",
          })),
        );
      }

      function appendRenderKeyPart(parts, value) {
        const text = String(value ?? "");
        parts.push(String(text.length), ":", text, "\u001f");
      }

      function workspaceWindowsRenderKey(workspace) {
        const windows = workspace?.windows || [];
        const parts = [];
        appendRenderKeyPart(parts, "active_tab_id");
        appendRenderKeyPart(parts, appState?.active_tab_id || null);
        appendRenderKeyPart(parts, "active_window_ids");
        appendRenderKeyPart(parts, windows.length);
        for (const windowData of windows) {
          appendRenderKeyPart(parts, windowData?.id || "");
        }
        appendRenderKeyPart(parts, "all_project_window_ids");
        for (const windowId of allProjectWindowIds()) {
          appendRenderKeyPart(parts, windowId);
        }
        appendRenderKeyPart(parts, "windows");
        appendRenderKeyPart(parts, windows.length);
        for (const windowData of windows) {
          const geometry = windowData?.geometry || {};
          appendRenderKeyPart(parts, "id");
          appendRenderKeyPart(parts, windowData?.id || "");
          appendRenderKeyPart(parts, "preset");
          appendRenderKeyPart(parts, windowData?.preset || "");
          appendRenderKeyPart(parts, "title");
          appendRenderKeyPart(parts, windowData?.title || "");
          appendRenderKeyPart(parts, "dynamic_title");
          appendRenderKeyPart(parts, windowData?.dynamic_title || "");
          appendRenderKeyPart(parts, "dynamic_title_detail");
          appendRenderKeyPart(parts, windowData?.dynamic_title_detail || "");
          appendRenderKeyPart(parts, "purpose_title");
          appendRenderKeyPart(parts, windowData?.purpose_title || "");
          appendRenderKeyPart(parts, "agent_id");
          appendRenderKeyPart(parts, windowData?.agent_id || "");
          appendRenderKeyPart(parts, "agent_color");
          appendRenderKeyPart(parts, windowData?.agent_color || "");
          appendRenderKeyPart(parts, "status");
          appendRenderKeyPart(parts, windowData?.status || "");
          appendRenderKeyPart(parts, "geometry");
          appendRenderKeyPart(parts, "x");
          appendRenderKeyPart(parts, geometry.x ?? 0);
          appendRenderKeyPart(parts, "y");
          appendRenderKeyPart(parts, geometry.y ?? 0);
          appendRenderKeyPart(parts, "width");
          appendRenderKeyPart(parts, geometry.width ?? 0);
          appendRenderKeyPart(parts, "height");
          appendRenderKeyPart(parts, geometry.height ?? 0);
          appendRenderKeyPart(parts, "minimized");
          appendRenderKeyPart(parts, Boolean(windowData?.minimized));
          appendRenderKeyPart(parts, "maximized");
          appendRenderKeyPart(parts, Boolean(windowData?.maximized));
          appendRenderKeyPart(parts, "z_index");
          appendRenderKeyPart(parts, windowData?.z_index ?? 0);
          appendRenderKeyPart(parts, "tab_group_id");
          appendRenderKeyPart(parts, windowData?.tab_group_id || "");
          appendRenderKeyPart(parts, "tab_group_active");
          appendRenderKeyPart(parts, Boolean(windowData?.tab_group_active));
        }
        return parts.join("");
      }

      function windowListRenderKey() {
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

      function windowRuntimeStatusRenderKey(windowId, runtimeState, effectiveDetail, windowData) {
        const parts = [];
        appendRenderKeyPart(parts, "id");
        appendRenderKeyPart(parts, windowId || "");
        appendRenderKeyPart(parts, "mounted");
        appendRenderKeyPart(parts, windowMap.has(windowId));
        appendRenderKeyPart(parts, "runtime_state");
        appendRenderKeyPart(parts, runtimeState || "");
        appendRenderKeyPart(parts, "detail");
        appendRenderKeyPart(parts, effectiveDetail || "");
        appendRenderKeyPart(parts, "preset");
        appendRenderKeyPart(parts, windowData?.preset || "");
        appendRenderKeyPart(parts, "runtime_visible");
        appendRenderKeyPart(parts, shouldShowRuntimeStatus(windowData));
        appendRenderKeyPart(parts, "agent_state");
        appendRenderKeyPart(parts, mapAgentTelemetryState(runtimeState));
        return parts.join("");
      }

      function windowElementRenderKey(windowData) {
        const geometry = windowData.geometry || {};
        const maximizedFill =
          windowData.maximized && !windowData.minimized
            ? maximizedGeometry(visibleBounds(), viewport.zoom)
            : null;
        const tabGroupId = windowGroupId(windowData);
        const parts = [];
        appendRenderKeyPart(parts, "id");
        appendRenderKeyPart(parts, windowData.id || "");
        appendRenderKeyPart(parts, "preset");
        appendRenderKeyPart(parts, windowData.preset || "");
        appendRenderKeyPart(parts, "title");
        appendRenderKeyPart(parts, windowData.title || "");
        appendRenderKeyPart(parts, "dynamic_title");
        appendRenderKeyPart(parts, windowData.dynamic_title || "");
        appendRenderKeyPart(parts, "dynamic_title_detail");
        appendRenderKeyPart(parts, windowData.dynamic_title_detail || "");
        appendRenderKeyPart(parts, "purpose_title");
        appendRenderKeyPart(parts, windowData.purpose_title || "");
        appendRenderKeyPart(parts, "agent_id");
        appendRenderKeyPart(parts, windowData.agent_id || "");
        appendRenderKeyPart(parts, "agent_color");
        appendRenderKeyPart(parts, windowData.agent_color || "");
        appendRenderKeyPart(parts, "status");
        appendRenderKeyPart(parts, windowData.status || "");
        appendRenderKeyPart(parts, "runtime_state");
        appendRenderKeyPart(parts, runtimeStateForWindow(windowData));
        appendRenderKeyPart(parts, "detail");
        appendRenderKeyPart(parts, detailMap.get(windowData.id) || "");
        appendRenderKeyPart(parts, "display_title");
        appendRenderKeyPart(parts, windowDisplayTitle(windowData));
        appendRenderKeyPart(parts, "title_tooltip");
        appendRenderKeyPart(parts, windowTitleTooltip(windowData));
        appendRenderKeyPart(parts, "role_badge");
        appendRenderKeyPart(parts, windowRoleBadgeLabel(windowData));
        appendRenderKeyPart(parts, "geometry_revision");
        appendRenderKeyPart(parts, workspaceGeometryRevision(windowData));
        appendRenderKeyPart(parts, "geometry");
        appendRenderKeyPart(parts, "x");
        appendRenderKeyPart(parts, geometry.x ?? "");
        appendRenderKeyPart(parts, "y");
        appendRenderKeyPart(parts, geometry.y ?? "");
        appendRenderKeyPart(parts, "width");
        appendRenderKeyPart(parts, geometry.width ?? "");
        appendRenderKeyPart(parts, "height");
        appendRenderKeyPart(parts, geometry.height ?? "");
        appendRenderKeyPart(parts, "minimized");
        appendRenderKeyPart(parts, Boolean(windowData.minimized));
        appendRenderKeyPart(parts, "maximized");
        appendRenderKeyPart(parts, Boolean(windowData.maximized));
        appendRenderKeyPart(parts, "z_index");
        appendRenderKeyPart(parts, windowData.z_index ?? "");
        appendRenderKeyPart(parts, "tab_group_id");
        appendRenderKeyPart(parts, windowData.tab_group_id || "");
        appendRenderKeyPart(parts, "tab_group_active");
        appendRenderKeyPart(parts, Boolean(windowData.tab_group_active));
        appendRenderKeyPart(parts, "maximized_fill");
        appendRenderKeyPart(parts, Boolean(maximizedFill));
        if (maximizedFill) {
          appendRenderKeyPart(parts, "x");
          appendRenderKeyPart(parts, maximizedFill.x);
          appendRenderKeyPart(parts, "y");
          appendRenderKeyPart(parts, maximizedFill.y);
          appendRenderKeyPart(parts, "width");
          appendRenderKeyPart(parts, maximizedFill.width);
          appendRenderKeyPart(parts, "height");
          appendRenderKeyPart(parts, maximizedFill.height);
        }
        appendRenderKeyPart(parts, "tabs");
        for (const tab of activeWorkspace().windows || []) {
          if (windowGroupId(tab) !== tabGroupId) {
            continue;
          }
          appendRenderKeyPart(parts, "id");
          appendRenderKeyPart(parts, tab.id || "");
          appendRenderKeyPart(parts, "preset");
          appendRenderKeyPart(parts, tab.preset || "");
          appendRenderKeyPart(parts, "title");
          appendRenderKeyPart(parts, tab.title || "");
          appendRenderKeyPart(parts, "dynamic_title");
          appendRenderKeyPart(parts, tab.dynamic_title || "");
          appendRenderKeyPart(parts, "dynamic_title_detail");
          appendRenderKeyPart(parts, tab.dynamic_title_detail || "");
          appendRenderKeyPart(parts, "purpose_title");
          appendRenderKeyPart(parts, tab.purpose_title || "");
          appendRenderKeyPart(parts, "agent_id");
          appendRenderKeyPart(parts, tab.agent_id || "");
          appendRenderKeyPart(parts, "agent_color");
          appendRenderKeyPart(parts, tab.agent_color || "");
          appendRenderKeyPart(parts, "status");
          appendRenderKeyPart(parts, tab.status || "");
          appendRenderKeyPart(parts, "tab_group_id");
          appendRenderKeyPart(parts, tab.tab_group_id || "");
          appendRenderKeyPart(parts, "tab_group_active");
          appendRenderKeyPart(parts, Boolean(tab.tab_group_active));
        }
        return parts.join("");
      }

      function projectPickerRenderKey(activeTab = activeProjectTab()) {
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
      let versionState = { current: "", latest: "" };
      let pendingAboutHashOpen = false;
      const indexStatusByProjectRoot = new Map();
      let projectError = "";
      const TERMINAL_SELECTION_DRAG_THRESHOLD = 4;

      // SPEC-1939 Phase 13: project-bar Index badge withdrawn. The
      // aggregated payload now feeds only the per-tab dot indicator and the
      // Settings.Index panel; the project-bar surface no longer renders an
      // Index summary because repo-shared (issues/specs) and per-worktree
      // (files/files-docs) scopes were being collapsed into a single state.
      function setIndexStatus(projectRoot, status) {
        if (!projectRoot) {
          return;
        }
        indexStatusByProjectRoot.set(projectRoot, {
          state: status?.state || "",
          detail: status?.detail || "",
          repair_started_at: status?.repair_started_at || null,
          progress: status?.progress || null,
          scopes: status?.scopes || {},
          worktrees: status?.worktrees || {},
        });
        renderIndexPanelInAllSettingsWindows();
        renderProjectIndexWindows();
        refreshProjectTabDots();
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
        if (renderedAppVersionLabel === label) {
          return;
        }
        renderedAppVersionLabel = label;
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
        if (pendingAboutHashOpen && versionState.current) {
          pendingAboutHashOpen = false;
          releaseNotesWindow.openAbout(versionState.current || null);
        }
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
          return "work";
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
        if (preset === "index") {
          return "index";
        }
        if (preset === "work" || preset === "workspace") {
          return "work";
        }
        if (preset === "console") {
          return "console";
        }
        return "mock";
      }

      function knowledgeKindForPreset(preset) {
        if (preset === "issue" || preset === "spec" || preset === "pr") {
          return preset;
        }
        return null;
      }

      const INDEX_SEARCH_SCOPES = Object.freeze([
        { id: "issues", label: "Issues" },
        { id: "specs", label: "SPECs" },
        { id: "board", label: "Board" },
        { id: "discussions", label: "Discussions" },
        { id: "files", label: "Files" },
        { id: "files-docs", label: "Docs" },
        { id: "memory", label: "Memory" },
      ]);
      const INDEX_SEARCH_DEFAULT_SCOPES = Object.freeze([
        "issues",
        "specs",
        "board",
        "discussions",
        "memory",
      ]);
      function ensureIndexSearchState(windowId) {
        if (!indexSearchStateMap.has(windowId)) {
          indexSearchStateMap.set(windowId, {
            activeTab: "search",
            query: "",
            matchMode: "semantic",
            selectedScopes: new Set(INDEX_SEARCH_DEFAULT_SCOPES),
            selectedWorktreeHash: "",
            searchTimer: 0,
            requestId: 0,
            inFlightRequestId: 0,
            inFlightSignature: "",
            searching: false,
            results: [],
            suggestions: [],
            selectedResultIndex: -1,
            error: "",
          });
        }
        return indexSearchStateMap.get(windowId);
      }

      function invalidateProjectIndexSearchRequest(state) {
        state.requestId += 1;
        state.inFlightRequestId = 0;
        state.inFlightSignature = "";
      }

      function clearProjectIndexSearchState(state) {
        if (state.searchTimer) {
          clearTimeout(state.searchTimer);
          state.searchTimer = 0;
        }
        invalidateProjectIndexSearchRequest(state);
        state.results = [];
        state.suggestions = [];
        state.selectedResultIndex = -1;
        state.searching = false;
        state.error = "";
      }

      function markProjectIndexSearchPending(state) {
        invalidateProjectIndexSearchRequest(state);
        state.error = "";
      }

      function activeIndexStatus() {
        const activeProjectRoot = activeProjectTab()?.project_root || "";
        return (activeProjectRoot && indexStatusByProjectRoot.get(activeProjectRoot)) || null;
      }

      function indexWorktreeEntries(status) {
        const worktrees = status?.worktrees || {};
        return Object.entries(worktrees)
          .map(([hash, meta]) => ({
            hash,
            branch: meta?.branch || "",
            path: meta?.path || "",
            label: meta?.branch || meta?.path || hash,
          }))
          .sort((left, right) => left.label.localeCompare(right.label));
      }

      function activeWorktreeHashForIndex(status) {
        const activeProjectRoot = activeProjectTab()?.project_root || "";
        const entries = indexWorktreeEntries(status);
        return (
          entries.find((entry) => entry.path === activeProjectRoot)?.hash
          || entries[0]?.hash
          || ""
        );
      }

      function selectedIndexWorktreeHash(state, status) {
        const entries = indexWorktreeEntries(status);
        if (state.selectedWorktreeHash && entries.some((entry) => entry.hash === state.selectedWorktreeHash)) {
          return state.selectedWorktreeHash;
        }
        return activeWorktreeHashForIndex(status);
      }

      function indexFileScopesSelected(state) {
        return state.selectedScopes.has("files") || state.selectedScopes.has("files-docs");
      }

      function selectedIndexScopeLabels(state) {
        return INDEX_SEARCH_SCOPES
          .filter((scope) => state.selectedScopes.has(scope.id))
          .map((scope) => scope.label);
      }

      function formatIndexSearchMatch(distance) {
        if (!Number.isFinite(distance)) return "";
        const score = Math.max(0, Math.min(1, 1 - distance));
        return `${Math.round(score * 100)}% match`;
      }

      function indexSearchVisibleResults(state) {
        return [
          ...state.results.map((result) => ({ result, suggestion: false })),
          ...state.suggestions.map((result) => ({ result, suggestion: true })),
        ];
      }

      function selectedIndexSearchItem(state) {
        const visible = indexSearchVisibleResults(state);
        return visible[state.selectedResultIndex] || visible[0] || null;
      }

      function formatIndexSearchEvidence(result, includeMissing = false) {
        const matched = Array.isArray(result?.matched_terms) ? result.matched_terms : [];
        const missing = Array.isArray(result?.missing_terms) ? result.missing_terms : [];
        const parts = [];
        if (matched.length > 0) {
          parts.push(`Matched: ${matched.join(", ")}`);
        }
        if (includeMissing && missing.length > 0) {
          parts.push(`Missing: ${missing.join(", ")}`);
        }
        return parts.join(" · ");
      }

      function indexSearchLoadingLabel(state) {
        return state.matchMode === "all_terms" ? "Searching all terms" : "Searching semantic index";
      }

      function indexSearchPlaceholder(state) {
        return state.matchMode === "all_terms"
          ? "All terms required, e.g. Work discussion"
          : "Search by meaning, e.g. work lifecycle";
      }

      function scheduleProjectIndexSearch(windowId) {
        const state = ensureIndexSearchState(windowId);
        if (state.searchTimer) {
          clearTimeout(state.searchTimer);
        }
        state.searchTimer = setTimeout(() => {
          state.searchTimer = 0;
          sendProjectIndexSearch(windowId);
        }, 250);
      }

      function sendProjectIndexSearch(windowId) {
        const state = ensureIndexSearchState(windowId);
        const query = state.query.trim();
        if (!query) {
          clearProjectIndexSearchState(state);
          renderProjectIndexSearch(windowId);
          return;
        }
        const scopes = Array.from(state.selectedScopes);
        if (scopes.length === 0) {
          invalidateProjectIndexSearchRequest(state);
          state.error = "Select at least one scope.";
          state.searching = false;
          renderProjectIndexSearch(windowId);
          return;
        }
        const requestId = state.requestId + 1;
        const status = activeIndexStatus();
        const worktreeHash = indexFileScopesSelected(state)
          ? selectedIndexWorktreeHash(state, status)
          : "";
        const matchMode = state.matchMode || "semantic";
        const searchSignature = JSON.stringify({ query, scopes, worktreeHash, matchMode });
        if (state.searching && state.inFlightSignature === searchSignature) {
          renderProjectIndexSearch(windowId);
          return;
        }
        state.requestId = requestId;
        state.inFlightRequestId = requestId;
        state.inFlightSignature = searchSignature;
        state.searching = true;
        state.error = "";
        send({
          kind: "search_project_index",
          id: windowId,
          query,
          request_id: requestId,
          scopes,
          match_mode: state.matchMode,
          worktree_hash: worktreeHash || null,
        });
        renderProjectIndexSearch(windowId);
      }

      function renderProjectIndexSearch(windowId) {
        const state = ensureIndexSearchState(windowId);
        const root = document.querySelector(
          `[data-id='${CSS.escape(windowId)}'] .index-search-root`,
        );
        if (!root) return;
        const status = activeIndexStatus();
        const searchPanel = root.querySelector("[data-index-panel='search']");
        const healthPanel = root.querySelector("[data-index-panel='health']");
        const healthTable = root.querySelector(".index-health-table");
        const tabs = root.querySelectorAll("[data-index-tab]");
        tabs.forEach((tab) => {
          const active = tab.dataset.indexTab === state.activeTab;
          tab.classList.toggle("active", active);
          tab.setAttribute("aria-selected", String(active));
        });
        if (searchPanel) {
          searchPanel.hidden = state.activeTab !== "search";
        }
        if (healthPanel) {
          healthPanel.hidden = state.activeTab !== "health";
        }
        renderProjectIndexSearchControls(root, state, status);
        renderProjectIndexSearchResults(root, state);
        if (healthTable) {
          renderIndexSettingsPanel({
            panel: healthTable,
            status,
            projectRoot: activeProjectTab()?.project_root || "",
            send,
          });
        }
      }

      function renderProjectIndexSearchControls(root, state, status) {
        const scopes = root.querySelector(".index-scope-list");
        if (scopes) {
          clearChildren(scopes);
          for (const scope of INDEX_SEARCH_SCOPES) {
            const button = makeEl("button", {
              className: "index-scope-button",
              attrs: {
                type: "button",
                "aria-pressed": String(state.selectedScopes.has(scope.id)),
              },
              dataset: { scope: scope.id },
              text: scope.label,
            });
            scopes.appendChild(button);
          }
        }
        const matchModes = root.querySelectorAll("[data-match-mode]");
        matchModes.forEach((button) => {
          const active = button.dataset.matchMode === state.matchMode;
          button.classList.toggle("active", active);
          button.setAttribute("aria-pressed", String(active));
        });
        const runButton = root.querySelector(".index-run-button");
        if (runButton) {
          runButton.disabled = !state.query.trim() || state.searching;
          runButton.textContent = state.searching ? "Searching" : "Search";
        }
        const input = root.querySelector(".index-search-input");
        if (input) {
          input.placeholder = indexSearchPlaceholder(state);
        }
        const fileWorktreeField = root.querySelector(".index-worktree-field");
        if (fileWorktreeField) {
          fileWorktreeField.hidden = !indexFileScopesSelected(state);
        }
        const select = root.querySelector(".index-worktree-select");
        if (select) {
          clearChildren(select);
          select.disabled = !indexFileScopesSelected(state);
          const entries = indexWorktreeEntries(status);
          const selectedHash = selectedIndexWorktreeHash(state, status);
          if (entries.length === 0) {
            select.appendChild(makeEl("option", { attrs: { value: "" }, text: "Current workspace" }));
          } else {
            for (const entry of entries) {
              select.appendChild(
                makeEl("option", {
                  attrs: { value: entry.hash },
                  text: entry.label,
                }),
              );
            }
          }
          select.value = selectedHash;
        }
        const statusNode = root.querySelector(".index-search-status");
        if (statusNode) {
          const scopeSummary = selectedIndexScopeLabels(state).join(", ");
          if (state.searching) {
            statusNode.textContent = state.results.length > 0
              ? `Updating results · ${scopeSummary}`
              : `${indexSearchLoadingLabel(state)} · ${scopeSummary}`;
          } else if (state.error) {
            statusNode.textContent = state.error;
          } else if (state.query.trim() && indexSearchVisibleResults(state).length > 0) {
            statusNode.textContent = state.matchMode === "all_terms"
              ? `${state.results.length} strict results · ${state.suggestions.length} semantic suggestions · ${scopeSummary}`
              : `${state.results.length} results · ${scopeSummary}`;
          } else if (state.query.trim()) {
            statusNode.textContent = "No indexed results";
          } else {
            statusNode.textContent = `Ready · ${scopeSummary}`;
          }
          statusNode.dataset.kind = state.error ? "error" : "";
        }
      }

      function renderProjectIndexSearchResults(root, state) {
        const layout = root.querySelector(".index-search-layout");
        const list = root.querySelector(".index-result-list");
        const detail = root.querySelector(".index-result-detail");
        if (!list || !detail) return;
        const visibleResults = indexSearchVisibleResults(state);
        clearChildren(list);
        clearChildren(detail);
        layout?.classList.toggle("is-empty", visibleResults.length === 0);
        if (layout) {
          layout.setAttribute("aria-busy", String(Boolean(state.searching)));
        }
        if (!state.query.trim()) {
          list.appendChild(makeEl("div", { className: "workspace-empty-state", text: "Search indexed content." }));
        } else if (state.searching && state.results.length === 0) {
          list.appendChild(makeIndexSearchLoadingState(indexSearchLoadingLabel(state)));
        } else if (state.error) {
          list.appendChild(makeEl("div", { className: "workspace-empty-state", text: state.error }));
        } else if (visibleResults.length === 0) {
          list.appendChild(makeEl("div", { className: "workspace-empty-state", text: "No indexed results" }));
        } else {
          if (state.searching) {
            list.appendChild(makeIndexSearchLoadingState("Updating results"));
          }
          let rowIndex = 0;
          const appendResultGroup = (label, items, suggestion) => {
            if (items.length === 0) return;
            if (state.matchMode === "all_terms") {
              list.appendChild(makeEl("div", { className: "index-result-group-label", text: label }));
            }
            items.forEach((result) => {
              const index = rowIndex;
              rowIndex += 1;
              const row = makeEl("button", {
                className: `index-result-row${suggestion ? " is-suggestion" : ""}`,
                attrs: {
                  type: "button",
                  "aria-selected": String(index === state.selectedResultIndex),
                },
                dataset: { resultIndex: String(index) },
              });
              row.appendChild(makeEl("span", { className: "index-result-scope", text: result.scope || "" }));
              row.appendChild(makeEl("strong", { text: result.title || "Untitled" }));
              row.appendChild(makeEl("span", {
                className: "index-result-subtitle",
                text: [result.subtitle || "", formatIndexSearchMatch(result.distance)].filter(Boolean).join(" · "),
              }));
              const evidence = formatIndexSearchEvidence(result);
              if (evidence) {
                row.appendChild(makeEl("span", { className: "index-result-evidence", text: evidence }));
              }
              list.appendChild(row);
            });
          };
          appendResultGroup("Strict results", state.results, false);
          appendResultGroup("Semantic suggestions", state.suggestions, true);
        }

        const selectedItem = selectedIndexSearchItem(state);
        if (!selectedItem) {
          detail.appendChild(makeEl("div", { className: "workspace-empty-state", text: "Select a result" }));
          return;
        }
        const selected = selectedItem.result;
        state.selectedResultIndex = Math.max(
          0,
          Math.min(visibleResults.length - 1, state.selectedResultIndex),
        );
        detail.appendChild(makeEl("h3", { className: "index-detail-title", text: selected.title || "Untitled" }));
        detail.appendChild(makeEl("div", { className: "index-detail-subtitle", text: selected.subtitle || selected.scope || "" }));
        const match = formatIndexSearchMatch(selected.distance);
        if (match) {
          detail.appendChild(makeEl("div", { className: "index-detail-meta", text: match }));
        }
        const evidence = formatIndexSearchEvidence(selected, true);
        if (evidence) {
          detail.appendChild(makeEl("div", { className: "index-detail-meta", text: evidence }));
        }
        detail.appendChild(makeEl("p", { className: "index-detail-preview", text: selected.preview || "No preview available" }));
        detail.appendChild(makeEl("button", {
          className: "wizard-button primary",
          attrs: { type: "button" },
          dataset: { action: "open-index-result" },
          text: "Open",
        }));
      }

      function makeIndexSearchLoadingState(label = "Searching semantic index") {
        const node = makeEl("div", {
          className: "index-search-loading",
          attrs: { role: "status", "aria-live": "polite" },
        });
        const dots = makeEl("span", {
          className: "index-search-loading-dots",
          attrs: { "aria-hidden": "true" },
        });
        for (let i = 0; i < 3; i += 1) {
          dots.appendChild(makeEl("span", { className: "index-search-loading-dot" }));
        }
        node.appendChild(dots);
        node.appendChild(makeEl("span", { className: "index-search-loading-label", text: label }));
        return node;
      }

      function handleProjectIndexSearchResults(event) {
        const state = indexSearchStateMap.get(event.id);
        if (!state || event.request_id !== state.inFlightRequestId) {
          return;
        }
        state.searching = false;
        state.inFlightRequestId = 0;
        state.inFlightSignature = "";
        state.error = "";
        state.results = Array.isArray(event.results) ? event.results : [];
        state.suggestions = Array.isArray(event.suggestions) ? event.suggestions : [];
        state.selectedResultIndex = indexSearchVisibleResults(state).length > 0 ? 0 : -1;
        renderProjectIndexSearch(event.id);
      }

      function handleProjectIndexSearchError(event) {
        const state = indexSearchStateMap.get(event.id);
        if (!state || event.request_id !== state.inFlightRequestId) {
          return;
        }
        state.searching = false;
        state.inFlightRequestId = 0;
        state.inFlightSignature = "";
        state.error = event.message || "Project index search failed.";
        renderProjectIndexSearch(event.id);
      }

      function renderProjectIndexWindows() {
        for (const windowId of indexSearchStateMap.keys()) {
          renderProjectIndexSearch(windowId);
        }
      }

      function moveIndexResultSelection(windowId, delta) {
        const state = ensureIndexSearchState(windowId);
        const visibleResults = indexSearchVisibleResults(state);
        if (visibleResults.length === 0) return;
        const current = Math.max(0, state.selectedResultIndex);
        state.selectedResultIndex = Math.max(0, Math.min(visibleResults.length - 1, current + delta));
        renderProjectIndexSearch(windowId);
        document
          .querySelector(`[data-id='${CSS.escape(windowId)}'] [data-result-index='${state.selectedResultIndex}']`)
          ?.focus();
      }

      function openIndexResultTarget(result) {
        const target = result?.target || {};
        switch (target.kind) {
          case "issue":
            openKnowledgeIndexResultTarget("issue", target);
            return;
          case "spec":
            openKnowledgeIndexResultTarget("spec", target);
            return;
          case "board":
            focusOrSpawnPreset("board");
            return;
          case "file":
            focusOrSpawnPreset("file_tree");
            return;
          case "memory":
          case "discussion":
            focusOrSpawnPreset("index");
            return;
          default:
            return;
        }
      }

      function indexResultTargetNumber(target) {
        const rawNumber = target?.number ?? target?.spec_id ?? target?.id;
        const number = Number(rawNumber);
        if (!Number.isInteger(number) || number <= 0) {
          return null;
        }
        return number;
      }

      function openKnowledgeIndexResultTarget(preset, target) {
        const knowledgeKind = knowledgeKindForPreset(preset);
        const number = indexResultTargetNumber(target);
        const windowId = focusOrSpawnPreset(preset);
        if (!knowledgeKind || number === null) {
          return;
        }
        if (windowId) {
          requestKnowledgeDetail(windowId, knowledgeKind, number);
          renderKnowledgeBridge(windowId);
          return;
        }
        pendingIndexOpenTargetsByPreset.set(preset, { knowledgeKind, number });
      }

      function send(message) {
        if (socket && socket.readyState === WebSocket.OPEN) {
          socket.send(JSON.stringify(message));
          return;
        }
        pendingMessages.push(message);
      }

      const uiTraceWiring = createUiTraceWiring({
        profiler: uiTraceProfiler,
        send,
        alert: (message) => window.alert(message),
        log: (message) => console.info(message),
      });
      const traceUi = uiTraceWiring.traceUi;
      const tracePointer = uiTraceWiring.tracePointer;
      const traceMeasure = uiTraceWiring.traceMeasure;

      // SPEC-2359 US-41 Phase 8b: surface Workspace projection prune through
      // the Command Palette. The dry-run entry previews the plan; the apply
      // entry confirms before mutating projection files on disk.
      if (window.__operatorShell?.palette) {
        uiTraceWiring.registerPalette(window.__operatorShell.palette);
        window.__operatorShell.palette.register({
          id: "workspace-projection-prune-dry-run",
          label: "Work: Prune Stale Projections (dry-run)",
          group: "Work",
          handler: () => {
            send({
              kind: "workspace_projection_prune",
              dry_run: true,
              ids: [],
            });
          },
        });
        window.__operatorShell.palette.register({
          id: "workspace-projection-prune-apply",
          label: "Work: Prune Stale Projections (apply)",
          group: "Work",
          handler: () => {
            if (
              window.confirm(
                "Apply Work projection prune now? Archived entries past their grace period will be physically removed.",
              )
            ) {
              send({
                kind: "workspace_projection_prune",
                dry_run: false,
                ids: [],
              });
            }
          },
        });
      }

      // SPEC-2809 — registry of Console window controllers keyed by windowId.
      // Each Console window registers itself on render and unregisters on
      // close; the `process_line` dispatcher fans out incoming lines to every
      // active controller so multiple Console windows stay in sync.
      const consoleControllers = new Map();
      function ensureConsoleController(windowId) {
        let controller = consoleControllers.get(windowId);
        if (!controller) {
          controller = createConsoleWindow({
            document,
            windowId,
            send: (payload) => socketTransport.send(payload),
          });
          consoleControllers.set(windowId, controller);
        }
        return controller;
      }
      function disposeConsoleController(windowId) {
        const controller = consoleControllers.get(windowId);
        if (controller) {
          controller.close();
          consoleControllers.delete(windowId);
        }
      }
      function broadcastProcessLineToConsoles(line) {
        for (const controller of consoleControllers.values()) {
          controller.push(line);
        }
      }

      const releaseNotesWindow = createReleaseNotesWindow({
        document,
        send,
        beginUpdateDownloading: (version) =>
          updateCtaController.beginDownloadingFor(version),
      });

      function consumeAboutHash() {
        if (window.location.hash === "#about") {
          if (window.history && typeof window.history.replaceState === "function") {
            window.history.replaceState(
              null,
              "",
              `${window.location.pathname}${window.location.search}`,
            );
          }
          if (versionState.current) {
            releaseNotesWindow.openAbout(versionState.current || null);
          } else {
            pendingAboutHashOpen = true;
          }
        }
      }
      window.addEventListener("hashchange", consumeAboutHash);
      consumeAboutHash();

      if (appVersionLabel && !appVersionLabel.dataset.releaseNotesBound) {
        appVersionLabel.dataset.releaseNotesBound = "true";
        const openReleaseNotesFromLabel = () => {
          releaseNotesWindow.open(versionState.current || null);
        };
        appVersionLabel.addEventListener("click", openReleaseNotesFromLabel);
        appVersionLabel.addEventListener("keydown", (event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            openReleaseNotesFromLabel();
          }
        });
      }

      const updateCtaController = createUpdateCtaController({
        document,
        send,
        setVersionState,
        confirmUpdate: (version) =>
          window.confirm(`Apply update to v${version} now?\n\ngwt will restart automatically.`),
        openReleaseNotes: (version) => releaseNotesWindow.open(version || null),
      });

      function setConnectionState(connected) {
        connectionDot.classList.toggle("connected", connected);
        connectionLabel.textContent = connected ? "Connected" : "Reconnecting";
        // SPEC-2356 — propagate connection state to the Operator Status Strip
        // so the bottom strip clearly reflects whether the WebSocket bridge is
        // online. The class is set on the strip element and consumed via CSS.
        const strip = document.getElementById("op-status-strip");
        const connectionStatusLabel = strip?.querySelector(
          "[data-role='connection-label']",
        );
        if (connectionStatusLabel) {
          connectionStatusLabel.textContent = connected ? "ONLINE" : "OFFLINE";
        }
        if (strip) {
          strip.classList.toggle("op-status-strip--offline", !connected);
        }
        if (!connected) {
          for (const [windowId, state] of branchListStateMap.entries()) {
            let shouldRenderBranches = false;
            if (
              failRunningBranchCleanup(
                windowId,
                "Connection lost while cleaning up branches",
              )
            ) {
              shouldRenderBranches = true;
            }
            if (failLoadingBranchesOnConnectionLoss(windowId, state)) {
              shouldRenderBranches = true;
            }
            if (shouldRenderBranches) {
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
        socketReceiveDispatcherGeneration += 1;
        const ownGeneration = socketReceiveDispatcherGeneration;
        socketReceiveDispatcher = createSocketReceiveDispatcher({
          receive: (event) => {
            if (ownGeneration !== socketReceiveDispatcherGeneration) {
              return;
            }
            receive(event);
          },
          onTrace: (kind, fields) => {
            traceUi(kind, fields);
          },
          shouldTrace: uiTraceWiring.isTracing,
        });
        setConnectionState(true);
        send({ kind: "frontend_ready" });
        while (pendingMessages.length > 0) {
          socket.send(JSON.stringify(pendingMessages.shift()));
        }
      }

      function handleSocketMessage(event) {
        if (!socketReceiveDispatcher) {
          return;
        }
        socketReceiveDispatcher.handle(event);
      }

      // SPEC-2041 Phase 19 (Issue #2832 follow-up): synthetic event injection
      // hook used by Playwright spec `update-modal.spec.ts` to drive the
      // post-click update modal flow without a real GitHub release.
      // Listeners forward `window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: <payload> }))`
      // straight into `receive(...)`, which is the same entrypoint that
      // processed WebSocket frames use. The event name is double-underscored
      // by convention (internal hook) and the payload must be the same
      // discriminated union the backend emits (e.g. `{ kind: "update_state", ... }`).
      // No-op without `event.detail.kind` so accidental dispatches do not
      // misbehave.
      //
      // When the test injects an `update_*` event, set the test-mode flag so
      // subsequent live backend `update_*` messages are dropped — the live
      // gwt server emits its own real update_state / update_apply_error from
      // the periodic update checker, which used to race against the
      // synthetic flow and clobber the modal's reason / detach buttons
      // mid-click. The flag is page-scoped (no global state outlives the
      // Playwright page) so it never leaks into the production runtime.
      let __testInjectModeActive = false;
      window.addEventListener("__gwt_test_inject", (event) => {
        const payload = event && event.detail;
        if (!payload || typeof payload.kind !== "string") {
          return;
        }
        if (payload.kind.startsWith("update_")) {
          __testInjectModeActive = true;
        }
        payload.__injected = true;
        try {
          receive(payload);
        } catch (err) {
          console.warn("[gwt_test_inject] receive failed", err);
        }
      });
      function shouldDropLiveEventForTestMode(event) {
        if (!__testInjectModeActive) return false;
        if (!event || typeof event.kind !== "string") return false;
        if (event.__injected) return false;
        return event.kind.startsWith("update_");
      }

      function handleSocketClose() {
        socketReceiveDispatcherGeneration += 1;
        socketReceiveDispatcher = null;
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

      function allProjectWindowIds() {
        const ids = [];
        for (const tab of appState?.tabs || []) {
          for (const windowData of tab.workspace?.windows || []) {
            ids.push(windowData.id);
          }
        }
        return ids;
      }

      function allProjectWindowIdSet() {
        const ids = new Set();
        for (const tab of appState?.tabs || []) {
          for (const windowData of tab.workspace?.windows || []) {
            ids.add(windowData.id);
          }
        }
        return ids;
      }

      function workspaceWindowIdSet(workspace) {
        const ids = new Set();
        for (const windowData of workspace?.windows || []) {
          ids.add(windowData.id);
        }
        return ids;
      }

      function workspaceWindowById(windowId) {
        return activeWorkspace().windows.find((windowData) => windowData.id === windowId) || null;
      }

      function workspaceWindowElement(windowId) {
        return windowMap.get(windowId) || null;
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

      function trackWindowTabDragPoint(event) {
        if (!windowTabDragState) return;
        const point = clientPointFromDragEvent(event, canvas.getBoundingClientRect());
        if (point) {
          windowTabDragState.lastClientPoint = point;
        }
      }

      function detachGeometryFromTabDrag(event, drag, windowData) {
        const canvasRect = canvas.getBoundingClientRect();
        const releasePoint = resolveDragReleasePoint(
          event,
          drag?.lastClientPoint,
          canvasRect,
        );
        return detachGeometryFromClientPoint(
          releasePoint,
          windowData,
          canvasRect,
          viewport,
        );
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
          workspace: "Work",
          board: "Board",
          pr: "PR",
        };
        return labels[preset] || presetLabel(preset);
      }

      const AGENT_ROLE_LABELS = Object.freeze({
        claude: "Claude Code",
        "claude-code": "Claude Code",
        claudecode: "Claude Code",
        "claude code": "Claude Code",
        claude_code: "Claude Code",
        codex: "Codex",
        gemini: "Gemini CLI",
        "gemini-cli": "Gemini CLI",
        "gemini cli": "Gemini CLI",
        gemini_cli: "Gemini CLI",
        opencode: "OpenCode",
        "open-code": "OpenCode",
        open_code: "OpenCode",
        openclaw: "OpenClaw",
        "open-claw": "OpenClaw",
        open_claw: "OpenClaw",
        hermes: "Hermes Agent",
        "hermes-agent": "Hermes Agent",
        "hermes agent": "Hermes Agent",
        hermes_agent: "Hermes Agent",
        gh: "GitHub Copilot",
        copilot: "GitHub Copilot",
        "github-copilot": "GitHub Copilot",
        "github copilot": "GitHub Copilot",
        github_copilot: "GitHub Copilot",
      });

      const GENERIC_AGENT_ROLE_LABELS = new Set(["agent", "window"]);

      function normalizedAgentRoleKey(value) {
        return String(value || "").trim().toLowerCase();
      }

      function isGenericAgentRoleLabel(value) {
        return GENERIC_AGENT_ROLE_LABELS.has(normalizedAgentRoleKey(value));
      }

      function isAgentWindowPreset(preset) {
        return preset === "agent" || preset === "claude" || preset === "codex";
      }

      function agentRoleLabel(windowData) {
        const agentIdLabel =
          AGENT_ROLE_LABELS[normalizedAgentRoleKey(windowData?.agent_id)] || "";
        if (agentIdLabel) return agentIdLabel;
        const presetLabel =
          AGENT_ROLE_LABELS[normalizedAgentRoleKey(windowData?.preset)] || "";
        if (presetLabel) return presetLabel;
        const launchTitle = String(windowData?.title || "").trim();
        if (launchTitle && !isGenericAgentRoleLabel(launchTitle)) return launchTitle;
        return "";
      }

      function windowRoleBadgeLabel(windowData) {
        const displayTitle = windowDisplayTitle(windowData);
        const isAgentWindow = isAgentWindowPreset(windowData?.preset);
        const label = isAgentWindow
          ? agentRoleLabel(windowData)
          : presetRoleLabel(windowData?.preset || "");
        if (!label) return "";
        if (!isAgentWindow && label === displayTitle) return "";
        return label;
      }

      function setWindowRoleBadge(badgeElement, windowData) {
        if (!badgeElement) return;
        const label = windowRoleBadgeLabel(windowData);
        badgeElement.textContent = label;
        badgeElement.hidden = !label;
      }

      function shouldShowRuntimeStatus(windowData) {
        return presetSurface(windowData?.preset) === "terminal";
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
        const fill = maximizedGeometry(visibleBounds(), viewport.zoom);
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

      function renderAppState(nextState) {
        dismissOperatorBriefing();
        return traceMeasure(
          UI_TRACE_EVENT.renderAppState,
          { tabs: Array.isArray(nextState?.tabs) ? nextState.tabs.length : 0 },
          () => {
            appState = nextState || {
              app_version: "",
              tabs: [],
              active_tab_id: null,
              recent_projects: [],
            };
            setVersionState(appState.app_version, versionState.latest);
            const nextProjectTabsKey = projectTabsRenderKey(appState);
            if (renderedProjectTabsKey !== nextProjectTabsKey) {
              renderedProjectTabsKey = nextProjectTabsKey;
              renderProjectTabs();
            }
            const tab = activeProjectTab();
            renderProjectPicker(tab);
            updateActionAvailability(tab);
            renderProjectOnboarding(tab);
            renderWorkspace(tab?.workspace || emptyWorkspace());
            syncCurrentProjectWorkspaceIds(
              deriveCurrentProjectWorkspaceIds(tab?.workspace || {}),
            );
            renderWindowList();
          },
        );
      }

      let presetModalFocusReturn = null;
      let presetModalFocusTrapRelease = null;
      // SPEC-2356 "Surface Deck" — arrow-key roving across the 2-column
      // preset grid. Buttons are walked in document order; ←→ step within a
      // row, ↑↓ jump a full row (columns = 2). `.is-active` mirrors the
      // currently-focused tile so the keyboard glow tracks the cursor.
      // SPEC-2356 landscape weighted deck — roving is geometry direction-nearest
      // (not a fixed column-count jump) so the 45/33/22 weighted columns and the
      // uneven per-column row counts navigate intuitively. The handler reads real
      // tile geometry; a too-diagonal candidate (>~63°) is rejected so pressing
      // Down at the bottom of a short column clamps instead of leaping columns.
      function findGeometryNeighbor(buttons, current, key) {
        const src = buttons[current].getBoundingClientRect();
        const sx = src.left + src.width / 2;
        const sy = src.top + src.height / 2;
        const AXIS_BIAS = 2.5;
        let best = -1;
        let bestScore = Infinity;
        buttons.forEach((btn, i) => {
          if (i === current) return;
          const r = btn.getBoundingClientRect();
          const dx = r.left + r.width / 2 - sx;
          const dy = r.top + r.height / 2 - sy;
          const inDirection =
            (key === "ArrowRight" && dx > 1) ||
            (key === "ArrowLeft" && dx < -1) ||
            (key === "ArrowDown" && dy > 1) ||
            (key === "ArrowUp" && dy < -1);
          if (!inDirection) return;
          const horizontal = key === "ArrowRight" || key === "ArrowLeft";
          const primary = horizontal ? Math.abs(dx) : Math.abs(dy);
          const secondary = horizontal ? Math.abs(dy) : Math.abs(dx);
          if (secondary > primary * 2) return;
          const score = primary + secondary * AXIS_BIAS;
          if (score < bestScore) {
            bestScore = score;
            best = i;
          }
        });
        return best;
      }
      function presetRovingButtons() {
        return [...modal.querySelectorAll(".preset-button")];
      }
      function setActivePresetButton(buttons, index) {
        if (!buttons.length) return;
        const clamped = Math.max(0, Math.min(index, buttons.length - 1));
        buttons.forEach((button, i) => {
          button.classList.toggle("is-active", i === clamped);
        });
        const target = buttons[clamped];
        if (target && typeof target.focus === "function") {
          try { target.focus({ preventScroll: true }); }
          catch { target.focus(); }
        }
      }
      function handlePresetRovingKeydown(event) {
        const key = event.key;
        if (
          key !== "ArrowRight" &&
          key !== "ArrowLeft" &&
          key !== "ArrowUp" &&
          key !== "ArrowDown" &&
          key !== "Enter"
        ) {
          return;
        }
        const buttons = presetRovingButtons();
        if (!buttons.length) return;
        let current = buttons.indexOf(document.activeElement);
        if (current < 0) {
          current = buttons.findIndex((button) =>
            button.classList.contains("is-active"),
          );
          if (current < 0) current = 0;
        }
        if (key === "Enter") {
          event.preventDefault();
          buttons[current].click();
          return;
        }
        event.preventDefault();
        const next = findGeometryNeighbor(buttons, current, key);
        if (next >= 0) setActivePresetButton(buttons, next);
      }
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
        // SPEC-2356 "Surface Deck" — drop focus onto the first preset tile so
        // arrow-key roving works immediately without a stray Tab.
        const buttons = presetRovingButtons();
        if (buttons.length) {
          setActivePresetButton(buttons, 0);
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
          // SPEC-2356 — clear the roving highlight so a re-open starts clean.
          for (const button of presetRovingButtons()) {
            button.classList.remove("is-active");
          }
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
        focusOrSpawnPreset("work");
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
            merge_target: { kind: "workspace", reference: "Work complete" },
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
          forceFilesystemDelete: false,
          progress: null,
          results: [],
        };
        branchCleanupWindowId = WORKSPACE_CLEANUP_WINDOW_ID;
        renderBranchCleanupModal();
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
        const displayLabels = visibleKnowledgeLabels(detail.labels || []);
        const stalePhase = staleKnowledgePhaseWarning(detail);
        if (displayLabels.length > 0 || stalePhase) {
          const labelRow = createNode("div", "knowledge-label-row");
          for (const label of displayLabels) {
            labelRow.appendChild(createNode("span", "knowledge-chip", label));
          }
          if (stalePhase) {
            labelRow.appendChild(
              createNode("span", "kanban-card-chip kanban-card-chip--warning", stalePhase),
            );
          }
          body.appendChild(labelRow);
        }
        for (const section of detail.sections || []) {
          const card = createNode("section", "kanban-drawer-section");
          card.appendChild(
            createNode("div", "kanban-drawer-section-title", section.title),
          );
          card.appendChild(
            createKnowledgeMarkdownBody(section, "kanban-drawer-section-body"),
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
        traceUi(UI_TRACE_EVENT.applyViewport, {
          x: viewport.x,
          y: viewport.y,
          zoom: viewport.zoom,
        });
        viewportDomApplied = true;
      }

      // Issue #2698 PR 2 (B1) — throttle the `update_viewport` WS
      // stream. Previously every wheel/zoom event fired a send (up
      // to 60-120/sec on a Retina trackpad). Backend re-broadcasts
      // the resulting workspace_state to every client, so the spam
      // turns into a frontend re-render storm. The throttle keeps
      // sustained gestures under ~5 msg/sec while a `tailMs`-delay
      // commit lands when the user finally stops scrolling.
      const persistViewportThrottle = createViewportPersistThrottle({
        send: (payload) => {
          send({
            kind: "update_viewport",
            viewport: payload,
          });
        },
        tailMs: 100,
        maxWaitMs: 500,
      });

      function persistViewport() {
        persistViewportThrottle.schedule(viewport);
      }

      function flushPersistViewport() {
        // Definitive commit points (pointerup, window close,
        // visibility change) should not wait for the tail window.
        // Re-schedule first so the throttle captures the latest
        // viewport reference, then drain immediately.
        persistViewportThrottle.schedule(viewport);
        persistViewportThrottle.flushNow();
      }

      function activeViewportScopeKey() {
        return appState?.active_tab_id || "";
      }

      function recordLocalViewportEdit() {
        viewport = viewportSyncState.applyLocalViewport(viewport, {
          scopeKey: activeViewportScopeKey(),
        });
      }

      function sameViewportValues(left, right) {
        return left
          && right
          && left.x === right.x
          && left.y === right.y
          && left.zoom === right.zoom;
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
        const nextViewport = {
          x: anchorX - worldX * clampedZoom,
          y: anchorY - worldY * clampedZoom,
          zoom: clampedZoom,
        };
        if (sameViewportValues(viewport, nextViewport)) {
          return;
        }
        viewport = nextViewport;
        recordLocalViewportEdit();
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

      function sendStartupAutoResumeReady() {
        if (startupAutoResumeReadySent) {
          return;
        }
        startupAutoResumeReadySent = true;
        requestAnimationFrame(() => {
          send({
            kind: "startup_auto_resume_ready",
            bounds: visibleBounds(),
          });
        });
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
        if (
          modal.classList.contains("open") ||
          wizardModal.classList.contains("open") ||
          cloneProjectModal?.classList.contains("open")
        ) {
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

      // SPEC-2008 Phase 24 / T-188: a hidden tab (display:none) skips fit /
      // refresh just like a minimized window. The shared predicate lives in
      // `terminal-viewport-reflow.js` so the host resize controller and
      // unit tests can reuse it.
      function canRefreshTerminalViewport(windowId) {
        return viewportEligibleForRefresh({
          element: windowMap.get(windowId),
          workspaceWindow: workspaceWindowById(windowId),
        });
      }

      // SPEC-2008 Phase 26.A regression fix (Issue #2832): completeInitialFitHandshake
      // must verify the container has a real layout box before flipping
      // `isReady` true, otherwise fit resolves against a 0-sized parent
      // and the deferredWrites flush into xterm's default 80×24 grid.
      // 60 frames at the default 60Hz cap retries to ~1 s, after which we
      // fall through so a permanently 0-size window can not pin Claude
      // Code output forever.
      const HANDSHAKE_RETRY_LIMIT = 60;

      function terminalContainerHasLayoutBox(windowId) {
        const runtime = terminalMap.get(windowId);
        const terminalHost = runtime?.terminal?.element?.parentElement;
        if (terminalHost) {
          return elementHasLayoutBox(terminalHost);
        }
        const element = windowMap.get(windowId);
        // Fall through to true when the element is not registered yet — the
        // initial-fit handshake is gated by canRefreshTerminalViewport which
        // already short-circuits the no-element case.
        return elementHasLayoutBox(element);
      }

      function retryInitialFitHandshake(windowId, runtime, reason) {
        runtime.handshakeAttempts = (runtime.handshakeAttempts || 0) + 1;
        if (runtime.handshakeAttempts <= HANDSHAKE_RETRY_LIMIT) {
          requestAnimationFrame(() => completeInitialFitHandshake(windowId));
          return;
        }
        console.warn(
          `[gwt] terminal ${windowId} initial-fit handshake gave up after ${HANDSHAKE_RETRY_LIMIT} attempts; ${reason}.`,
        );
      }

      function fitTerminal(windowId, persist = false) {
        return traceMeasure(
          UI_TRACE_EVENT.fitTerminal,
          { window_id: windowId, persist },
          () => {
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
            runTerminalActivationSequence({
              runtime,
              windowId,
              shouldFocus: false,
              shouldPersistGeometry: persist,
              sendGeometry,
            });
          }
        );
      }

      function scheduleTerminalFit(windowId, persist = false) {
        if (!terminalFitScheduler) {
          requestAnimationFrame(() => fitTerminal(windowId, persist));
          return false;
        }
        return terminalFitScheduler.enqueue(windowId, { persist });
      }

      terminalFitScheduler = createTerminalFitScheduler({ fitTerminal });

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

      function finishWindowResize(pointerId, event = null) {
        if (!resizeState || resizeState.pointerId !== pointerId) {
          return;
        }
        const runtime = terminalMap.get(resizeState.id);
        syncResizeStatePointerEvent(resizeState, event);
        cancelTerminalResizeFit();
        cancelResizePointermoveApply();
        cancelResizeStalenessGuard();
        // Flush the last pointer coordinates to the DOM so the final geometry
        // matches the latest pointer event. fitTerminal + sendGeometry below
        // observe the up-to-date `element.style.width/height`.
        applyResizePointermove(resizeState);
        fitTerminal(resizeState.id, false);
        commitLocalGeometryEdit(
          geometrySyncState,
          resizeState.id,
          resizeState.baseGeometryRevision,
        );
        sendGeometry(
          resizeState.id,
          runtime?.terminal.cols || 80,
          runtime?.terminal.rows || 24,
          resizeState.baseGeometryRevision,
        );
        runtime?.terminal.focus();
        resizeState = null;
        // SPEC-2356 Phase 9 (T-136): release the hover-reveal peek strip lock
        // so pointer events resume on the screen-edge triggers once resize
        // ends.
        delete document.documentElement.dataset.opResizeActive;
      }

      // SPEC-2014 Phase C4: coalesce pointermove-driven `style.width/height`
      // writes via requestAnimationFrame. Without this, fast trackpads /
      // 120Hz pointers can cause Windows WebView2 to spend more time in
      // layout than in paint, manifesting as the resize freeze.
      function scheduleResizePointermoveApply() {
        if (!resizeState || resizeState.applyFrame != null) {
          return;
        }
        const pointerId = resizeState.pointerId;
        const windowId = resizeState.id;
        const scheduledAt = performance.now();
        traceUi(UI_TRACE_EVENT.resizePointermoveFrameScheduled, {
          window_id: windowId,
          pointer_id: pointerId,
        });
        resizeState.applyFrame = requestAnimationFrame(() => {
          if (!resizeState || resizeState.pointerId !== pointerId) {
            return;
          }
          resizeState.applyFrame = null;
          traceUi(UI_TRACE_EVENT.resizePointermoveFrame, {
            window_id: windowId,
            pointer_id: pointerId,
            delay_ms: performance.now() - scheduledAt,
          });
          applyResizePointermove(resizeState);
          scheduleTerminalResizeFit(resizeState.id);
        });
      }

      function cancelResizePointermoveApply() {
        if (resizeState && resizeState.applyFrame != null) {
          cancelAnimationFrame(resizeState.applyFrame);
          resizeState.applyFrame = null;
        }
      }

      function applyResizePointermove(state) {
        if (!state) {
          return;
        }
        const element = windowMap.get(state.id);
        if (!element) {
          return;
        }
        const { clientX, clientY, width, height } = resizeGeometryFromPointerState(state, {
          zoom: viewport.zoom,
        });
        element.style.width = `${width}px`;
        element.style.height = `${height}px`;
        traceUi(UI_TRACE_EVENT.resizePointermoveApply, {
          window_id: state.id,
          pointer_id: state.pointerId,
          client_x: clientX,
          client_y: clientY,
          width,
          height,
        });
      }

      // SPEC-2014 Phase C1: maximum wall time (in ms) a single resize gesture
      // may stay active before we assume the pointer-end event was lost and
      // force a teardown. Long enough to cover slow Windows ConPTY resizes
      // (anecdotally a couple of seconds in the worst case) yet short enough
      // that the user does not have to restart the app when WebView2 drops
      // the pointerup.
      const RESIZE_STALENESS_TIMEOUT_MS = 30_000;

      function scheduleResizeStalenessGuard(pointerId) {
        return setTimeout(() => {
          if (!resizeState || resizeState.pointerId !== pointerId) {
            return;
          }
          const elapsed = Math.round(
            performance.now() - (resizeState.startedAt ?? performance.now()),
          );
          console.warn(
            `[resize] staleness guard fired after ${elapsed}ms; clearing stuck resizeState (pointerId=${pointerId})`,
          );
          // Trigger the normal teardown path so xterm fit / sendGeometry /
          // hover-reveal cleanup all run, then null the state.
          finishWindowResize(pointerId);
        }, RESIZE_STALENESS_TIMEOUT_MS);
      }

      function cancelResizeStalenessGuard() {
        if (resizeState && resizeState.stalenessTimer != null) {
          clearTimeout(resizeState.stalenessTimer);
          resizeState.stalenessTimer = null;
        }
      }

      function forceResetResizeState(reason) {
        if (!resizeState) {
          return;
        }
        const previous = resizeState;
        console.warn(
          `[resize] force-reset resizeState (reason=${reason}, previousPointerId=${previous.pointerId}, windowId=${previous.id})`,
        );
        if (previous.fitFrame != null) {
          cancelAnimationFrame(previous.fitFrame);
        }
        if (previous.applyFrame != null) {
          cancelAnimationFrame(previous.applyFrame);
        }
        if (previous.stalenessTimer != null) {
          clearTimeout(previous.stalenessTimer);
        }
        clearLocalGeometryEdit(geometrySyncState, previous.id);
        resizeState = null;
        delete document.documentElement.dataset.opResizeActive;
      }

      function scheduleTerminalViewportRefresh(windowId) {
        const runtime = terminalMap.get(windowId);
        if (!runtime) {
          return false;
        }
        if (!canRefreshTerminalViewport(windowId)) {
          markTerminalViewportRefreshPending(windowId);
          return false;
        }
        if (!terminalViewportRefreshScheduler) {
          requestAnimationFrame(() => {
            if (!canRefreshTerminalViewport(windowId)) {
              markTerminalViewportRefreshPending(windowId);
              return;
            }
            const activeRuntime = terminalMap.get(windowId);
            if (!activeRuntime) {
              return;
            }
            activeRuntime.viewportRefreshPending = false;
            refreshTerminalViewport(windowId);
          });
          return false;
        }
        return terminalViewportRefreshScheduler.enqueue(windowId);
      }

      function refreshTerminalViewport(windowId) {
        const runtime = terminalMap.get(windowId);
        if (!runtime || !canRefreshTerminalViewport(windowId)) {
          return;
        }
        runtime.terminal.refresh(0, runtime.terminal.rows - 1);
      }

      function markTerminalViewportRefreshPending(windowId) {
        const runtime = terminalMap.get(windowId);
        if (runtime) {
          runtime.viewportRefreshPending = true;
        }
      }

      terminalViewportRefreshScheduler = createTerminalViewportRefreshScheduler({
        canRefresh: canRefreshTerminalViewport,
        refresh: (windowId) => {
          const runtime = terminalMap.get(windowId);
          if (!runtime) {
            return;
          }
          runtime.viewportRefreshPending = false;
          refreshTerminalViewport(windowId);
        },
        markPending: markTerminalViewportRefreshPending,
      });

      function forceTerminalViewportRefresh(windowId, { shouldPersistGeometry = true } = {}) {
        const runtime = terminalMap.get(windowId);
        if (!runtime) {
          return false;
        }
        if (!canRefreshTerminalViewport(windowId)) {
          runtime.viewportRefreshPending = true;
          return false;
        }
        const activation = runTerminalActivationSequence({
          runtime,
          windowId,
          shouldFocus: false,
          shouldPersistGeometry,
          sendGeometry,
        });
        if (!activation.ran) {
          runtime.viewportRefreshPending = true;
          scheduleTerminalFocusActivation(windowId, { shouldPersistGeometry });
          return false;
        }
        runtime.viewportRefreshPending = false;
        refreshTerminalViewport(windowId);
        return true;
      }

      function rearmPendingTerminalViewportRefresh(windowId) {
        const runtime = terminalMap.get(windowId);
        if (!runtime) {
          return false;
        }
        return rearmRefreshOnVisible({
          hasPendingRefresh: () => runtime.viewportRefreshPending === true,
          canRefresh: () => canRefreshTerminalViewport(windowId),
          clearPendingRefresh: () => {
            runtime.viewportRefreshPending = false;
          },
          scheduleRefresh: () => {
            forceTerminalViewportRefresh(windowId, { shouldPersistGeometry: true });
          },
        });
      }

      function rearmVisibleTerminalViewportRefreshes() {
        for (const windowId of terminalMap.keys()) {
          if (!canRefreshTerminalViewport(windowId)) {
            continue;
          }
          if (!rearmPendingTerminalViewportRefresh(windowId)) {
            scheduleTerminalViewportRefresh(windowId);
          }
        }
      }

      function scheduleTerminalFocusActivation(
        windowId,
        { shouldPersistGeometry = true } = {},
      ) {
        const runtime = terminalMap.get(windowId);
        if (!runtime || runtime.activationFrame !== null) {
          return;
        }
        runtime.activationFrame = requestAnimationFrame(() => {
          runtime.activationFrame = null;
          const activeRuntime = terminalMap.get(windowId);
          if (!activeRuntime || !canRefreshTerminalViewport(windowId)) {
            return;
          }
          // Issue #2704 — suppress only the trailing `terminal.focus()`
          // step when a modal is open or a text input owns focus, so the
          // Clone Project URL/Search field (and other modal inputs) keep
          // keyboard focus while the background terminal keeps streaming
          // `workspace_state` events. Geometry persistence is controlled
          // by the caller so routine workspace renders do not echo
          // backend resize broadcasts back into another workspace render.
          const shouldFocus = !shouldSkipTerminalFocusActivation({
            doc: document,
            modalElements: [
              modal,
              wizardModal,
              cloneProjectModal,
              branchCleanupModal,
              migrationModal,
            ],
          });
          // SPEC-2008 Phase 26.B / FR-056: render BEFORE fit so xterm's
          // cell metrics are populated by the time proposeDimensions
          // runs. The previous order (fit-then-refresh) silently no-op'd
          // whenever the terminal had been display:none — proposeDimensions
          // returns undefined when cell.width === 0, leaving the viewport
          // stuck on the pre-hidden cols/rows until the next OS resize.
          // Issue #2937: capture the activation result. When the focus
          // reflow can't resolve a real grid yet — e.g. a tab-group member
          // revealed before its flex/grid layout settles, so the container
          // still has a 0-size layout box — runTerminalActivationSequence
          // returns { ran: false } and leaves the PTY at its stale grid.
          // Mirror completeInitialFitHandshake's bounded rAF retry instead
          // of giving up after one frame, so the focus path is not a
          // one-shot silent no-op (#2832 parity for the focus trigger).
          const activation = runTerminalActivationSequence({
            runtime: activeRuntime,
            windowId,
            shouldFocus,
            shouldPersistGeometry,
            syncGeometryOnGridChange: true,
            sendGeometry,
          });
          if (!activation.ran) {
            activeRuntime.activationAttempts =
              (activeRuntime.activationAttempts || 0) + 1;
            if (activeRuntime.activationAttempts <= HANDSHAKE_RETRY_LIMIT) {
              scheduleTerminalFocusActivation(windowId, {
                shouldPersistGeometry,
              });
            }
            return;
          }
          activeRuntime.activationAttempts = 0;
          // SPEC-2008 Phase 26.A / FR-057: if the runtime was created in
          // a hidden state, its initial fit handshake never completed
          // (completeInitialFitHandshake bails when canRefreshTerminalViewport
          // is false). The hidden → visible transition is the first chance
          // to drain the pending buffers; complete the handshake here so
          // subsequent writeOutput calls stop hitting deferredWrites.
          if (activeRuntime.isReady === false) {
            completeInitialFitHandshake(windowId);
          }
          // Schedule one more viewport refresh on the next frame so the
          // post-fit cols/rows are reflected in the rendered buffer even
          // when xterm coalesces internal redraws. This keeps the prior
          // refresh re-arm behavior active for repeated activations while
          // routing routine refresh work through the shared scheduler.
          scheduleTerminalViewportRefresh(windowId);
        });
      }

      function sendGeometry(
        windowId,
        cols,
        rows,
        baseGeometryRevision = localGeometryBaseRevision(
          geometrySyncState,
          windowId,
          workspaceWindowById(windowId),
        ),
      ) {
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
          base_geometry_revision: baseGeometryRevision,
        });
      }

      // SPEC-2356 — Living Telemetry counters in the Operator Status Strip.
      // Aggregates `data-agent-state` across all open windows and pushes the
      // counts into the bottom strip. We also expose agent count to the
      // sidebar layer for the "Quick" section's hint.
      function operatorTelemetryRenderKey(counts) {
        const parts = [];
        appendRenderKeyPart(parts, "active");
        appendRenderKeyPart(parts, counts?.active ?? null);
        appendRenderKeyPart(parts, "idle");
        appendRenderKeyPart(parts, counts?.idle ?? null);
        appendRenderKeyPart(parts, "blocked");
        appendRenderKeyPart(parts, counts?.blocked ?? null);
        appendRenderKeyPart(parts, "done");
        appendRenderKeyPart(parts, counts?.done ?? null);
        appendRenderKeyPart(parts, "agents");
        appendRenderKeyPart(parts, counts?.agents ?? null);
        appendRenderKeyPart(parts, "branches");
        appendRenderKeyPart(parts, counts?.branches ?? null);
        appendRenderKeyPart(parts, "git");
        appendRenderKeyPart(parts, counts?.git ?? null);
        appendRenderKeyPart(parts, "hooks");
        appendRenderKeyPart(parts, counts?.hooks ?? null);
        appendRenderKeyPart(parts, "layers");
        appendRenderKeyPart(parts, counts?.layers ?? null);
        return parts.join("");
      }

      function applyOperatorTelemetryCounts(counts) {
        if (!window.__operatorShell?.applyTelemetryCounts) return;
        const nextOperatorTelemetryKey = operatorTelemetryRenderKey(counts);
        if (renderedOperatorTelemetryKey === nextOperatorTelemetryKey) {
          return;
        }
        try {
          window.__operatorShell.applyTelemetryCounts(counts);
          renderedOperatorTelemetryKey = nextOperatorTelemetryKey;
        } catch (e) {
          console.warn("operator telemetry update failed", e);
        }
      }

      function recomputeOperatorTelemetry() {
        if (!window.__operatorShell?.applyTelemetryCounts) return;
        const counts = { active: 0, idle: 0, blocked: 0, done: 0, agents: 0 };
        for (const [windowId, el] of windowMap.entries()) {
          const state = el?.dataset?.agentState;
          if (!state) continue;
          // SPEC-2356 follow-up: only count live agent panes. Other workspace
          // windows (Board / Workspace / Logs / Branches / etc.) carry
          // data-agent-state for overlay/animation purposes but must not
          // inflate the Sidebar Layers Agents row or Status Strip cells.
          const windowData = workspaceWindowById(windowId);
          if (!windowData || !presetSupportsWaitingStatus(windowData.preset)) continue;
          if (state in counts) counts[state] += 1;
          counts.agents += 1;
        }
        if (activeWorkProjection) {
          const activeWorks = activeWorkItemsFromProjection();
          if (activeWorks.length > 0) {
            const activeAgents = activeWorks.reduce(
              (total, work) => total + Number(work.active_agents || 0),
              0,
            );
            const blockedAgents = activeWorks.reduce(
              (total, work) => total + Number(work.blocked_agents || 0),
              0,
            );
            const totalAgents = activeWorks.reduce(
              (total, work) => total + (Array.isArray(work.agents) ? work.agents.length : 0),
              0,
            );
            counts.active = Math.max(counts.active, activeAgents || activeWorks.length);
            counts.blocked = Math.max(counts.blocked, blockedAgents);
            counts.agents = Math.max(counts.agents, totalAgents || activeAgents + blockedAgents);
            counts.branches = Math.max(Number(counts.branches || 0), activeWorks.length);
          } else {
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
        }
        applyOperatorTelemetryCounts(counts);
      }

      // ---- Provider usage & rate limits (SPEC-2970) ----
      let latestProviderUsage = { accounts: [], sessions: [], consumption: [] };

      const USAGE_PROVIDER_NAME = { codex: "Codex", claude_code: "Claude Code" };
      const USAGE_WINDOW_LABEL = {
        five_hour: "5-hour",
        weekly: "Weekly",
        opus_weekly: "Opus weekly",
        sonnet_weekly: "Sonnet weekly",
        code_review_weekly: "Code review weekly",
      };

      function usageStateReason(state) {
        if (!state) return "";
        switch (state.kind) {
          case "disabled":
            return "Enable in Settings";
          case "no_data":
            return "No data yet";
          case "unavailable":
            return state.reason ? `Unavailable — ${state.reason}` : "Unavailable";
          case "stale":
            return `stale ${Math.round((state.age_secs || 0) / 60)}m`;
          default:
            return "";
        }
      }

      function usageFmtResetAt(iso) {
        if (!iso) return "";
        const d = new Date(iso);
        if (Number.isNaN(d.getTime())) return "";
        return d.toLocaleString();
      }

      function usageFmtTokens(n) {
        if (n == null) return "—";
        if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`;
        if (n >= 1000) return `${Math.round(n / 1000)}k`;
        return String(n);
      }

      function applyProviderUsageUi(snapshot) {
        latestProviderUsage = snapshot || { accounts: [], sessions: [], consumption: [] };
        try {
          window.__operatorShell?.applyProviderUsage?.(latestProviderUsage);
        } catch (e) {
          console.warn("usage pill update failed", e);
        }
        try {
          refreshUsageHoverIfOpen();
        } catch {
          /* no-op */
        }
        // Re-render regardless of session count: when a snapshot drops back to
        // sessions:[] (agent stopped, rollout/transcript unreadable, settings
        // change) the Work surface must clear its stale token/context instead
        // of keeping the previous poll's values. SPEC-2359 Phase W-12 Slice 3
        // (FR-351): the sidebar Active Works overview is gone, so usage now
        // refreshes through the Workspace Overview (Kanban) Work surface.
        try {
          workspaceOverviewSurface.renderWindows();
        } catch {
          /* no-op */
        }
      }

      function usageForSession(sessionId) {
        return (
          (latestProviderUsage.sessions || []).find(
            (s) => s.session_id === sessionId,
          ) || null
        );
      }

      function buildUsageBar(percent) {
        const wrap = document.createElement("div");
        wrap.className = "op-usage-bar";
        const fill = document.createElement("div");
        fill.className = "op-usage-bar__fill";
        const pct = Math.max(0, Math.min(100, Math.round(percent)));
        fill.style.width = `${pct}%`;
        if (pct >= 90) fill.dataset.level = "high";
        else if (pct >= 70) fill.dataset.level = "mid";
        wrap.appendChild(fill);
        return wrap;
      }

      function renderUsageAccountRow(account) {
        const row = document.createElement("div");
        row.className = "op-usage-account";
        row.dataset.provider = account.provider;
        const head = document.createElement("div");
        head.className = "op-usage-account__head";
        const name = document.createElement("span");
        name.className = "op-usage-account__name";
        name.textContent = USAGE_PROVIDER_NAME[account.provider] || account.provider;
        head.appendChild(name);
        if (account.plan) {
          const plan = document.createElement("span");
          plan.className = "op-usage-account__plan";
          plan.textContent = account.plan;
          head.appendChild(plan);
        }
        const reason = usageStateReason(account.state);
        if (reason) {
          const isDisabled = (account.state && account.state.kind) === "disabled";
          const r = document.createElement(isDisabled ? "button" : "span");
          r.className = "op-usage-account__reason";
          r.textContent = reason;
          if (isDisabled) {
            r.type = "button";
            r.classList.add("op-usage-account__reason--action");
            r.addEventListener("click", (e) => {
              e.stopPropagation();
              if (typeof window.__gwtHideUsageHover === "function") {
                window.__gwtHideUsageHover();
              }
              document.dispatchEvent(
                new CustomEvent("settings:open", { detail: { target: "usage" } }),
              );
            });
          }
          head.appendChild(r);
        }
        row.appendChild(head);
        for (const w of account.windows || []) {
          const line = document.createElement("div");
          line.className = "op-usage-window";
          const label = document.createElement("span");
          label.className = "op-usage-window__label";
          label.textContent = USAGE_WINDOW_LABEL[w.kind] || w.kind;
          const pct = document.createElement("span");
          pct.className = "op-usage-window__pct";
          pct.textContent = `${Math.round(w.used_percent)}%`;
          line.appendChild(label);
          line.appendChild(buildUsageBar(w.used_percent));
          line.appendChild(pct);
          if (w.resets_at) {
            const reset = document.createElement("span");
            reset.className = "op-usage-window__reset";
            reset.textContent = `↻ ${usageFmtResetAt(w.resets_at)}`;
            line.appendChild(reset);
          }
          row.appendChild(line);
        }
        return row;
      }

      function consumptionTotal(b) {
        if (!b) return 0;
        return (b.input || 0) + (b.output || 0) + (b.cached || 0);
      }

      function fmtConsumptionBreakdown(b) {
        if (!b) return "—";
        return `in ${usageFmtTokens(b.input || 0)} · out ${usageFmtTokens(
          b.output || 0,
        )} · cached ${usageFmtTokens(b.cached || 0)}`;
      }

      function renderConsumptionChart(days) {
        const chart = document.createElement("div");
        chart.className = "op-usage-chart";
        const totals = days.map((d) => consumptionTotal(d.breakdown));
        const max = Math.max(1, ...totals);
        days.forEach((d, i) => {
          const col = document.createElement("div");
          col.className = "op-usage-chart__col";
          if (i === days.length - 1) col.dataset.today = "true";
          const bar = document.createElement("div");
          bar.className = "op-usage-chart__bar";
          const total = totals[i];
          bar.style.height = `${Math.max(2, Math.round((total / max) * 100))}%`;
          bar.title = `${d.date}: ${usageFmtTokens(total)} tokens`;
          col.appendChild(bar);
          chart.appendChild(col);
        });
        return chart;
      }

      function usageFmtResetShort(iso) {
        if (!iso) return "";
        const d = new Date(iso);
        if (Number.isNaN(d.getTime())) return "";
        return d.toLocaleString(undefined, {
          month: "numeric",
          day: "numeric",
          hour: "2-digit",
          minute: "2-digit",
        });
      }

      function usageConsumptionFor(provider) {
        return (
          (latestProviderUsage.consumption || []).find((c) => c.provider === provider) || null
        );
      }

      // One rate-limit window as an aligned row: label · bar · % · reset.
      function buildUsageWindowRow(w) {
        const row = document.createElement("div");
        row.className = "op-usage-win";
        const lbl = document.createElement("span");
        lbl.className = "op-usage-win__lbl";
        lbl.textContent = USAGE_WINDOW_LABEL[w.kind] || w.kind;
        const bar = buildUsageBar(w.used_percent);
        bar.classList.add("op-usage-win__bar");
        const pct = document.createElement("span");
        pct.className = "op-usage-win__pct";
        pct.textContent = `${Math.round(w.used_percent)}%`;
        const reset = document.createElement("span");
        reset.className = "op-usage-win__reset";
        reset.textContent = w.resets_at ? `↻ ${usageFmtResetShort(w.resets_at)}` : "";
        row.appendChild(lbl);
        row.appendChild(bar);
        row.appendChild(pct);
        row.appendChild(reset);
        return row;
      }

      // Consumption as an aligned 4-column grid (period × in/out/cached).
      function buildUsageConsumptionGrid(pc) {
        const grid = document.createElement("div");
        grid.className = "op-usage-cgrid";
        const t = pc.today || {};
        const w = pc.this_week || {};
        const cells = [
          ["hdr", "tokens"],
          ["colh", "in"],
          ["colh", "out"],
          ["colh", "cached"],
          ["rowh", "Today"],
          ["num", usageFmtTokens(t.input || 0)],
          ["num", usageFmtTokens(t.output || 0)],
          ["num", usageFmtTokens(t.cached || 0)],
          ["rowh", "Week"],
          ["num", usageFmtTokens(w.input || 0)],
          ["num", usageFmtTokens(w.output || 0)],
          ["num", usageFmtTokens(w.cached || 0)],
        ];
        for (const [kind, text] of cells) {
          const cell = document.createElement("span");
          cell.className = `op-usage-cgrid__${kind}`;
          cell.textContent = text;
          grid.appendChild(cell);
        }
        return grid;
      }

      // A provider card: header (icon · name · plan) + rate-limit windows (or a
      // degraded reason) + consumption grid + 7-day sparkline. Grouping all of
      // one provider's data together is the key readability win.
      function buildUsageProviderCard(account) {
        const card = document.createElement("div");
        card.className = "op-usage-card";
        card.dataset.provider = account.provider;

        const head = document.createElement("div");
        head.className = "op-usage-card__head";
        const icon = document.createElement("span");
        icon.className = "op-usage-card__icon";
        icon.textContent = account.provider === "claude_code" ? "◇" : "⬡";
        const name = document.createElement("span");
        name.className = "op-usage-card__name";
        name.textContent = USAGE_PROVIDER_NAME[account.provider] || account.provider;
        head.appendChild(icon);
        head.appendChild(name);
        if (account.plan) {
          const plan = document.createElement("span");
          plan.className = "op-usage-card__plan";
          plan.textContent = account.plan;
          head.appendChild(plan);
        }
        card.appendChild(head);

        const windows = account.windows || [];
        if (windows.length) {
          const wins = document.createElement("div");
          wins.className = "op-usage-wins";
          for (const w of windows) wins.appendChild(buildUsageWindowRow(w));
          card.appendChild(wins);
        } else {
          const reason = usageStateReason(account.state);
          if (reason) {
            const isDisabled = (account.state && account.state.kind) === "disabled";
            const r = document.createElement(isDisabled ? "button" : "div");
            r.className = "op-usage-card__reason";
            r.textContent = reason;
            if (isDisabled) {
              r.type = "button";
              r.classList.add("op-usage-card__reason--action");
              r.addEventListener("click", (e) => {
                e.stopPropagation();
                if (typeof window.__gwtHideUsageHover === "function") {
                  window.__gwtHideUsageHover();
                }
                document.dispatchEvent(
                  new CustomEvent("settings:open", { detail: { target: "usage" } }),
                );
              });
            }
            card.appendChild(r);
          }
        }

        const pc = usageConsumptionFor(account.provider);
        if (pc) {
          const cwrap = document.createElement("div");
          cwrap.className = "op-usage-card__cons";
          cwrap.appendChild(buildUsageConsumptionGrid(pc));
          if (Array.isArray(pc.days) && pc.days.length) {
            cwrap.appendChild(renderConsumptionChart(pc.days));
          }
          card.appendChild(cwrap);
        }
        return card;
      }

      // SPEC-2970 — the full usage detail as provider cards, appended to a
      // container. The hover popover is the single surface for all usage info
      // (the click-open modal was removed per UX feedback). The per-session
      // list was removed per UX feedback — it grew to hundreds of rows and
      // overwhelmed the popover; per-session token usage still drives each
      // agent's inline footer (see usageForSession).
      function buildUsageFullSections(container) {
        for (const account of latestProviderUsage.accounts || []) {
          container.appendChild(buildUsageProviderCard(account));
        }
      }

      // ---- Consolidated usage hover popover (SPEC-2970 UX) ----
      // Hovering the status-bar USAGE cell shows EVERYTHING at once (both
      // providers' windows with bars + full consumption with charts +
      // sessions). The click-open modal was removed per UX feedback — the hover
      // popover is the single surface. Move the cursor into it to scroll/read.
      let usageHoverEl = null;
      let usageHoverHideTimer = null;
      let usageHoverAnchor = null;

      function buildUsageHoverBody() {
        const wrap = document.createElement("div");
        wrap.className = "op-usage-hover__body";
        const head = document.createElement("div");
        head.className = "op-usage-hover__head";
        head.textContent = "Usage & Limits";
        wrap.appendChild(head);
        buildUsageFullSections(wrap);
        return wrap;
      }

      function positionUsageHover() {
        if (!usageHoverEl || !usageHoverAnchor) return;
        const r = usageHoverAnchor.getBoundingClientRect();
        const w = usageHoverEl.offsetWidth;
        const left = Math.max(8, Math.min(r.left, window.innerWidth - w - 8));
        usageHoverEl.style.left = `${left}px`;
        usageHoverEl.style.bottom = `${Math.max(8, window.innerHeight - r.top + 6)}px`;
      }

      function cancelUsageHoverHide() {
        if (usageHoverHideTimer) {
          clearTimeout(usageHoverHideTimer);
          usageHoverHideTimer = null;
        }
      }

      function refreshUsageHoverIfOpen() {
        if (!usageHoverEl || usageHoverEl.hidden) return;
        while (usageHoverEl.firstChild) usageHoverEl.removeChild(usageHoverEl.firstChild);
        usageHoverEl.appendChild(buildUsageHoverBody());
        positionUsageHover();
      }

      window.__gwtShowUsageHover = (anchor) => {
        cancelUsageHoverHide();
        usageHoverAnchor = anchor || usageHoverAnchor;
        if (!usageHoverEl) {
          usageHoverEl = document.createElement("div");
          usageHoverEl.className = "op-usage-hover";
          usageHoverEl.addEventListener("mouseenter", cancelUsageHoverHide);
          usageHoverEl.addEventListener("mouseleave", () => window.__gwtHideUsageHover());
          document.body.appendChild(usageHoverEl);
        }
        while (usageHoverEl.firstChild) usageHoverEl.removeChild(usageHoverEl.firstChild);
        usageHoverEl.appendChild(buildUsageHoverBody());
        usageHoverEl.hidden = false;
        usageHoverEl.style.visibility = "hidden";
        requestAnimationFrame(() => {
          positionUsageHover();
          if (usageHoverEl) usageHoverEl.style.visibility = "visible";
        });
      };

      window.__gwtHideUsageHover = () => {
        cancelUsageHoverHide();
        usageHoverHideTimer = setTimeout(() => {
          if (usageHoverEl) usageHoverEl.hidden = true;
          usageHoverHideTimer = null;
        }, 180);
      };

      // SPEC-2970 FR-009/FR-013 — Settings "Usage & Limits" panel: Claude
      // account usage is opt-in (Keychain + network); Codex is local + auto.
      function renderUsagePanel(panel) {
        while (panel.firstChild) panel.removeChild(panel.firstChild);
        const section = document.createElement("div");
        section.className = "settings-section";

        const heading = document.createElement("h3");
        heading.textContent = "Provider Usage & Limits";
        section.appendChild(heading);

        const codexNote = document.createElement("p");
        codexNote.className = "settings-hint";
        codexNote.textContent =
          "Codex usage is read from local session files automatically.";
        section.appendChild(codexNote);

        const label = document.createElement("label");
        label.className = "settings-toggle";
        const checkbox = document.createElement("input");
        checkbox.type = "checkbox";
        const claudeAccount = (latestProviderUsage.accounts || []).find(
          (a) => a.provider === "claude_code",
        );
        checkbox.checked = !!(
          claudeAccount &&
          claudeAccount.state &&
          claudeAccount.state.kind !== "disabled"
        );
        checkbox.addEventListener("change", () => {
          try {
            send({
              kind: "set_claude_account_usage_enabled",
              enabled: checkbox.checked,
            });
          } catch {
            /* no-op */
          }
        });
        const span = document.createElement("span");
        span.textContent = "Show Claude Code account usage (5-hour / weekly)";
        label.appendChild(checkbox);
        label.appendChild(span);
        section.appendChild(label);

        const consent = document.createElement("p");
        consent.className = "settings-hint";
        consent.textContent =
          "Off by default (opt-in). When enabled, Claude account usage reads your OAuth token from the Keychain / credentials file and requests usage from the Anthropic API (polled at most once every 3 minutes). While disabled, no Keychain read or network request happens. Per-session token usage is read locally and is not affected by this setting.";
        section.appendChild(consent);

        panel.appendChild(section);
      }

      function activeWorkFocusableAgents(work) {
        const agents = Array.isArray(work?.agents) ? work.agents : [];
        return agents.filter((agent) => {
          if (!agent?.window_id) return false;
          const windowData = workspaceWindowById(agent.window_id);
          if (!windowData || !presetSupportsWaitingStatus(windowData.preset)) return false;
          const status = runtimeStateForWindow(windowData);
          return status !== "stopped" && status !== "exited" && status !== "error";
        });
      }

      function activeWorkItemsFromProjection() {
        if (!activeWorkProjection) return [];
        const source = Array.isArray(activeWorkProjection.active_works)
          ? activeWorkProjection.active_works
          : [];
        if (source.length > 0) {
          return source
            .map((work) => ({ ...work, agents: activeWorkFocusableAgents(work) }))
            .filter((work) => work.agents.length > 0);
        }
        const agents = activeWorkFocusableAgents(activeWorkProjection);
        if (agents.length === 0) return [];
        return [{ ...activeWorkProjection, agents }];
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

      function createWorkspacePrMeta(projection) {
        if (!projection?.pr_number) return null;
        const item = createNode("span", "workspace-pr-meta");
        const label = `PR #${projection.pr_number}`;
        if (projection.pr_url) {
          const link = createNode("a", "workspace-pr-link", label);
          link.href = projection.pr_url;
          link.target = "_blank";
          link.rel = "noopener noreferrer";
          item.appendChild(link);
        } else {
          item.appendChild(createNode("span", "", label));
        }
        appendMeta(item, projection.pr_state);
        return item;
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

      function applyStatus(windowId, status, detail) {
        return traceMeasure(
          UI_TRACE_EVENT.applyStatus,
          { window_id: windowId, status },
          () => {
            const windowData = workspaceWindowById(windowId);
            const runtimeState = normalizeWindowRuntimeState(status, windowData?.preset);
            windowRuntimeStateMap.set(windowId, runtimeState);
            if (detail) {
              detailMap.set(windowId, detail);
            } else if (
              runtimeState === "running" ||
              runtimeState === "starting" ||
              runtimeState === "idle" ||
              runtimeState === "waiting"
            ) {
              detailMap.delete(windowId);
            }
            const effectiveDetail = detailMap.get(windowId) || "";
            const nextRuntimeStatusKey = windowRuntimeStatusRenderKey(
              windowId,
              runtimeState,
              effectiveDetail,
              windowData,
            );
            if (renderedRuntimeStatusKeys.get(windowId) === nextRuntimeStatusKey) {
              return;
            }
            renderedRuntimeStatusKeys.set(windowId, nextRuntimeStatusKey);
            const element = windowMap.get(windowId);
            if (!element) {
              renderWindowList();
              refreshProjectTabDots();
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
              "not_started",
              "ready",
              "idle",
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
            const statusTitle = effectiveDetail
              ? `${windowRuntimeLabel(runtimeState)}: ${effectiveDetail}`
              : windowRuntimeLabel(runtimeState);
            chip.title = statusTitle;
            label.title = statusTitle;
            if (overlay) {
              const messageEl = overlay.querySelector(".overlay-message");
              if (messageEl) {
                messageEl.textContent = effectiveDetail || "";
              } else {
                overlay.textContent = effectiveDetail || "";
              }
              updateTerminalOverlayCopyState(overlay);
              const shouldShowOverlay = false;
              const shouldSpin = false;
              const spinner = overlay.querySelector(".overlay-spinner");
              if (spinner) {
                spinner.hidden = !shouldSpin;
              }
              overlay.classList.toggle("visible", shouldShowOverlay);
              if (shouldSpin) {
                startSpinnerAnimation(overlay);
              } else {
                stopSpinnerAnimation(overlay);
              }
            }
            renderWindowList();
            refreshProjectTabDots();
          },
        );
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
        const targetElement = windowMap.get(windowId);
        if (focusedId === windowId && targetElement?.classList.contains("focused")) {
          return;
        }
        const previousFocusedId = focusedId;
        focusedId = windowId;
        if (previousFocusedId && previousFocusedId !== windowId) {
          const previousElement = windowMap.get(previousFocusedId);
          previousElement?.classList.remove("focused");
        }
        if (targetElement) {
          targetElement.classList.add("focused");
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
          // Final maximized geometry (zoom-corrected screen inset). The backend
          // stores it as-is; see maximizedGeometry in window-geometry-sync.js.
          bounds: maximizedGeometry(visibleBounds(), viewport.zoom),
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
          // Native tooltip with the full window title (or dynamic detail) so a
          // tab truncated by max-width still reveals its title on hover. Mirrors
          // the titlebar (titleText.title) and window-list row tooltips.
          tabButton.title = windowTitleTooltip(tab);
          tabButton.addEventListener("click", (event) => {
            event.stopPropagation();
            send({ kind: "activate_window_tab", id: tab.id });
          });
          tabButton.addEventListener("dragstart", (event) => {
            windowTabDragState = {
              id: tab.id,
              docked: false,
              lastClientPoint: clientPointFromDragEvent(
                event,
                canvas.getBoundingClientRect(),
              ),
            };
            event.dataTransfer?.setData("text/plain", tab.id);
            if (event.dataTransfer) {
              event.dataTransfer.effectAllowed = "move";
            }
          });
          tabButton.addEventListener("drag", trackWindowTabDragPoint);
          tabButton.addEventListener("dragend", (event) => {
            const drag = windowTabDragState;
            trackWindowTabDragPoint(event);
            windowTabDragState = null;
            if (!drag || drag.docked) return;
            const draggedWindow = workspaceWindowById(drag.id);
            if (!draggedWindow?.tab_group_id) return;
            const geometry = detachGeometryFromTabDrag(event, drag, draggedWindow);
            if (!geometry) return;
            send({
              kind: "detach_window_tab",
              id: drag.id,
              geometry,
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

      function isBlinkBrowser() {
        const ua = navigator.userAgent || "";
        return /Chrome\//.test(ua);
      }

      const SUPPORTED_IMAGE_PASTE_MIME_TYPES = new Set([
        "image/png",
        "image/jpeg",
        "image/webp",
      ]);
      const MAX_FILE_DROP_COUNT = 16;
      const FILE_DROP_AGENT_TARGET_MESSAGE = "Drop files on a running Agent window.";
      const FILE_DROP_COUNT_MESSAGE = `Drop up to ${MAX_FILE_DROP_COUNT} files at once.`;
      const FILE_DROP_UPLOAD_FAILURE_MESSAGE = "Could not upload dropped file.";
      const IMAGE_PASTE_UPLOAD_FAILURE_MESSAGE = "Could not upload pasted image.";

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

      function defaultImagePasteFilename(mimeType) {
        switch (mimeType) {
          case "image/jpeg":
            return "clipboard-image.jpg";
          case "image/webp":
            return "clipboard-image.webp";
          case "image/png":
          default:
            return "clipboard-image.png";
        }
      }

      function uploadFileFromImageBlob(blob, mimeType, filename) {
        const type = mimeType || blob?.type || "";
        const name = filename || blob?.name || defaultImagePasteFilename(type);
        if (typeof File === "function" && (!(blob instanceof File) || !blob.name)) {
          return new File([blob], name, { type });
        }
        return blob;
      }

      function dataTransferHasFiles(dataTransfer) {
        const types = Array.from(dataTransfer?.types || []);
        return types.includes("Files") || Boolean(dataTransfer?.files?.length);
      }

      function droppedFilesWithinCountLimit(files) {
        return files.length <= MAX_FILE_DROP_COUNT;
      }

      function droppedFilesValidationFailure(files) {
        if (!droppedFilesWithinCountLimit(files)) {
          return FILE_DROP_COUNT_MESSAGE;
        }
        return null;
      }

      function showFileDropAlert(message) {
        if (typeof window.alert === "function") {
          window.alert(message);
        }
      }

      function totalFileBytes(files) {
        return files.reduce((total, file) => total + (file.size || 0), 0);
      }

      function displayAttachmentBasename(filename) {
        const value = String(filename || "").trim();
        const parts = value.split(/[\\/]+/).filter(Boolean);
        return parts.at(-1) || "file";
      }

      function attachmentFileCountLabel(count) {
        return `${count} ${count === 1 ? "file" : "files"}`;
      }

      function createAttachmentOperationId() {
        const random =
          typeof globalThis.crypto?.randomUUID === "function"
            ? globalThis.crypto.randomUUID()
            : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
        return `attachment-${random}`;
      }

      const attachmentProgressControllers = new Map();

      function ensureAttachmentProgressSurface(windowId) {
        const host = workspaceWindowElement(windowId) || document.body;
        let surface = host.querySelector(".attachment-progress");
        if (surface) {
          return surface;
        }
        surface = document.createElement("div");
        surface.className = "attachment-progress";
        surface.hidden = true;
        surface.setAttribute("role", "status");
        surface.setAttribute("aria-live", "polite");
        surface.innerHTML = `
          <div class="attachment-progress__row">
            <span class="attachment-progress__label"></span>
          </div>
          <div class="attachment-progress__track" role="progressbar" aria-valuemin="0" aria-valuemax="100">
            <div class="attachment-progress__bar"></div>
          </div>
        `;
        host.appendChild(surface);
        return surface;
      }

      function attachmentPhaseLabel(phase, fallback = "") {
        switch (phase) {
          case "queued":
            return "Queued";
          case "staging":
            return "Staging";
          case "injecting":
            return "Injecting";
          case "attached":
            return "Attached";
          case "failed":
            return fallback || "Could not attach";
          default:
            return fallback || "Uploading";
        }
      }

      function createAttachmentProgressController(windowId, files, operationId = createAttachmentOperationId()) {
        const surface = ensureAttachmentProgressSurface(windowId);
        const label = surface.querySelector(".attachment-progress__label");
        const track = surface.querySelector(".attachment-progress__track");
        const bar = surface.querySelector(".attachment-progress__bar");
        const abortController = new AbortController();
        const fileCount = files.length;
        const state = {
          phase: "Uploading",
          filename: fileCount === 1 ? displayAttachmentBasename(files[0]?.name) : "",
          totalBytes: totalFileBytes(files),
          loadedBytes: 0,
          failed: false,
          done: false,
          visible: true,
        };

        function percent() {
          if (state.totalBytes <= 0) {
            return state.done ? 100 : 0;
          }
          return Math.max(
            0,
            Math.min(100, Math.round((state.loadedBytes / state.totalBytes) * 100)),
          );
        }

        function render() {
          if (!state.visible) {
            return;
          }
          const value = percent();
          const filename = state.filename ? ` · ${state.filename}` : "";
          const suffix = state.totalBytes > 0 ? ` · ${value}%` : "";
          surface.hidden = false;
          surface.dataset.state = state.failed ? "error" : state.done ? "done" : "active";
          label.textContent = `${state.phase} ${attachmentFileCountLabel(fileCount)}${filename}${suffix}`;
          track.setAttribute("aria-valuenow", String(value));
          bar.style.width = `${value}%`;
        }

        function showNow() {
          state.visible = true;
          render();
        }

        render();

        function setPhase(phase) {
          state.phase = phase;
          render();
        }

        function setUploadProgress(loadedBytes, totalBytes = null) {
          if (Number.isFinite(totalBytes) && totalBytes >= 0) {
            state.totalBytes = totalBytes;
          }
          state.loadedBytes = Math.max(0, loadedBytes || 0);
          render();
        }

        function succeed() {
          state.done = true;
          state.phase = "Attached";
          state.loadedBytes = state.totalBytes;
          if (state.visible) {
            render();
            setTimeout(() => {
              if (surface.dataset.state === "done") {
                surface.hidden = true;
                attachmentProgressControllers.delete(operationId);
              }
            }, 700);
          }
        }

        function fail(message) {
          state.failed = true;
          state.phase = message;
          showNow();
        }

        function applyBackendProgress(event) {
          if (event.filename) {
            state.filename = displayAttachmentBasename(event.filename);
          }
          if (Number.isFinite(event.bytes_total)) {
            state.totalBytes = event.bytes_total;
          }
          if (Number.isFinite(event.bytes_done)) {
            state.loadedBytes = event.bytes_done;
          }
          if (event.phase === "failed") {
            fail(event.message || "Could not attach");
            return;
          }
          if (event.phase === "attached") {
            succeed();
            return;
          }
          setPhase(attachmentPhaseLabel(event.phase));
        }

        const controller = {
          operationId,
          signal: abortController.signal,
          setPhase,
          setUploadProgress,
          succeed,
          fail,
          applyBackendProgress,
        };
        attachmentProgressControllers.set(operationId, controller);
        return controller;
      }

      function handleAttachmentProgress(event) {
        const operationId = event?.operation_id || "";
        if (!operationId) {
          return;
        }
        let controller = attachmentProgressControllers.get(operationId);
        if (!controller) {
          controller = createAttachmentProgressController(
            event.id,
            [
              {
                name: event.filename || "file",
                size: event.bytes_total || 0,
              },
            ],
            operationId,
          );
        }
        controller.applyBackendProgress(event);
      }

      function attachmentFilesFromNativePaths(paths) {
        return paths.map((path) => ({
          name: displayAttachmentBasename(path),
          size: 0,
        }));
      }

      let attachmentUploadTokenPromise = null;

      async function attachmentUploadToken() {
        if (!attachmentUploadTokenPromise) {
          attachmentUploadTokenPromise = fetch("/internal/attachment-upload-token", {
            credentials: "same-origin",
          }).then(async (response) => {
            if (!response.ok) {
              throw new Error(`attachment upload token failed: ${response.status}`);
            }
            const payload = await response.json();
            if (!payload?.token) {
              throw new Error("attachment upload token missing");
            }
            return payload.token;
          });
        }
        return attachmentUploadTokenPromise;
      }

      function uploadAttachmentFile(file, { onProgress, signal } = {}) {
        if (typeof window.__gwtAttachmentUploader === "function") {
          return window.__gwtAttachmentUploader({ file, onProgress, signal });
        }
        return new Promise((resolve, reject) => {
          void attachmentUploadToken()
            .then((token) => {
              if (signal?.aborted) {
                reject(new Error("upload aborted"));
                return;
              }
              const params = new URLSearchParams({
                filename: file.name || "file",
                size: String(file.size || 0),
              });
              if (file.type) {
                params.set("mime_type", file.type);
              }
              const xhr = new XMLHttpRequest();
              xhr.open("POST", `/internal/attachments/upload?${params.toString()}`);
              xhr.setRequestHeader("x-gwt-upload-token", token);
              xhr.responseType = "json";
              xhr.upload.onprogress = (event) => {
                onProgress?.({
                  loaded: event.loaded || 0,
                  total: event.lengthComputable ? event.total : file.size || 0,
                });
              };
              xhr.onload = () => {
                if (xhr.status < 200 || xhr.status >= 300) {
                  reject(new Error(`upload failed: ${xhr.status}`));
                  return;
                }
                resolve(xhr.response || JSON.parse(xhr.responseText || "{}"));
              };
              xhr.onerror = () => reject(new Error("upload failed"));
              xhr.onabort = () => reject(new Error("upload aborted"));
              signal?.addEventListener("abort", () => xhr.abort(), { once: true });
              xhr.send(file);
            })
            .catch(reject);
        });
      }

      async function uploadFilesAsAttachments(files, progress) {
        const totalBytes = totalFileBytes(files);
        let completedBytes = 0;
        const attachments = [];
        for (const file of files) {
          const uploaded = await uploadAttachmentFile(file, {
            signal: progress.signal,
            onProgress: ({ loaded }) => {
              progress.setUploadProgress(completedBytes + (loaded || 0), totalBytes);
            },
          });
          completedBytes += file.size || uploaded?.size || 0;
          progress.setUploadProgress(
            totalBytes > 0 ? Math.min(completedBytes, totalBytes) : 0,
            totalBytes,
          );
          attachments.push({
            source: "uploaded",
            upload_id: uploaded.upload_id,
            filename: uploaded.filename || file.name || "file",
            mime_type: uploaded.mime_type ?? (file.type || null),
            size: uploaded.size ?? file.size ?? 0,
          });
        }
        return attachments;
      }

      async function uploadPastedImage(windowId, blob, { mimeType, filename } = {}) {
        const file = uploadFileFromImageBlob(blob, mimeType, filename);
        const progress = createAttachmentProgressController(windowId, [file]);
        try {
          const uploaded = await uploadAttachmentFile(file, {
            signal: progress.signal,
            onProgress: ({ loaded, total }) => progress.setUploadProgress(loaded || 0, total),
          });
          progress.setPhase("Queued");
          send({
            kind: "paste_image_uploaded",
            id: windowId,
            operation_id: progress.operationId,
            upload_id: uploaded.upload_id,
            mime_type: uploaded.mime_type ?? file.type ?? mimeType ?? "",
            filename: uploaded.filename || file.name || filename || null,
            size: uploaded.size ?? file.size ?? 0,
          });
        } catch (_error) {
          progress.fail(IMAGE_PASTE_UPLOAD_FAILURE_MESSAGE);
          showFileDropAlert(IMAGE_PASTE_UPLOAD_FAILURE_MESSAGE);
        }
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

      async function copyTerminalSelection(windowId, { clearSelectionAfterCopy = false } = {}) {
        const runtime = terminalMap.get(windowId);
        if (!runtime || !runtime.terminal.hasSelection()) {
          return false;
        }
        const selection = runtime.terminal.getSelection();
        if (!selection) {
          return false;
        }
        const copied = await writeClipboardText(selection, () => runtime.terminal.focus());
        if (copied && clearSelectionAfterCopy) {
          runtime.terminal.clearSelection();
        }
        return copied;
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
          const copyDecision = classifyTerminalCopyKeyEvent(event, {
            hasSelection: terminal.hasSelection(),
          });
          if (!copyDecision.copy) {
            return true;
          }
          event.preventDefault();
          event.stopPropagation();
          if (!terminal.hasSelection()) {
            return false;
          }
          void copyTerminalSelection(windowId, {
            clearSelectionAfterCopy: copyDecision.clearSelectionAfterCopy,
          });
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

          void uploadPastedImage(windowId, file, {
            mimeType: file.type || item.type,
            filename: file.name || null,
          }).finally(() => {
            terminal.focus();
          });
        };

        terminalRoot.addEventListener("paste", handlePaste, true);
        return () => {
          terminalRoot.removeEventListener("paste", handlePaste, true);
        };
      }

      function installTerminalFileDropHandlers(windowId, terminalRoot, terminal) {
        const handleDragOver = (event) => {
          if (!dataTransferHasFiles(event.dataTransfer)) {
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          event.dataTransfer.dropEffect = "copy";
        };

        const handleDrop = (event) => {
          const files = Array.from(event.dataTransfer?.files || []);
          if (files.length === 0) {
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          if (!isAgentWindowPreset(workspaceWindowById(windowId)?.preset)) {
            showFileDropAlert(FILE_DROP_AGENT_TARGET_MESSAGE);
            terminal.focus();
            return;
          }
          const failure = droppedFilesValidationFailure(files);
          if (failure) {
            showFileDropAlert(failure);
            terminal.focus();
            return;
          }

          void sendDroppedFileAttachments(windowId, files).finally(() => {
            terminal.focus();
          });
        };

        terminalRoot.addEventListener("dragover", handleDragOver, true);
        terminalRoot.addEventListener("drop", handleDrop, true);
        return () => {
          terminalRoot.removeEventListener("dragover", handleDragOver, true);
          terminalRoot.removeEventListener("drop", handleDrop, true);
        };
      }

      function installTerminalContextMenuHandlers(windowId, terminalRoot, terminal) {
        const controller = createTerminalContextMenuController({
          document,
          window,
          terminalRoot,
          readClipboardText: readNavigatorClipboardText,
          readClipboardItems: readNavigatorClipboardItems,
          supportedImageTypes: SUPPORTED_IMAGE_PASTE_MIME_TYPES,
          pasteText: (text) => terminal.paste(text),
          pasteImage: ({ blob, mimeType, filename }) =>
            uploadPastedImage(windowId, blob, { mimeType, filename }),
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

      function terminalWindowIdFromPoint(x, y) {
        if (!Number.isFinite(x) || !Number.isFinite(y)) {
          return null;
        }
        const target = document.elementFromPoint(x, y);
        const terminalRoot = target?.closest?.(".terminal-root");
        const windowElement = terminalRoot?.closest?.(".workspace-window");
        const windowId = windowElement?.dataset?.id || null;
        if (!windowId || !terminalMap.has(windowId)) {
          return null;
        }
        return windowId;
      }

      function workspaceWindowIdFromDropEvent(event) {
        const targetWindow = event.target?.closest?.(".workspace-window");
        const targetWindowId = targetWindow?.dataset?.id || null;
        if (targetWindowId) {
          return targetWindowId;
        }
        const x = Number(event.clientX);
        const y = Number(event.clientY);
        if (!Number.isFinite(x) || !Number.isFinite(y)) {
          return null;
        }
        const pointTarget = document.elementFromPoint(x, y);
        const pointWindow = pointTarget?.closest?.(".workspace-window");
        return pointWindow?.dataset?.id || null;
      }

      function agentWindowIdFromDropEvent(event) {
        const windowId = workspaceWindowIdFromDropEvent(event);
        if (!windowId || !terminalMap.has(windowId)) {
          return null;
        }
        const windowData = workspaceWindowById(windowId);
        if (!isAgentWindowPreset(windowData?.preset)) {
          return null;
        }
        return windowId;
      }

      function eventTargetsTerminalRoot(event) {
        return Boolean(event.target?.closest?.(".terminal-root"));
      }

      function focusTerminalForWindow(windowId) {
        terminalMap.get(windowId)?.terminal.focus();
      }

      async function sendDroppedFileAttachments(windowId, files) {
        const failure = droppedFilesValidationFailure(files);
        if (failure) {
          showFileDropAlert(failure);
          focusTerminalForWindow(windowId);
          return;
        }

        const progress = createAttachmentProgressController(windowId, files);
        try {
          const attachments = await uploadFilesAsAttachments(files, progress);
          progress.setPhase("Queued");
          send({
            kind: "attach_files",
            id: windowId,
            operation_id: progress.operationId,
            files: attachments,
          });
        } catch (_error) {
          progress.fail(FILE_DROP_UPLOAD_FAILURE_MESSAGE);
          showFileDropAlert(FILE_DROP_UPLOAD_FAILURE_MESSAGE);
        } finally {
          focusTerminalForWindow(windowId);
        }
      }

      function installBrowserFileDropBridge() {
        const handleDragOver = (event) => {
          if (!dataTransferHasFiles(event.dataTransfer) || eventTargetsTerminalRoot(event)) {
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          if (agentWindowIdFromDropEvent(event)) {
            event.dataTransfer.dropEffect = "copy";
          } else {
            event.dataTransfer.dropEffect = "none";
          }
        };

        const handleDrop = (event) => {
          if (!dataTransferHasFiles(event.dataTransfer) || eventTargetsTerminalRoot(event)) {
            return;
          }
          const files = Array.from(event.dataTransfer?.files || []);
          if (files.length === 0) {
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          const windowId = agentWindowIdFromDropEvent(event);
          if (!windowId) {
            showFileDropAlert(FILE_DROP_AGENT_TARGET_MESSAGE);
            return;
          }
          void sendDroppedFileAttachments(windowId, files);
        };

        window.addEventListener("dragover", handleDragOver, true);
        window.addEventListener("drop", handleDrop, true);
      }

      function installNativeFileDropBridge() {
        window.addEventListener("gwt:native-file-drop", (event) => {
          const detail = event.detail || {};
          const paths = Array.isArray(detail.paths)
            ? detail.paths.filter((path) => typeof path === "string" && path.length > 0)
            : [];
          if (paths.length === 0) {
            return;
          }
          const windowId = terminalWindowIdFromPoint(Number(detail.x), Number(detail.y));
          if (!windowId) {
            return;
          }
          const progress = createAttachmentProgressController(
            windowId,
            attachmentFilesFromNativePaths(paths),
          );
          progress.setPhase("Queued");
          send({
            kind: "attach_files",
            id: windowId,
            operation_id: progress.operationId,
            files: paths.map((path) => ({
              source: "native_path",
              path,
            })),
          });
          terminalMap.get(windowId)?.terminal.focus();
        });
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

      // xterm 6.0.0 measures the character cell via an OffscreenCanvas strategy
      // (`ctx.font = `${fontSize}px ${fontFamily}``). Canvas 2D `ctx.font` does
      // NOT resolve CSS custom properties, so a `var(--font-mono)` family token
      // is dropped and the SHORTER system fallback (SF Mono / Menlo) is measured,
      // while the rendered rows resolve var() to the TALLER JetBrains Mono — a
      // permanent measure-vs-render mismatch that clips glyphs vertically. Resolve
      // --font-mono here (keeping typography.css as the single source of truth) so
      // the measured font equals the rendered font.
      function resolveTerminalFontFamily() {
        const resolved = getComputedStyle(document.documentElement)
          .getPropertyValue("--font-mono")
          .trim();
        return (
          resolved ||
          '"JetBrains Mono Variable", "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, Consolas, monospace'
        );
      }

      function createTerminalRuntime(windowId, terminalContainer) {
        if (terminalMap.has(windowId)) {
          return terminalMap.get(windowId);
        }
        const terminal = new Terminal({
          cursorBlink: true,
          convertEol: true,
          theme: XTERM_THEME_DARK,
          fontFamily: resolveTerminalFontFamily(),
          fontSize: 14,
          lineHeight: isBlinkBrowser() ? 1.35 : 1.3,
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
        const fileDropCleanup = installTerminalFileDropHandlers(
          windowId,
          terminalContainer,
          terminal,
        );
        const contextMenuCleanup = installTerminalContextMenuHandlers(
          windowId,
          terminalContainer,
          terminal,
        );
        const wheelScrollCleanup = createTerminalWheelScrollController({
          terminalRoot: terminalContainer,
          terminal,
          window,
          isApplicationScrollFallbackEnabled: () =>
            isAgentWindowPreset(workspaceWindowById(windowId)?.preset),
          sendTerminalInput: (data) => {
            if (terminalMap.get(windowId)?.isReady !== true) {
              return;
            }
            terminal.focus();
            send({ kind: "terminal_input", id: windowId, data });
          },
        });
        const viewportRefreshCleanup = installTerminalViewportRefreshHandlers(windowId, terminal);
        // Re-fit whenever the terminal container actually changes size. Covers
        // size changes that bypass the per-event reflow wiring (maximize /
        // restore via server geometry, restore-from-minimize, tile/stack, or a
        // fit that no-opped against an unsettled layout box) which otherwise
        // leave the grown container showing a black band below the grid. The
        // manual drag-resize handler already owns reflow + final geometry, so
        // skip while it is active to avoid a redundant per-frame PTY resync.
        const containerResizeCleanup = attachContainerResizeReflow({
          element: terminalContainer,
          windowId,
          fitTerminal,
          shouldSkip: () => !!resizeState && resizeState.id === windowId,
        });
        const cleanup = () => {
          copyCleanup();
          imagePasteCleanup();
          fileDropCleanup();
          contextMenuCleanup();
          wheelScrollCleanup.dispose();
          viewportRefreshCleanup();
          containerResizeCleanup();
        };
        terminal.onData((data) => {
          inputTraceSeq += 1;
          const wsState = socket ? socket.readyState : -1;
          // Issue #2924: drop pre-ready onData firings — see
          // gateTerminalInputForReadiness in terminal-viewport-reflow.js
          // for the contract. The runtime is fetched fresh each firing so
          // a late teardown/dispose race resolves cleanly.
          const gate = gateTerminalInputForReadiness({
            runtime: terminalMap.get(windowId),
            data,
          });
          if (!gate.forward) {
            console.debug("[gwt_input_trace:onData:dropped]", {
              seq: inputTraceSeq,
              windowId,
              dataLen: data.length,
              reason: gate.reason,
              wsState,
            });
            return;
          }
          console.debug("[gwt_input_trace:onData]", {
            seq: inputTraceSeq,
            windowId,
            dataLen: data.length,
            wsState,
          });
          send({ kind: "terminal_input", id: windowId, data });
        });
        const runtime = {
          terminal,
          fitAddon,
          cleanup,
          viewportRefreshPending: false,
          activationFrame: null,
          // SPEC-2008 Phase 26.A / FR-057: initial fit handshake state.
          // `isReady` flips to `true` AFTER the first
          // runTerminalActivationSequence has run inside the rAF below,
          // i.e. after xterm has reached its real cols/rows. Until then,
          // all writeOutput / replaceTerminalSnapshot calls are captured
          // in deferredWrites / pendingSnapshotMap so the bytes are not
          // written against xterm's default 80x24 grid. Without this,
          // the early Claude Code TUI bytes (which were generated for
          // the backend's spawn cols/rows) land at the wrong grid size
          // and stay layout-locked until the next manual resize,
          // producing the post-launch corruption symptom.
          isReady: false,
          deferredWrites: [],
          hasOutput: false,
          // Issue #2937: bounds the focus-path reflow retry in
          // scheduleTerminalFocusActivation when the revealed container's
          // layout box has not settled yet (mirrors handshakeAttempts).
          activationAttempts: 0,
          // Issue #2832: see completeInitialFitHandshake.
          handshakeAttempts: 0,
        };
        terminalMap.set(windowId, runtime);
        decoderMap.set(windowId, new TextDecoder());
        // SPEC-2008 Phase 26.A / FR-057: schedule the initial fit
        // handshake. The handshake only completes once the runtime's
        // element is actually visible — see completeInitialFitHandshake.
        // If the window was created in a hidden state (e.g. inactive
        // tab group member), the rAF below runs but is a no-op; the
        // handshake then completes on the next hidden → visible
        // transition via scheduleTerminalFocusActivation.
        requestAnimationFrame(() => completeInitialFitHandshake(windowId));

        return runtime;
      }

      // SPEC-2008 Phase 26.A / FR-057: run the initial fit + replay
      // pending buffered content. Idempotent and gated on
      // `canRefreshTerminalViewport(windowId)` so we never flip
      // `isReady = true` while the runtime element is still hidden —
      // doing so would let later `writeOutput` calls bypass the
      // deferredWrites buffer and render against xterm's default 80×24
      // grid before fit ever had a chance to populate cell metrics.
      function completeInitialFitHandshake(windowId) {
        const runtime = terminalMap.get(windowId);
        if (!runtime || runtime.isReady) {
          return;
        }
        if (!canRefreshTerminalViewport(windowId)) {
          // Still hidden; wait for the next reveal. The hidden →
          // visible transition handler (scheduleTerminalFocusActivation)
          // will call back into this helper.
          return;
        }
        // SPEC-2008 Phase 26.A / FR-057 (regression fix, Issue #2832): the
        // visibility predicate above only checks `.hidden` / `.minimized`.
        // It does not catch the case where the element is structurally
        // visible but the parent has not yet been laid out (e.g. a freshly
        // appended workspace window whose CSS width/height has not
        // propagated by the time `requestAnimationFrame` fires). In that
        // state `fitAddon.fit()` resolves cell-grid dimensions against a
        // 0-sized container and silently leaves the terminal at the xterm
        // default 80×24 grid. The previous code then flipped
        // `isReady = true` and flushed `deferredWrites` into that broken
        // grid, producing the post-launch corruption that resize/move
        // recovered from. Re-schedule via rAF until the container has a
        // non-zero box, with an attempt ceiling so a perma-hidden window
        // can not pin the loop.
        if (!terminalContainerHasLayoutBox(windowId)) {
          retryInitialFitHandshake(windowId, runtime, "terminal host stayed 0-size");
          return;
        }

        const activation = runTerminalActivationSequence({
          runtime,
          windowId,
          shouldFocus: false,
          shouldPersistGeometry: true,
          sendGeometry,
        });
        if (!activation.ran) {
          retryInitialFitHandshake(windowId, runtime, "xterm fit dimensions stayed unavailable");
          return;
        }
        runtime.handshakeAttempts = 0;
        runtime.isReady = true;

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

        // Flush any terminal_output WebSocket bytes that arrived
        // between createTerminalRuntime returning and this handshake
        // firing.
        if (runtime.deferredWrites.length) {
          const flush = runtime.deferredWrites;
          runtime.deferredWrites = [];
          for (const chunk of flush) {
            writeOutput(windowId, chunk);
          }
        }
      }

      function writeOutput(windowId, base64) {
        return traceMeasure(
          UI_TRACE_EVENT.writeOutput,
          { window_id: windowId, bytes_base64: base64 ? base64.length : 0 },
          () => {
            const runtime = terminalMap.get(windowId);
            if (!runtime) {
              const queue = pendingOutputMap.get(windowId) || [];
              queue.push(base64);
              pendingOutputMap.set(windowId, queue);
              return;
            }
            if (base64) {
              runtime.hasOutput = true;
            }
            // SPEC-2008 Phase 26.A / FR-057: if the terminal has not yet
            // completed its initial fit, hold the chunk in the runtime's
            // deferred queue. The createTerminalRuntime rAF flushes this
            // queue after the activation sequence so writes land at the
            // real cols/rows instead of xterm's default 80×24 grid.
            if (runtime.isReady === false) {
              runtime.deferredWrites.push(base64);
              return;
            }
            terminalOutputBatcher.enqueue(windowId, base64);
          },
        );
      }

      function replaceTerminalSnapshot(windowId, base64) {
        const runtime = terminalMap.get(windowId);
        if (!runtime) {
          pendingSnapshotMap.set(windowId, base64);
          return;
        }
        if (base64) {
          runtime.hasOutput = true;
        }
        // SPEC-2008 Phase 26.A / FR-057: snapshots that arrive before
        // the initial fit are held in pendingSnapshotMap and replayed
        // by the createTerminalRuntime rAF after activation completes.
        // This prevents a snapshot reset+write from rendering at xterm's
        // default 80×24 grid.
        if (runtime.isReady === false) {
          pendingSnapshotMap.set(windowId, base64);
          return;
        }
        terminalOutputBatcher.clear(windowId);
        const decoder = decoderMap.get(windowId);
        runtime.terminal.reset();
        runtime.terminal.write(decoder.decode(decodeBase64(base64)), () => {
          // SPEC-2008 Phase 26.B / FR-056: `terminal.reset()` wipes the
          // internal viewport (scroll position, cell metrics caches,
          // alternate-buffer marker). The previous code only scheduled a
          // viewport refresh, which short-circuits whenever the window is
          // hidden — leaving the next visible activation with stale state
          // and dead scrollback wheel. Force the render-before-fit
          // sequence directly so the viewport is consistent the moment the
          // snapshot lands. We skip focus stealing (`shouldFocus: false`)
          // because snapshot replays happen on background tabs too.
          forceTerminalViewportRefresh(windowId, { shouldPersistGeometry: true });
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
                ["Work", "synced"],
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
              heading: "Work",
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
            // SPEC-2009 amendment: per-window picker + viewer state.
            picker: {
              open: false,
              loading: false,
              entries: [],
              error: "",
            },
            selectedWorktreeId: "",
            selectedWorktreeLabel: "",
            splitterRatio: 0.4,
            viewer: {
              path: "",
              mode: "empty", // empty | text | binary | hex | error | loading
              text: "",
              encoding: "",
              totalSize: 0,
              hexOffset: 0,
              hexBytes: "",
              error: { kind: "", message: "", size: null, limit: null },
              // SPEC-2009 amendment Phase 2: dirty / edit / save state.
              dirty: false,
              originalText: "",
              originalBytes: null, // Uint8Array
              originalEncoding: "",
              originalNewline: "lf",
              originalHasBom: false,
              originalMtime: 0,
              originalSize: 0,
              readOnly: false,
              savedAt: 0,
              saveInFlight: false,
              undoStack: [],
              redoStack: [],
            },
            // Pending navigation queued behind the Discard modal so we can
            // continue or abort after the user resolves the unsaved edit.
            discardModal: {
              open: false,
              pendingAction: null, // { kind: 'switch_file'|'open_picker'|'close_window'|'switch_worktree', payload }
            },
            conflictModal: {
              open: false,
              currentMtime: 0,
              currentSize: 0,
              pendingPayload: null, // SaveFileContent payload that triggered the conflict
            },
          });
        }
        return fileTreeStateMap.get(windowId);
      }

      function requestFileTreeWorktrees(windowId) {
        const state = ensureFileTreeState(windowId);
        state.picker.loading = true;
        state.picker.error = "";
        send({ kind: "list_file_tree_worktrees", id: windowId });
      }

      function selectFileTreeWorktree(windowId, worktreeId) {
        send({
          kind: "select_file_tree_worktree",
          id: windowId,
          worktree_id: worktreeId,
        });
      }

      function requestFileContent(windowId, path, mode, hexOffset = null, hexLength = null) {
        send({
          kind: "load_file_content",
          id: windowId,
          path,
          mode,
          hex_offset: hexOffset,
          hex_length: hexLength,
        });
      }

      function formatHexDump(offset, base64Bytes) {
        let binary;
        try {
          binary = atob(base64Bytes || "");
        } catch (e) {
          return "(invalid hex chunk)";
        }
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i += 1) {
          bytes[i] = binary.charCodeAt(i);
        }
        const BYTES_PER_LINE = 16;
        const lines = [];
        for (let i = 0; i < bytes.length; i += BYTES_PER_LINE) {
          const slice = bytes.slice(i, i + BYTES_PER_LINE);
          const lineOffset = (offset + i).toString(16).padStart(8, "0").toUpperCase();
          const hexParts = [];
          const asciiParts = [];
          for (let j = 0; j < BYTES_PER_LINE; j += 1) {
            if (j < slice.length) {
              const b = slice[j];
              hexParts.push(b.toString(16).padStart(2, "0").toUpperCase());
              asciiParts.push(b >= 0x20 && b < 0x7F ? String.fromCharCode(b) : ".");
            } else {
              hexParts.push("  ");
              asciiParts.push(" ");
            }
          }
          lines.push(lineOffset + "  " + hexParts.join(" ") + "  |" + asciiParts.join("") + "|");
        }
        return lines.join("\n");
      }

      function formatBytes(size) {
        if (size === null || size === undefined) {
          return "";
        }
        if (size < 1024) {
          return size + " B";
        }
        if (size < 1024 * 1024) {
          return (size / 1024).toFixed(1) + " KiB";
        }
        return (size / (1024 * 1024)).toFixed(2) + " MiB";
      }

      function makeEl(tag, options = {}, children = []) {
        const el = document.createElement(tag);
        if (options.className) el.className = options.className;
        if (options.text != null) el.textContent = String(options.text);
        if (options.attrs) {
          for (const [k, v] of Object.entries(options.attrs)) {
            if (v == null) continue;
            el.setAttribute(k, String(v));
          }
        }
        if (options.dataset) {
          for (const [k, v] of Object.entries(options.dataset)) {
            if (v == null) continue;
            el.dataset[k] = String(v);
          }
        }
        for (const child of children) {
          if (child == null) continue;
          if (typeof child === "string") {
            el.appendChild(document.createTextNode(child));
          } else {
            el.appendChild(child);
          }
        }
        return el;
      }

      function clearChildren(el) {
        while (el.firstChild) el.removeChild(el.firstChild);
      }

      function openWorktreePicker(windowId) {
        const state = ensureFileTreeState(windowId);
        state.picker.open = true;
        state.picker.entries = [];
        state.picker.error = "";
        renderWorktreePicker(windowId);
        requestFileTreeWorktrees(windowId);
      }

      function closeWorktreePicker(windowId) {
        const state = ensureFileTreeState(windowId);
        state.picker.open = false;
        renderWorktreePicker(windowId);
      }

      function renderWorktreePicker(windowId) {
        const modal = document.getElementById("file-tree-worktree-picker-modal");
        if (!modal) return;
        const shell = modal.querySelector(".modal-shell");
        if (!shell) return;
        const state = ensureFileTreeState(windowId);
        clearChildren(shell);
        if (!state.picker.open) {
          modal.setAttribute("aria-hidden", "true");
          modal.style.display = "none";
          modal.dataset.windowId = "";
          return;
        }
        modal.dataset.windowId = windowId;
        modal.setAttribute("aria-hidden", "false");
        modal.style.display = "flex";

        const header = makeEl("header", { className: "worktree-picker-header" }, [
          makeEl("h2", { text: "Select Worktree" }),
          makeEl("button", {
            className: "icon-button",
            text: "×",
            attrs: { type: "button", "aria-label": "Close picker" },
            dataset: { pickerAction: "cancel" },
          }),
        ]);
        const bodyContainer = makeEl("div", { className: "worktree-picker-body" });
        if (state.picker.loading && state.picker.entries.length === 0) {
          bodyContainer.appendChild(
            makeEl("div", { className: "worktree-picker-empty", text: "Loading worktrees…" }),
          );
        } else if (state.picker.error) {
          bodyContainer.appendChild(
            makeEl("div", { className: "worktree-picker-error", text: state.picker.error }),
          );
        } else if (state.picker.entries.length === 0) {
          bodyContainer.appendChild(
            makeEl("div", {
              className: "worktree-picker-empty",
              text: "No worktrees available. Use Start Work to create a new work.",
            }),
          );
        } else {
          const list = makeEl("div", { className: "worktree-picker-list" });
          for (const entry of state.picker.entries) {
            const row = makeEl(
              "button",
              {
                className: "worktree-picker-row",
                attrs: { type: "button" },
                dataset: { worktreeId: entry.id },
              },
              [
                makeEl("div", { className: "worktree-picker-row-label", text: entry.label }),
                makeEl("div", { className: "worktree-picker-row-meta" }, [
                  makeEl("span", {
                    className: "worktree-picker-kind",
                    text: entry.kind === "bare_main" ? "main" : "workspace",
                  }),
                  entry.is_active
                    ? makeEl("span", { className: "worktree-picker-active", text: "active" })
                    : null,
                  makeEl("span", { className: "worktree-picker-path", text: entry.path }),
                ]),
              ],
            );
            row.addEventListener("click", (event) => {
              event.preventDefault();
              state.selectedWorktreeId = entry.id;
              state.selectedWorktreeLabel = entry.label;
              closeWorktreePicker(windowId);
              selectFileTreeWorktree(windowId, entry.id);
              // Reset tree + viewer state when switching worktree.
              state.loaded.clear();
              state.expanded.clear();
              state.loading.clear();
              state.error = "";
              state.viewer = {
                path: "",
                mode: "empty",
                text: "",
                encoding: "",
                totalSize: 0,
                hexOffset: 0,
                hexBytes: "",
                error: { kind: "", message: "", size: null, limit: null },
              };
              requestFileTree(windowId, "");
              renderFileTreeViewer(windowId);
              renderFileTree(windowId);
            });
            list.appendChild(row);
          }
          bodyContainer.appendChild(list);
        }
        shell.appendChild(header);
        shell.appendChild(bodyContainer);
        shell
          .querySelector('[data-picker-action="cancel"]')
          ?.addEventListener("click", () => closeWorktreePicker(windowId));
      }

      // SPEC-2009 amendment Phase 2b: lazy-load highlight.js once on first
      // text viewer use. Bundled module sits at /assets/highlight/.
      let highlightModulePromise = null;
      function loadHighlightModule() {
        if (!highlightModulePromise) {
          highlightModulePromise = import("/assets/highlight/highlight.min.js")
            .then((mod) => mod.default || mod)
            .catch((err) => {
              console.warn("highlight.js failed to load", err);
              return null;
            });
        }
        return highlightModulePromise;
      }

      const FILE_EXT_TO_LANGUAGE = {
        js: "javascript", mjs: "javascript", cjs: "javascript", jsx: "javascript",
        ts: "typescript", tsx: "typescript",
        rs: "rust", py: "python", rb: "ruby", go: "go", java: "java",
        c: "c", h: "c", cpp: "cpp", cc: "cpp", hpp: "cpp", cs: "csharp",
        sh: "bash", bash: "bash", zsh: "bash", fish: "bash",
        json: "json", yaml: "yaml", yml: "yaml", toml: "ini", ini: "ini",
        md: "markdown", markdown: "markdown",
        html: "xml", htm: "xml", xml: "xml", svg: "xml",
        css: "css", scss: "scss", less: "less",
        sql: "sql", dockerfile: "dockerfile",
        kt: "kotlin", swift: "swift", php: "php", lua: "lua",
        vue: "xml", graphql: "graphql", gql: "graphql",
      };

      function detectLanguageByExtension(path) {
        if (!path) return "";
        const base = String(path).split("/").pop() || "";
        if (/^Dockerfile/i.test(base)) return "dockerfile";
        if (/^Makefile/i.test(base)) return "makefile";
        const dot = base.lastIndexOf(".");
        if (dot < 0) return "";
        const ext = base.slice(dot + 1).toLowerCase();
        return FILE_EXT_TO_LANGUAGE[ext] || "";
      }

      function applySyntaxHighlight(codeEl, text, language) {
        if (!codeEl) return;
        // Always set raw text first so the viewer renders something even
        // before highlight.js loads (or if it fails to load entirely).
        codeEl.textContent = text || "";
        codeEl.className = language ? `hljs language-${language}` : "hljs";
        loadHighlightModule().then((hljs) => {
          if (!hljs) return;
          try {
            if (language && hljs.getLanguage && hljs.getLanguage(language)) {
              const html = hljs.highlight(text || "", { language, ignoreIllegals: true }).value;
              codeEl.innerHTML = html;
              codeEl.className = `hljs language-${language}`;
            } else {
              const result = hljs.highlightAuto(text || "");
              codeEl.innerHTML = result.value;
              codeEl.className = `hljs language-${result.language || "plaintext"}`;
            }
          } catch (e) {
            // Fall back to plain text on highlight failure.
            codeEl.textContent = text || "";
          }
        });
      }

      // SPEC-2009 amendment Phase 2: helper to decode the binary chunk sent
      // by the backend over base64 so the hex viewer can mutate single
      // bytes locally before issuing a save_file_content(mode=hex).
      function decodeBase64ToBytes(b64) {
        let binary;
        try {
          binary = atob(b64 || "");
        } catch (_e) {
          return new Uint8Array();
        }
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i += 1) {
          bytes[i] = binary.charCodeAt(i);
        }
        return bytes;
      }

      function encodeBytesToBase64(bytes) {
        if (!bytes || bytes.length === 0) return "";
        let binary = "";
        for (let i = 0; i < bytes.length; i += 1) {
          binary += String.fromCharCode(bytes[i]);
        }
        return btoa(binary);
      }

      function recomputeHexDirty(state) {
        const v = state.viewer;
        if (v.mode !== "hex" || !v.originalBytes) {
          return;
        }
        const current = decodeBase64ToBytes(v.hexBytes || "");
        if (current.length !== v.originalBytes.length) {
          v.dirty = true;
          return;
        }
        for (let i = 0; i < current.length; i += 1) {
          if (current[i] !== v.originalBytes[i]) {
            v.dirty = true;
            return;
          }
        }
        v.dirty = false;
      }

      function requestSaveFileContent(windowId) {
        const state = ensureFileTreeState(windowId);
        const v = state.viewer;
        if (v.saveInFlight || v.readOnly || !v.dirty) {
          return;
        }
        const payload = {
          kind: "save_file_content",
          id: windowId,
          path: v.path,
          mode: v.mode,
          expected_mtime: v.originalMtime,
          expected_size: v.originalSize,
        };
        if (v.mode === "text") {
          payload.text = v.text;
          payload.encoding = (v.originalEncoding || "utf-8").toLowerCase();
          payload.newline = (v.originalNewline || "lf").toLowerCase();
          payload.has_bom = v.originalHasBom;
        } else if (v.mode === "hex") {
          // Hex save sends a single byte at a single offset (replace-only).
          // We pull the dirty byte by diffing against originalBytes.
          const current = decodeBase64ToBytes(v.hexBytes || "");
          let dirtyOffset = -1;
          for (let i = 0; i < current.length; i += 1) {
            if (current[i] !== (v.originalBytes ? v.originalBytes[i] : -1)) {
              dirtyOffset = i;
              break;
            }
          }
          if (dirtyOffset < 0) {
            return;
          }
          payload.hex_offset = v.hexOffset + dirtyOffset;
          payload.hex_byte = current[dirtyOffset];
        } else {
          return;
        }
        v.saveInFlight = true;
        state.lastSavePayload = payload;
        send(payload);
        renderFileTreeViewer(windowId);
      }

      function applyAfterSaveContinuation(windowId) {
        const state = ensureFileTreeState(windowId);
        const pending = state.discardModal && state.discardModal.pendingAction;
        if (!pending || !pending.queuedFromDiscard) return;
        state.discardModal.pendingAction = null;
        runPendingNavigation(windowId, pending);
      }

      function runPendingNavigation(windowId, pending) {
        if (!pending) return;
        switch (pending.kind) {
          case "switch_file":
            beginViewerForFile(windowId, pending.path);
            break;
          case "open_picker":
            openWorktreePicker(windowId);
            break;
          case "switch_worktree":
            // Re-emit the worktree selection that was queued behind the modal.
            selectFileTreeWorktree(windowId, pending.worktreeId);
            break;
          case "close_window":
            // Forward to the backend close path so persistence stays in sync.
            send({ kind: "close_window", id: windowId });
            break;
          default:
            break;
        }
      }

      function beginViewerForFile(windowId, path) {
        const state = ensureFileTreeState(windowId);
        state.viewer = {
          ...state.viewer,
          path,
          mode: "loading",
          text: "",
          encoding: "",
          totalSize: 0,
          hexOffset: 0,
          hexBytes: "",
          error: { kind: "", message: "", size: null, limit: null },
          dirty: false,
          originalText: "",
          originalBytes: null,
          originalEncoding: "",
          originalNewline: "lf",
          originalHasBom: false,
          originalMtime: 0,
          originalSize: 0,
          readOnly: false,
          savedAt: 0,
          saveInFlight: false,
          undoStack: [],
          redoStack: [],
        };
        renderFileTreeViewer(windowId);
        requestFileContent(windowId, path, "text");
      }

      function queueNavigationGuardedByDirty(windowId, pendingAction) {
        const state = ensureFileTreeState(windowId);
        if (state.viewer.dirty) {
          state.discardModal = {
            open: true,
            pendingAction: { ...pendingAction, queuedFromDiscard: true },
          };
          renderDiscardModal(windowId);
          return true;
        }
        return false;
      }

      function closeDiscardModal(windowId) {
        const state = ensureFileTreeState(windowId);
        state.discardModal = { open: false, pendingAction: null };
        renderDiscardModal(windowId);
      }

      function renderDiscardModal(windowId) {
        const modal = document.getElementById("file-tree-discard-modal");
        if (!modal) return;
        const shell = modal.querySelector(".modal-shell");
        if (!shell) return;
        const state = ensureFileTreeState(windowId);
        clearChildren(shell);
        if (!state.discardModal.open) {
          modal.setAttribute("aria-hidden", "true");
          modal.dataset.windowId = "";
          return;
        }
        modal.dataset.windowId = windowId;
        modal.setAttribute("aria-hidden", "false");
        const header = makeEl("header", { className: "discard-modal-header" }, [
          makeEl("h2", { text: "Unsaved changes" }),
        ]);
        const bodyText = makeEl("div", { className: "discard-modal-body" }, [
          makeEl("p", {
            text: `${state.viewer.path} has unsaved changes. Save them or discard before continuing.`,
          }),
        ]);
        const footer = makeEl("footer", { className: "discard-modal-footer" });
        const saveBtn = makeEl("button", {
          className: "wizard-button primary",
          attrs: { type: "button" },
          text: "Save",
        });
        const discardBtn = makeEl("button", {
          className: "wizard-button",
          attrs: { type: "button" },
          text: "Discard",
        });
        const cancelBtn = makeEl("button", {
          className: "wizard-button",
          attrs: { type: "button" },
          text: "Cancel",
        });
        saveBtn.addEventListener("click", () => {
          requestSaveFileContent(windowId);
          // Close modal but keep pending action; resume after file_content_saved.
          state.discardModal.open = false;
          renderDiscardModal(windowId);
        });
        discardBtn.addEventListener("click", () => {
          // Roll back to the original baseline and run the pending action.
          const v = state.viewer;
          if (v.mode === "text") {
            v.text = v.originalText;
          } else if (v.mode === "hex") {
            v.hexBytes = encodeBytesToBase64(v.originalBytes || new Uint8Array());
          }
          v.dirty = false;
          const pending = state.discardModal.pendingAction;
          state.discardModal = { open: false, pendingAction: null };
          renderDiscardModal(windowId);
          runPendingNavigation(windowId, pending);
        });
        cancelBtn.addEventListener("click", () => closeDiscardModal(windowId));
        footer.appendChild(saveBtn);
        footer.appendChild(discardBtn);
        footer.appendChild(cancelBtn);
        shell.appendChild(header);
        shell.appendChild(bodyText);
        shell.appendChild(footer);
      }

      function renderConflictModal(windowId) {
        const modal = document.getElementById("file-tree-conflict-modal");
        if (!modal) return;
        const shell = modal.querySelector(".modal-shell");
        if (!shell) return;
        const state = ensureFileTreeState(windowId);
        clearChildren(shell);
        if (!state.conflictModal.open) {
          modal.setAttribute("aria-hidden", "true");
          modal.dataset.windowId = "";
          return;
        }
        modal.dataset.windowId = windowId;
        modal.setAttribute("aria-hidden", "false");
        const header = makeEl("header", { className: "conflict-modal-header" }, [
          makeEl("h2", { text: "File changed externally" }),
        ]);
        const bodyText = makeEl("div", { className: "conflict-modal-body" }, [
          makeEl("p", {
            text: `${state.viewer.path} was modified outside the editor. Choose how to proceed.`,
          }),
        ]);
        const footer = makeEl("footer", { className: "conflict-modal-footer" });
        const overwriteBtn = makeEl("button", {
          className: "wizard-button primary",
          attrs: { type: "button" },
          text: "Overwrite",
        });
        const reloadBtn = makeEl("button", {
          className: "wizard-button",
          attrs: { type: "button" },
          text: "Reload from disk",
        });
        const cancelBtn = makeEl("button", {
          className: "wizard-button",
          attrs: { type: "button" },
          text: "Cancel",
        });
        overwriteBtn.addEventListener("click", () => {
          // Re-issue the save with the latest expected_metadata so the
          // domain layer skips its conflict gate this time.
          const v = state.viewer;
          v.originalMtime = state.conflictModal.currentMtime;
          v.originalSize = state.conflictModal.currentSize;
          state.conflictModal = { open: false, currentMtime: 0, currentSize: 0, pendingPayload: null };
          renderConflictModal(windowId);
          requestSaveFileContent(windowId);
        });
        reloadBtn.addEventListener("click", () => {
          const v = state.viewer;
          state.conflictModal = { open: false, currentMtime: 0, currentSize: 0, pendingPayload: null };
          renderConflictModal(windowId);
          // Throw away the unsaved edit and re-read from disk.
          beginViewerForFile(windowId, v.path);
        });
        cancelBtn.addEventListener("click", () => {
          state.conflictModal = { open: false, currentMtime: 0, currentSize: 0, pendingPayload: null };
          renderConflictModal(windowId);
        });
        footer.appendChild(overwriteBtn);
        footer.appendChild(reloadBtn);
        footer.appendChild(cancelBtn);
        shell.appendChild(header);
        shell.appendChild(bodyText);
        shell.appendChild(footer);
      }

      function renderFileTreeViewer(windowId) {
        const state = ensureFileTreeState(windowId);
        // The workspace window element exposes its id via `data-id`
        // (see `ensureWindow`), not `data-window-id`. Using the right
        // attribute is required for the viewer DOM lookup to resolve in
        // both production and Playwright fixtures.
        const surface = document.querySelector(
          `[data-id='${CSS.escape(windowId)}'] .file-tree-viewer`,
        );
        if (!surface) return;
        const header = surface.querySelector(".file-tree-viewer-header");
        const body = surface.querySelector(".file-tree-viewer-body");
        if (!header || !body) return;
        clearChildren(header);
        clearChildren(body);
        const v = state.viewer;
        const sizeLabel = v.totalSize ? formatBytes(v.totalSize) : "";
        const headerPath = makeEl("span", { className: "file-tree-viewer-path", text: v.path || "" });
        const dirtyMarker = makeEl("span", {
          className: "file-tree-viewer-dirty",
          text: "●",
        });
        switch (v.mode) {
          case "empty":
            header.appendChild(
              makeEl("span", {
                className: "file-tree-viewer-placeholder",
                text: "No file selected",
              }),
            );
            body.appendChild(
              makeEl("div", {
                className: "file-tree-viewer-empty",
                text: "Select a file to view its contents.",
              }),
            );
            break;
          case "loading":
            header.appendChild(headerPath);
            body.appendChild(
              makeEl("div", { className: "file-tree-viewer-empty", text: "Loading…" }),
            );
            break;
          case "text": {
            header.appendChild(headerPath);
            // SPEC-2009 Phase 2b: dirty marker / Saved badge live in the
            // header as toggleable elements so the input handler can flip
            // their visibility without a full re-render. Keeping the
            // textarea alive across keystrokes is what stops focus loss
            // mid-typing (Phase 2 had a re-render-on-input loop that
            // recreated the textarea each character — visible in headed
            // Playwright but masked by fill() in the headless smoke).
            dirtyMarker.style.display = v.dirty ? "" : "none";
            header.appendChild(dirtyMarker);
            const langBadge = makeEl("span", {
              className: "file-tree-viewer-lang",
              text: detectLanguageByExtension(v.path).toUpperCase() || "PLAIN",
            });
            header.appendChild(langBadge);
            header.appendChild(
              makeEl("span", {
                className: "file-tree-viewer-meta",
                text: (v.encoding || "") + " · " + (v.originalNewline || "lf").toUpperCase() + " · " + sizeLabel,
              }),
            );
            if (v.readOnly) {
              header.appendChild(
                makeEl("span", {
                  className: "file-tree-viewer-readonly",
                  text: "read-only",
                }),
              );
            }
            const saveBtn = makeEl("button", {
              className: "wizard-button file-tree-viewer-save",
              attrs: { type: "button" },
              text: v.saveInFlight ? "Saving…" : "Save",
            });
            const updateSaveBtn = () => {
              saveBtn.textContent = v.saveInFlight ? "Saving…" : "Save";
              if (!v.dirty || v.readOnly || v.saveInFlight) {
                saveBtn.setAttribute("disabled", "");
              } else {
                saveBtn.removeAttribute("disabled");
              }
            };
            updateSaveBtn();
            saveBtn.addEventListener("click", () => requestSaveFileContent(windowId));
            header.appendChild(saveBtn);
            const savedBadge = makeEl("span", {
              className: "file-tree-viewer-saved",
              text: "Saved",
            });
            savedBadge.style.display = v.savedAt && Date.now() - v.savedAt < 2000 ? "" : "none";
            header.appendChild(savedBadge);

            // Overlay editor: highlighted <pre> sits behind a transparent
            // <textarea>. Both share the same monospace metrics so the
            // overlay aligns character-for-character. Scroll syncs in
            // both directions so the highlight follows the caret.
            const wrap = makeEl("div", { className: "file-tree-viewer-editor-wrap" });
            const language = detectLanguageByExtension(v.path);
            const hlPre = makeEl("pre", { className: "file-tree-viewer-hl" });
            const hlCode = makeEl("code", {
              className: language ? `hljs language-${language}` : "hljs",
            });
            hlPre.appendChild(hlCode);
            const textarea = makeEl("textarea", {
              className: "file-tree-viewer-text file-tree-viewer-editor",
              attrs: { spellcheck: "false", wrap: "off" },
            });
            textarea.value = v.text;
            if (v.readOnly || v.saveInFlight) {
              textarea.setAttribute("disabled", "");
            }
            applySyntaxHighlight(hlCode, v.text, language);
            const syncScroll = () => {
              hlPre.scrollTop = textarea.scrollTop;
              hlPre.scrollLeft = textarea.scrollLeft;
            };
            textarea.addEventListener("scroll", syncScroll);
            textarea.addEventListener("input", () => {
              v.text = textarea.value;
              v.dirty = v.text !== v.originalText;
              dirtyMarker.style.display = v.dirty ? "" : "none";
              updateSaveBtn();
              applySyntaxHighlight(hlCode, v.text, language);
              syncScroll();
            });
            wrap.appendChild(hlPre);
            wrap.appendChild(textarea);
            body.appendChild(wrap);
            break;
          }
          case "binary": {
            header.appendChild(headerPath);
            header.appendChild(
              makeEl("span", { className: "file-tree-viewer-meta", text: "binary · " + sizeLabel }),
            );
            if (v.readOnly) {
              header.appendChild(
                makeEl("span", {
                  className: "file-tree-viewer-readonly",
                  text: "read-only",
                }),
              );
            }
            const btn = makeEl("button", {
              className: "wizard-button",
              text: "View as hex",
              attrs: { type: "button" },
              dataset: { viewerAction: "view-as-hex" },
            });
            btn.addEventListener("click", () => {
              v.mode = "loading";
              v.hexOffset = 0;
              v.hexBytes = "";
              renderFileTreeViewer(windowId);
              requestFileContent(windowId, v.path, "hex", 0, 64 * 16);
            });
            header.appendChild(btn);
            body.appendChild(
              makeEl("div", {
                className: "file-tree-viewer-notice",
                text:
                  "Cannot display as text. Use “View as hex” for a 16-byte/row hex dump.",
              }),
            );
            break;
          }
          case "hex": {
            header.appendChild(headerPath);
            if (v.dirty) header.appendChild(dirtyMarker);
            header.appendChild(
              makeEl("span", { className: "file-tree-viewer-meta", text: "hex · " + sizeLabel }),
            );
            if (v.readOnly) {
              header.appendChild(
                makeEl("span", {
                  className: "file-tree-viewer-readonly",
                  text: "read-only",
                }),
              );
            }
            const saveBtn = makeEl("button", {
              className: "wizard-button file-tree-viewer-save",
              attrs: { type: "button" },
              text: v.saveInFlight ? "Saving…" : "Save",
            });
            if (!v.dirty || v.readOnly || v.saveInFlight) {
              saveBtn.setAttribute("disabled", "");
            }
            saveBtn.addEventListener("click", () => requestSaveFileContent(windowId));
            header.appendChild(saveBtn);
            if (v.savedAt && Date.now() - v.savedAt < 2000) {
              header.appendChild(
                makeEl("span", {
                  className: "file-tree-viewer-saved",
                  text: "Saved",
                }),
              );
            }
            // Render hex byte cells inline so we can attach click handlers
            // for single-byte replace edits.
            const container = makeEl("div", { className: "file-tree-viewer-hex" });
            const bytes = decodeBase64ToBytes(v.hexBytes || "");
            const BYTES_PER_LINE = 16;
            for (let line = 0; line < bytes.length; line += BYTES_PER_LINE) {
              const row = makeEl("div", { className: "file-tree-hex-row" });
              const offsetLabel = (v.hexOffset + line)
                .toString(16)
                .padStart(8, "0")
                .toUpperCase();
              row.appendChild(makeEl("span", { className: "file-tree-hex-offset", text: offsetLabel }));
              const bytesContainer = makeEl("span", { className: "file-tree-hex-bytes" });
              const asciiContainer = makeEl("span", { className: "file-tree-hex-ascii" });
              for (let j = 0; j < BYTES_PER_LINE; j += 1) {
                const idx = line + j;
                if (idx < bytes.length) {
                  const b = bytes[idx];
                  const cell = makeEl("button", {
                    className: "file-tree-hex-cell",
                    attrs: { type: "button" },
                    text: b.toString(16).padStart(2, "0").toUpperCase(),
                    dataset: { hexOffset: String(v.hexOffset + idx) },
                  });
                  if (v.readOnly) {
                    cell.setAttribute("disabled", "");
                  } else {
                    cell.addEventListener("click", () => {
                      const input = window.prompt("Replace byte (2 hex digits)", cell.textContent);
                      if (input == null) return;
                      const normalised = input.trim();
                      if (!/^[0-9a-fA-F]{1,2}$/.test(normalised)) {
                        window.alert("Enter 1 or 2 hex digits (0-9, A-F).");
                        return;
                      }
                      const newByte = parseInt(normalised, 16);
                      const prev = bytes[idx];
                      if (prev === newByte) return;
                      bytes[idx] = newByte;
                      v.undoStack.push({ offset: idx, prev });
                      v.redoStack = [];
                      v.hexBytes = encodeBytesToBase64(bytes);
                      recomputeHexDirty(state);
                      renderFileTreeViewer(windowId);
                    });
                  }
                  bytesContainer.appendChild(cell);
                  bytesContainer.appendChild(document.createTextNode(" "));
                  asciiContainer.appendChild(
                    document.createTextNode(b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : "."),
                  );
                } else {
                  bytesContainer.appendChild(document.createTextNode("   "));
                  asciiContainer.appendChild(document.createTextNode(" "));
                }
              }
              row.appendChild(bytesContainer);
              row.appendChild(makeEl("span", { className: "file-tree-hex-divider", text: "|" }));
              row.appendChild(asciiContainer);
              row.appendChild(makeEl("span", { className: "file-tree-hex-divider", text: "|" }));
              container.appendChild(row);
            }
            body.appendChild(container);
            break;
          }
          case "error":
            header.appendChild(headerPath);
            body.appendChild(
              makeEl("div", {
                className: "file-tree-viewer-error",
                text: v.error.message || "Unable to load file",
              }),
            );
            break;
          default:
            header.appendChild(
              makeEl("span", {
                className: "file-tree-viewer-placeholder",
                text: "Unknown state",
              }),
            );
        }
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
              forceFilesystemDelete: false,
              progress: null,
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
            saveInFlight: false,
          });
        }
        return profileStateMap.get(windowId);
      }

      function ensureKnowledgeBridgeState(windowId, knowledgeKind) {
        if (!knowledgeBridgeStateMap.has(windowId)) {
          knowledgeBridgeStateMap.set(windowId, {
            kind: knowledgeKind,
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
            autoRefreshTimer: null,
          });
        }
        const state = knowledgeBridgeStateMap.get(windowId);
        state.kind = knowledgeKind || state.kind;
        if (state.hideDone === undefined) {
          state.hideDone = readKanbanHideDonePreference();
        }
        if (!state.pendingPhaseUpdates) {
          state.pendingPhaseUpdates = new Map();
        }
        return state;
      }

      function knowledgeAutoRefreshIsBusy(state) {
        return (
          state.loading ||
          state.refreshing ||
          state.searching ||
          state.searchInFlight
        );
      }

      function ensureKnowledgeAutoRefresh(windowId, knowledgeKind) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        if (state.autoRefreshTimer) {
          return;
        }
        state.autoRefreshTimer = setInterval(() => {
          if (!windowMap.get(windowId)) {
            clearInterval(state.autoRefreshTimer);
            state.autoRefreshTimer = null;
            return;
          }
          if (!state.refreshEnabled || knowledgeAutoRefreshIsBusy(state)) {
            return;
          }
          requestKnowledgeBridge(windowId, knowledgeKind, true);
        }, KNOWLEDGE_AUTO_REFRESH_INTERVAL_MS);
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
            // SPEC-2019 Amendment 2026-05-20 (Process facet) — AND-filter
            // by ProcessKind on top of severity + query. "" means "all".
            // Matches LogEvent.fields.kind injected by `spawn_logged`
            // summary events (target = "gwt.process.summary").
            processKind: "",
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
            composerTitle: "",
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
            audienceFilter: "workspace",
            currentWorkspaceId: "",
            forYouUnread: 0,
            lastNotifiedMentionEntryId: null,
            currentWorkspaceId: currentProjectWorkspaceId,
          });
        }
        return boardStateMap.get(windowId);
      }

      // SPEC-2359 FR-098/101 + US-53: track every live assigned Work id for
      // the Board Work filter. Broadcast entries remain visible everywhere;
      // scoped entries match when their audience includes any active Work id.
      const WORK_ID_KEY_SEPARATOR = "\u001f";
      let currentProjectWorkspaceId = [];
      let currentProjectWorkspaceKey = "";
      let activeWorkProjectionWorkspaceIds = [];
      function uniqueWorkIds(values) {
        const ids = [];
        for (const value of values || []) {
          const id = String(value || "").trim();
          if (id && !ids.includes(id)) ids.push(id);
        }
        return ids;
      }
      function workIdsKey(ids) {
        return (ids || []).join(WORK_ID_KEY_SEPARATOR);
      }
      function cacheActiveWorkProjectionWorkspaceIds(projection) {
        activeWorkProjectionWorkspaceIds = uniqueWorkIds(
          Array.isArray(projection?.active_works)
            ? projection.active_works.map((work) => work?.id)
            : [],
        );
      }
      function deriveCurrentProjectWorkspaceIds(workspaceState) {
        if (activeWorkProjectionWorkspaceIds.length > 0) {
          return activeWorkProjectionWorkspaceIds;
        }
        const agents = workspaceState?.workspace?.agents
          || workspaceState?.agents
          || [];
        return uniqueWorkIds(
          agents
            .filter(
              (agent) =>
                String(agent?.affiliation_status || "").toLowerCase() === "assigned"
                && typeof agent?.workspace_id === "string"
                && agent.workspace_id.length > 0,
            )
            .map((agent) => agent.workspace_id),
        );
      }
      function syncCurrentProjectWorkspaceIds(nextIds) {
        const ids = Array.isArray(nextIds) ? nextIds : [];
        const nextKey = workIdsKey(ids);
        if (nextKey === currentProjectWorkspaceKey) {
          return false;
        }
        currentProjectWorkspaceId = ids;
        currentProjectWorkspaceKey = nextKey;
        refreshBoardCurrentWorkspaceId();
        return true;
      }
      function refreshBoardCurrentWorkspaceId() {
        for (const state of boardStateMap.values()) {
          state.currentWorkspaceId = currentProjectWorkspaceId;
        }
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
          all: state.audienceFilter === "all",
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
          all: state.audienceFilter === "all",
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

      function replaceKnowledgeEntry(entries, fresh) {
        if (!fresh || !Array.isArray(entries)) {
          return false;
        }
        const index = entries.findIndex((entry) => entry.number === fresh.number);
        if (index < 0) {
          return false;
        }
        entries[index] = fresh;
        return true;
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

      function setLaunchWizardPendingDisabled(root, disabled) {
        if (!disabled) return;
        const selector =
          "input, textarea, select, button, [role='button'], [contenteditable='true']";
        for (const element of root.querySelectorAll(selector)) {
          if ("disabled" in element) {
            element.disabled = true;
          }
          element.setAttribute("aria-disabled", "true");
          element.setAttribute("tabindex", "-1");
        }
      }

      function createKnowledgeMarkdownBody(section, className = "knowledge-section-body") {
        const node = createNode("div", `${className} knowledge-markdown-body`);
        const html = typeof section?.body_html === "string" ? section.body_html.trim() : "";
        if (html) {
          node.innerHTML = html;
        } else {
          node.classList.add("is-plaintext");
          node.textContent = section?.body || "";
        }
        return node;
      }

      // SPEC-2359 US-42 — Workspace Resume Picker controller. The
      // Workspace Overview Resume button asks the backend to list
      // resumable agents; the response opens this modal so the user can
      // pick which previously-assigned agent to restart in-place
      // (without going through the Launch Wizard).
      const workspaceResumePicker = createWorkspaceResumePickerController({
        modalEl: document.getElementById("workspace-resume-picker-modal"),
        dialogEl: document.querySelector("#workspace-resume-picker-modal .modal-shell"),
        createNode,
        send,
        getResumeBounds: () => visibleBounds(),
      });

      const workspaceOverviewSurface = createWorkspaceOverviewSurface({
        activeWorkspace,
        agentStatusLabel,
        appendMeta,
        createWorkspacePrMeta,
        createNode,
        getActiveWorkProjection: () => activeWorkProjection,
        openWorkspaceCleanup,
        send,
        windowMap,
        workspaceWindowById,
        openWorkspaceResumePicker: (workspaceId) => workspaceResumePicker.open(workspaceId),
        getResumeBounds: () => visibleBounds(),
        branchesSurface: {
          ensureBranchListState: (...a) => ensureBranchListState(...a),
          requestBranches: (...a) => requestBranches(...a),
          renderBranches: (...a) => renderBranches(...a),
          openBranchCleanupModal: (...a) => openBranchCleanupModal(...a),
        },
      });

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

      function boardOriginActiveAgents() {
        const assigned = Array.isArray(activeWorkProjection?.agents)
          ? activeWorkProjection.agents
          : [];
        const unassigned = Array.isArray(activeWorkProjection?.unassigned_agents)
          ? activeWorkProjection.unassigned_agents
          : [];
        return assigned.concat(unassigned);
      }

      function openBoardOriginAgent(windowId, entry) {
        const originSessionId = boardEntryOriginSessionId(entry);
        if (!originSessionId) {
          return;
        }
        send({
          kind: "open_board_origin_agent",
          id: windowId,
          origin_session_id: originSessionId,
          bounds: visibleBounds(),
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
        const processKind = String(state.processKind || "");
        return (state.entries || [])
          .filter(
            (entry) =>
              logSeverityRank(entry.severity) >= minimumRank &&
              logMatchesQuery(entry, query) &&
              logMatchesProcessKind(entry, processKind),
          )
          .slice()
          .reverse();
      }

      // SPEC-2019 Amendment 2026-05-20 — AND-combine the Process kind chip
      // with severity / keyword filters. When `processKind` is empty the
      // entry passes through; otherwise the entry must carry the matching
      // `kind` field in its `fields` map. `spawn_logged` summary events
      // emit the kind there (target = "gwt.process.summary").
      function logMatchesProcessKind(entry, processKind) {
        if (!processKind) {
          return true;
        }
        const fields = entry.fields || {};
        return String(fields.kind || "") === processKind;
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
        const processKindSelect = body.querySelector(".logs-process-kind-select");
        const searchInput = body.querySelector(".logs-search-input");
        const timeline = body.querySelector(".logs-timeline");
        const detailPane = body.querySelector(".logs-detail-pane");
        if (
          !status ||
          !unreadButton ||
          !severitySelect ||
          !processKindSelect ||
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
        processKindSelect.value = state.processKind || "";
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

      function normalizeProfileEnvKey(key) {
        return String(key || "").trim();
      }

      function profileDraftPayload(draft) {
        if (!draft) {
          return { envVars: [], disabledEnv: [] };
        }
        const envByKey = new Map();
        for (const entry of draft.envVars || []) {
          const key = normalizeProfileEnvKey(entry.key);
          if (!key) {
            continue;
          }
          envByKey.set(key, {
            key,
            value: entry.value ?? "",
          });
        }
        const disabledSet = new Set();
        for (const entry of draft.disabledEnv || []) {
          const key = normalizeProfileEnvKey(entry);
          if (key) {
            disabledSet.add(key);
          }
        }
        for (const key of disabledSet) {
          envByKey.delete(key);
        }
        return {
          envVars: Array.from(envByKey.values()).sort((left, right) =>
            left.key.localeCompare(right.key),
          ),
          disabledEnv: Array.from(disabledSet).sort((left, right) =>
            left.localeCompare(right),
          ),
        };
      }

      function removeProfileEnvOverride(draft, key) {
        const normalized = normalizeProfileEnvKey(key);
        draft.envVars = (draft.envVars || []).filter(
          (entry) => normalizeProfileEnvKey(entry.key) !== normalized,
        );
      }

      function removeProfileDisabledKey(draft, key) {
        const normalized = normalizeProfileEnvKey(key);
        draft.disabledEnv = (draft.disabledEnv || []).filter(
          (entry) => normalizeProfileEnvKey(entry) !== normalized,
        );
      }

      function setProfileEnvOverride(draft, key, value) {
        const normalized = normalizeProfileEnvKey(key);
        if (!normalized) {
          return null;
        }
        removeProfileDisabledKey(draft, normalized);
        const existing = (draft.envVars || []).find(
          (entry) => normalizeProfileEnvKey(entry.key) === normalized,
        );
        if (existing) {
          existing.key = normalized;
          existing.value = value ?? "";
          return existing;
        }
        const entry = { key: normalized, value: value ?? "" };
        draft.envVars.push(entry);
        return entry;
      }

      function setProfileRowMode(draft, key, mode) {
        const normalized = normalizeProfileEnvKey(key);
        if (!normalized) {
          return;
        }
        if (mode === "use_os") {
          removeProfileEnvOverride(draft, normalized);
          removeProfileDisabledKey(draft, normalized);
          return;
        }
        if (mode === "disabled") {
          removeProfileEnvOverride(draft, normalized);
          if (
            !(draft.disabledEnv || []).some(
              (entry) => normalizeProfileEnvKey(entry) === normalized,
            )
          ) {
            draft.disabledEnv.push(normalized);
          }
          return;
        }
        const existing = (draft.envVars || []).find(
          (entry) => normalizeProfileEnvKey(entry.key) === normalized,
        );
        setProfileEnvOverride(draft, normalized, existing?.value ?? "");
      }

      function profileEnvironmentRows(snapshot, draft) {
        const payload = profileDraftPayload(draft);
        const osEntries = (snapshot.os_env || [])
          .map((entry) => ({
            key: normalizeProfileEnvKey(entry.key),
            value: entry.value ?? "",
          }))
          .filter((entry) => entry.key)
          .sort((left, right) => left.key.localeCompare(right.key));
        const overrides = new Map(payload.envVars.map((entry) => [entry.key, entry]));
        const disabled = new Set(payload.disabledEnv);
        const osKeys = new Set();
        const rows = [];

        for (const entry of osEntries) {
          osKeys.add(entry.key);
          const mode = disabled.has(entry.key)
            ? "disabled"
            : overrides.has(entry.key)
              ? "override"
              : "use_os";
          const profileValue = overrides.get(entry.key)?.value ?? "";
          rows.push({
            kind: "os",
            key: entry.key,
            osValue: entry.value,
            mode,
            profileValue,
            result:
              mode === "disabled"
                ? "Disabled"
                : mode === "override"
                  ? profileValue
                  : entry.value,
          });
        }

        const addedKeys = new Set();
        for (const entry of payload.envVars) {
          if (osKeys.has(entry.key)) {
            continue;
          }
          addedKeys.add(entry.key);
          rows.push({
            kind: "added",
            key: entry.key,
            osValue: "",
            mode: "override",
            profileValue: entry.value,
            result: entry.value,
          });
        }
        for (const key of payload.disabledEnv) {
          if (osKeys.has(key) || addedKeys.has(key)) {
            continue;
          }
          rows.push({
            kind: "added",
            key,
            osValue: "",
            mode: "disabled",
            profileValue: "",
            result: "Disabled",
          });
        }

        rows.sort((left, right) => {
          if (left.kind !== right.kind) {
            return left.kind === "os" ? -1 : 1;
          }
          return left.key.localeCompare(right.key);
        });

        (draft.envVars || []).forEach((entry, index) => {
          if (normalizeProfileEnvKey(entry.key)) {
            return;
          }
          rows.push({
            kind: "pending",
            key: entry.key || "",
            osValue: "",
            mode: "override",
            profileValue: entry.value ?? "",
            result: entry.value ?? "",
            draftIndex: index,
          });
        });

        return rows;
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
        const payload = profileDraftPayload(draft);
        return JSON.stringify({
          currentName: draft.currentName,
          name: draft.name,
          description: draft.description,
          envVars: payload.envVars,
          disabledEnv: payload.disabledEnv,
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

      function profileHasEditableFocus(windowId) {
        const element = windowMap.get(windowId);
        const active = document.activeElement;
        return Boolean(
          element &&
            active &&
            element.contains(active) &&
            active.matches?.("input, textarea, select"),
        );
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
        state.saveInFlight = true;
        state.error = "";
        updateProfileStatus(windowId);
        const payload = profileDraftPayload(state.draft);
        send({
          kind: "save_profile",
          id: windowId,
          current_name: state.draft.currentName,
          name: state.draft.name,
          description: state.draft.description,
          env_vars: payload.envVars,
          disabled_env: payload.disabledEnv,
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
          os_env: [],
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
              "Create a profile to track env overrides and disabled OS variables.",
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
              "Each profile keeps its own env overrides and disabled OS variables.",
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

        const envSection = createNode("div", "profile-section profile-env-section");
        envSection.appendChild(createNode("div", "mock-label", "Environment Variables"));
        const envGrid = createNode("div", "profile-env-grid");
        const headerRow = createNode("div", "profile-env-grid-row profile-env-grid-head");
        for (const label of ["Key", "OS", "Mode", "Profile", "Result"]) {
          headerRow.appendChild(createNode("div", "", label));
        }
        envGrid.appendChild(headerRow);

        const rows = profileEnvironmentRows(snapshot, state.draft);
        rows.forEach((envRow, index) => {
          const row = createNode(
            "div",
            `profile-env-grid-row profile-env-row is-${envRow.mode}`,
          );

          if (envRow.kind === "os") {
            const keyCell = createNode("div", "profile-env-key", envRow.key);
            keyCell.title = envRow.key;
            row.appendChild(keyCell);
          } else {
            const keyInput = document.createElement("input");
            keyInput.type = "text";
            keyInput.placeholder = "KEY";
            keyInput.value = envRow.key;
            keyInput.setAttribute("aria-label", `Environment variable key, row ${index + 1}`);
            keyInput.addEventListener("input", () => {
              if (envRow.kind === "pending") {
                state.draft.envVars[envRow.draftIndex].key = keyInput.value;
                envRow.key = keyInput.value;
              } else if (envRow.mode === "disabled") {
                const previous = normalizeProfileEnvKey(envRow.key);
                const target = (state.draft.disabledEnv || []).findIndex(
                  (entry) => normalizeProfileEnvKey(entry) === previous,
                );
                if (target >= 0) {
                  state.draft.disabledEnv[target] = keyInput.value;
                  envRow.key = keyInput.value;
                }
              } else {
                const previous = normalizeProfileEnvKey(envRow.key);
                const target = (state.draft.envVars || []).find(
                  (entry) => normalizeProfileEnvKey(entry.key) === previous,
                );
                if (target) {
                  target.key = keyInput.value;
                  envRow.key = keyInput.value;
                }
              }
              scheduleProfileSave(windowId);
            });
            keyInput.addEventListener("blur", () => {
              flushProfileSave(windowId);
            });
            row.appendChild(keyInput);
          }

          const osCell = createNode("div", "profile-env-os-value", envRow.osValue || "-");
          osCell.title = envRow.osValue || "";
          row.appendChild(osCell);

          const modeSelect = document.createElement("select");
          modeSelect.setAttribute("aria-label", `Environment variable mode, row ${index + 1}`);
          const modeOptions =
            envRow.kind === "os"
              ? [
                  ["use_os", "Use OS"],
                  ["override", "Override"],
                  ["disabled", "Disabled"],
                ]
              : [
                  ["override", "Enabled"],
                  ["disabled", "Disabled"],
                ];
          for (const option of modeOptions) {
            const element = document.createElement("option");
            element.value = option[0];
            element.textContent = option[1];
            modeSelect.appendChild(element);
          }
          modeSelect.value = envRow.mode;
          modeSelect.addEventListener("change", () => {
            const rowKey =
              envRow.kind === "pending"
                ? state.draft.envVars[envRow.draftIndex]?.key
                : envRow.key;
            if (envRow.kind === "pending" && !normalizeProfileEnvKey(rowKey)) {
              if (modeSelect.value === "use_os") {
                state.draft.envVars.splice(envRow.draftIndex, 1);
              }
            } else {
              setProfileRowMode(state.draft, rowKey, modeSelect.value);
            }
            renderProfile(windowId, true);
            scheduleProfileSave(windowId);
          });
          row.appendChild(modeSelect);

          const valueInput = document.createElement("input");
          valueInput.type = "text";
          valueInput.placeholder = "Profile";
          valueInput.value = envRow.profileValue;
          valueInput.setAttribute("aria-label", `Profile value, row ${index + 1}`);
          const resultCell = createNode("div", "profile-env-result", envRow.result);
          resultCell.title = envRow.result;
          valueInput.addEventListener("input", () => {
            if (envRow.kind === "pending") {
              state.draft.envVars[envRow.draftIndex].value = valueInput.value;
              resultCell.textContent = valueInput.value;
            } else {
              setProfileEnvOverride(state.draft, envRow.key, valueInput.value);
              modeSelect.value = "override";
              resultCell.textContent = valueInput.value;
            }
            scheduleProfileSave(windowId);
          });
          valueInput.addEventListener("blur", () => flushProfileSave(windowId));
          row.appendChild(valueInput);
          row.appendChild(resultCell);
          envGrid.appendChild(row);
        });

        const addRow = createNode("button", "profile-env-add-row", "+ Add variable");
        addRow.type = "button";
        addRow.addEventListener("click", () => {
          state.draft.envVars.push({ key: "", value: "" });
          renderProfile(windowId, true);
        });
        envGrid.appendChild(addRow);
        envSection.appendChild(envGrid);
        editor.appendChild(envSection);

        updateProfileStatus(windowId);
      }

      // SPEC-2959: the known Work lanes available to the Board, derived from
      // the active Work projection. Used for lane labels and the composer
      // "To:" selector.
      function boardLaneWorkspaces() {
        const works = Array.isArray(activeWorkProjection?.active_works)
          ? activeWorkProjection.active_works
          : [];
        const result = [];
        for (const work of works) {
          const id = String(work?.id || "").trim();
          if (!id) continue;
          const agents = Array.isArray(work?.agents) ? work.agents : [];
          const titleSummary =
            agents.map((agent) => String(agent?.title_summary || "").trim()).find(Boolean) || "";
          const branch =
            String(work?.branch || "").trim() ||
            agents.map((agent) => String(agent?.branch || "").trim()).find(Boolean) ||
            "";
          result.push({
            id,
            titleSummary,
            title: String(work?.title || "").trim(),
            branch,
            lifecycle: String(work?.lifecycle_stage || "").trim(),
          });
        }
        return result;
      }

      // SPEC-2959 FR-018: resolve the composer "To:" value. An explicit, still
      // valid selection wins; otherwise default to the active Work, else General.
      function boardComposerTarget(state) {
        const ids = new Set(boardLaneWorkspaces().map((work) => work.id));
        const explicit = state?.composerTarget;
        if (explicit === GENERAL_LANE_KEY) return GENERAL_LANE_KEY;
        if (explicit && ids.has(explicit)) return explicit;
        const active = Array.isArray(currentProjectWorkspaceId)
          ? currentProjectWorkspaceId.find((id) => ids.has(id))
          : null;
        return active || GENERAL_LANE_KEY;
      }

      function submitBoardEntry(windowId) {
        const state = ensureBoardState(windowId);
        const body = state.composerBody.trim();
        if (!body) {
          state.error = "Entry body is required.";
          renderBoard(windowId);
          return;
        }
        // SPEC-2963: optional post title/subject.
        const title = (state.composerTitle || "").trim();
        const mentions = mentionsForBoardSubmit(state);
        state.loading = true;
        state.submitting = true;
        state.error = "";
        const parentId = state.replyParentId || null;
        state.pendingSubmit = {
          body,
          title,
          parentId,
          existingEntryIds: new Set(state.entries.map((entry) => entry.id)),
        };
        // SPEC-2959 FR-018..021: resolve the composer "To:" selection into the
        // post's lane. General → broadcast (empty audience); a Work id pins the
        // post to that lane; an empty selection lets the backend use the active
        // workspace default.
        const target = boardComposerTarget(state);
        const broadcast = target === GENERAL_LANE_KEY;
        const targetWorkspace =
          !broadcast && target && target !== "__default__" ? target : null;
        send({
          kind: "post_board_entry",
          id: windowId,
          entry_kind: state.composerKind,
          body,
          title: title || null,
          parent_id: parentId,
          topics: [],
          owners: [],
          mentions,
          target_workspace: targetWorkspace,
          broadcast,
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
        const allFilter = body.querySelector("[data-action='toggle-board-all']");
        const forYouFilter = body.querySelector("[data-action='toggle-board-for-you']");
        const workspaceFilter = body.querySelector("[data-action='toggle-board-workspace']");
        if (!status || !timeline || !composer) {
          return;
        }
        if (pendingBoardEntryFocusId && !state.focusEntryId) {
          state.focusEntryId = pendingBoardEntryFocusId;
          state.pendingFocusScroll = true;
        }
        state.currentWorkspaceId = currentProjectWorkspaceId;

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
        if (allFilter) {
          allFilter.setAttribute(
            "aria-pressed",
            state.audienceFilter === "all" ? "true" : "false",
          );
          allFilter.classList.toggle("active", state.audienceFilter === "all");
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
        if (workspaceFilter) {
          const workspaceActive = state.audienceFilter === "workspace";
          workspaceFilter.setAttribute("aria-pressed", workspaceActive ? "true" : "false");
          workspaceFilter.classList.toggle("active", workspaceActive);
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
                : state.audienceFilter === "workspace"
                  ? "No posts in this Work."
                : "No coordination entries yet.",
            ),
          );
        }
        let focusTarget = null;
        // SPEC-2959: build a single message card. Reused for every lane so the
        // bubble layout, reply quote, for-you highlight, and origin actions stay
        // identical to the previous flat timeline (FR-017).
        const buildBoardMessageCard = (entry) => {
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
          const originLabel = boardEntryOriginLabel(entry);
          if (originLabel) {
            meta.appendChild(createNode("span", "board-origin-badge", originLabel));
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
          if (entry.title) {
            card.appendChild(createNode("div", "board-message-title", entry.title));
          }
          // SPEC-2963: the body is authored in Markdown; render the
          // server-sanitized `body_html` (falls back to plaintext when absent),
          // reusing the Knowledge surface's markdown renderer.
          card.appendChild(createKnowledgeMarkdownBody(entry, "board-message-body"));
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
          const originActionLabel = boardEntryOriginActionLabel(
            entry,
            boardOriginActiveAgents(),
          );
          if (originActionLabel) {
            const originButton = createNode("button", "board-origin-button", originActionLabel);
            originButton.type = "button";
            originButton.addEventListener("click", () => openBoardOriginAgent(windowId, entry));
            messageActions.appendChild(originButton);
          }
          card.appendChild(messageActions);
          return card;
        };

        // SPEC-2959: group the visible entries into Work lanes and render each
        // lane with a collapsible header. Active/recent lanes are expanded;
        // Done/Archived lanes default to collapsed (FR-009/012/013/014).
        if (!state.collapsedLanes) state.collapsedLanes = {};
        if (!state.laneSeen) state.laneSeen = {};
        const lanes = groupBoardLanes(visibleEntries, {
          workspaces: boardLaneWorkspaces(),
        });
        for (const lane of lanes) {
          const explicit = state.collapsedLanes[lane.key];
          const collapsed = explicit === true || explicit === false ? explicit : lane.isDone;
          if (state.laneSeen[lane.key] === undefined) {
            state.laneSeen[lane.key] = lane.entries.length;
          }
          const unread = collapsed
            ? Math.max(0, lane.entries.length - state.laneSeen[lane.key])
            : 0;
          if (!collapsed) {
            state.laneSeen[lane.key] = lane.entries.length;
          }

          const laneEl = createNode("section", "board-lane");
          laneEl.dataset.laneKey = lane.key;
          if (lane.isGeneral) laneEl.classList.add("general");
          if (lane.isDone) laneEl.classList.add("done");
          if (collapsed) laneEl.classList.add("collapsed");

          const header = createNode("button", "board-lane-header");
          header.type = "button";
          header.setAttribute("aria-expanded", collapsed ? "false" : "true");
          header.appendChild(
            createNode("span", "board-lane-caret", collapsed ? "▸" : "▾"),
          );
          header.appendChild(createNode("span", "board-lane-label", lane.label));
          header.appendChild(
            createNode("span", "board-lane-count", String(lane.entries.length)),
          );
          if (unread > 0) {
            const badge = createNode("span", "board-lane-unread", String(unread));
            badge.setAttribute("aria-label", `${unread} unread in ${lane.label}`);
            header.appendChild(badge);
          }
          header.addEventListener("click", () => {
            state.collapsedLanes[lane.key] = !collapsed;
            renderBoard(windowId);
          });
          laneEl.appendChild(header);

          if (!collapsed) {
            const laneBody = createNode("div", "board-lane-body");
            for (const entry of lane.entries) {
              laneBody.appendChild(buildBoardMessageCard(entry));
            }
            laneEl.appendChild(laneBody);
          }
          timeline.appendChild(laneEl);
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

        // SPEC-2959 FR-018/019: composer "To:" selector — default active Work,
        // with other Works and General (broadcast) selectable.
        const toField = createNode("label", "board-composer-to");
        toField.appendChild(createNode("span", "mock-label", "To"));
        const toSelect = document.createElement("select");
        toSelect.className = "board-composer-to-select settings-select";
        const generalOption = document.createElement("option");
        generalOption.value = GENERAL_LANE_KEY;
        generalOption.textContent = "General (broadcast)";
        toSelect.appendChild(generalOption);
        for (const ws of boardLaneWorkspaces()) {
          const option = document.createElement("option");
          option.value = ws.id;
          option.textContent = ws.titleSummary || ws.title || ws.branch || ws.id;
          toSelect.appendChild(option);
        }
        toSelect.value = boardComposerTarget(state);
        toSelect.addEventListener("change", (event) => {
          state.composerTarget = event.target.value;
        });
        toField.appendChild(toSelect);
        composer.appendChild(toField);

        // SPEC-2963: optional post title/subject (Teams subject / Slack header
        // block / board card heading). Slack caps the header at 150 chars.
        const titleField = createNode("label", "board-composer-field board-composer-title-field");
        titleField.appendChild(createNode("span", "mock-label", "Title (optional)"));
        const titleInput = document.createElement("input");
        titleInput.type = "text";
        titleInput.className = "board-title-input";
        titleInput.maxLength = 150;
        titleInput.value = state.composerTitle || "";
        titleInput.placeholder = "Short subject for this post";
        titleInput.addEventListener("input", () => {
          state.composerTitle = titleInput.value;
        });
        titleField.appendChild(titleInput);
        composer.appendChild(titleField);

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

      // SPEC-2014 2026-05-29 amendment — operation-appropriate controls.
      // These delegate to the standalone launch-controls.js builders so the
      // control logic stays unit-testable; the wizard action payloads are
      // unchanged from the prior native <select> / checkbox controls.
      function appendReasoningField(parent, label, options, selectedValue, onChange) {
        const field = buildReasoningField(document, {
          label,
          options,
          selectedValue,
          onChange,
        });
        parent.appendChild(field);
        return field;
      }

      function appendToggleField(parent, label, copy, checked, onChange, wide = false) {
        const field = buildToggleField(document, {
          label,
          copy,
          checked,
          onChange,
          wide,
        });
        parent.appendChild(field);
        return field;
      }

      function appendChoiceOrSelectField(
        parent,
        label,
        options,
        selectedValue,
        onChange,
        wide = false,
      ) {
        const field = buildChoiceOrSelectField(document, {
          label,
          options,
          selectedValue,
          onChange,
          wide,
        });
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

      // SPEC-2014 FR-128: progress rail step key → wizard phase. Clicking a
      // reachable step dispatches goto_step with the mapped phase so the
      // backend can jump the ManualSetup (Setup 3-step) wizard between
      // Path / Settings / Runtime / Confirm without re-walking each step.
      const WIZARD_RAIL_STEP_PHASE = Object.freeze({
        path: "path",
        setup: "settings",
        runtime: "runtime",
        start: "confirm",
      });

      function gotoWizardStep(phase) {
        if (
          !releaseWizardInteractionGuardForChromeAction()
          || launchWizardOpenError
          || launchWizardPendingAction
        ) {
          return;
        }
        frontendUnits.launchWizardSurface.flushBranchDraft();
        sendWizardAction({ kind: "goto_step", phase });
      }

      function renderWizardProgressRail() {
        const rail = createNode("aside", "wizard-progress-rail");
        rail.setAttribute("aria-label", "Launch progress");
        for (const step of launchWizard.progress_steps || []) {
          const item = createNode("div", "wizard-progress-step");
          const state = step.state || "pending";
          item.dataset.state = state;
          // SPEC-2014 FR-128 — only reached steps (active/done) are
          // navigable; pending steps stay inert. A null phase mapping
          // (unknown key) also stays inert.
          const targetPhase = WIZARD_RAIL_STEP_PHASE[step.key];
          const isClickable =
            Boolean(targetPhase) && (state === "active" || state === "done");
          if (isClickable) {
            item.dataset.clickable = "true";
            item.setAttribute("role", "button");
            item.setAttribute("tabindex", "0");
            item.setAttribute(
              "aria-label",
              `Go to ${step.label} step`,
            );
            const jump = () => gotoWizardStep(targetPhase);
            item.addEventListener("click", jump);
            item.addEventListener("keydown", (event) => {
              if (event.key === "Enter" || event.key === " " || event.key === "Spacebar") {
                event.preventDefault();
                jump();
              }
            });
          }
          item.appendChild(createNode("span", "wizard-progress-marker"));
          const copy = createNode("span", "wizard-progress-copy");
          copy.appendChild(createNode("span", "wizard-progress-label", step.label));
          if (step.detail) {
            copy.appendChild(createNode("span", "wizard-progress-detail", step.detail));
          }
          item.appendChild(copy);
          rail.appendChild(item);
        }
        return rail;
      }

      let wizardFocusReturn = null;
      let wizardFocusTrapRelease = null;

      function closeLaunchWizardLocal() {
        clearLaunchWizardPendingAction();
        clearLaunchWizardOpening();
        launchWizard = null;
        launchWizardOpenError = null;
        // Issue #2698 PR 1 (B7) — local close wins over any pending
        // backend state. Discard (do not replay) the deferred event
        // so we don't undo the user-initiated close.
        wizardInteractionGuard.discard();
        renderLaunchWizard();
      }

      function releaseWizardInteractionGuardForChromeAction() {
        if (wizardInteractionGuard.isActive()) {
          wizardInteractionGuard.release();
        }
        return Boolean(launchWizard || launchWizardOpenError);
      }

      function setLaunchWizardPendingAction(action) {
        launchWizardPendingAction = action || null;
        renderLaunchWizard();
      }

      function clearLaunchWizardPendingAction() {
        launchWizardPendingAction = null;
      }

      function openStartWorkPendingWizard() {
        clearLaunchWizardPendingAction();
        launchWizard = null;
        launchWizardOpenError = null;
        launchWizardOpening = {
          title: "Start Work",
          meta: "Work launch",
          message: "Preparing Start Work...",
        };
        renderLaunchWizard();
      }

      function clearLaunchWizardOpening() {
        launchWizardOpening = null;
      }

      function syncLaunchWizardPendingChrome(isPending) {
        wizardModal.classList.toggle("is-launch-pending", isPending);
        if (wizardDialog) {
          wizardDialog.classList.toggle("is-launch-pending", isPending);
          wizardDialog.setAttribute("aria-busy", isPending ? "true" : "false");
        }
      }

      function closeLaunchWizardFromChrome() {
        if (!releaseWizardInteractionGuardForChromeAction()) {
          return;
        }
        clearLaunchWizardPendingAction();
        if (launchWizardOpenError) {
          closeLaunchWizardLocal();
          return;
        }
        frontendUnits.launchWizardSurface.sendAction({ kind: "cancel" });
      }

      function isPrimaryPointerActivation(event) {
        return !event || event.button === 0 || event.button === undefined;
      }

      function handleLaunchWizardSubmitFromChrome() {
        if (
          !releaseWizardInteractionGuardForChromeAction()
          || launchWizardOpenError
          || wizardSubmitButton.disabled
        ) {
          return;
        }
        frontendUnits.launchWizardSurface.flushBranchDraft();
        setLaunchWizardPendingAction({ kind: "submit" });
        frontendUnits.launchWizardSurface.sendAction({ kind: "submit" });
      }

      function renderLaunchWizard() {
        if (!launchWizard && !launchWizardOpenError && !launchWizardOpening) {
          clearLaunchWizardPendingAction();
          syncLaunchWizardPendingChrome(false);
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
          wizardSummary.innerHTML = "";
          wizardBody.innerHTML = "";
          wizardError.hidden = true;
          wizardError.textContent = "";
          if (wizardTitle) wizardTitle.textContent = "Launch Agent";
          wizardSubmitButton.textContent = "Launch";
          wizardSubmitButton.disabled = false;
          wizardSubmitButton.hidden = false;
          wizardBackButton.hidden = true;
          wizardBackButton.disabled = false;
          wizardCancelButton.textContent = "Cancel";
          wizardCancelButton.disabled = false;
          syncWizardDraftState();
          return;
        }

        syncWizardDraftState();
        closeModal();
        const isLaunchActionPending = Boolean(launchWizardPendingAction);
        const isLaunchOpeningPending = Boolean(launchWizardOpening);
        const isLaunchSubmitPending = launchWizardPendingAction?.kind === "submit";
        syncLaunchWizardPendingChrome(isLaunchActionPending || isLaunchOpeningPending);
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

        if (launchWizardOpening) {
          if (wizardTitle) {
            wizardTitle.textContent = launchWizardOpening.title || "Start Work";
          }
          wizardMeta.textContent = launchWizardOpening.meta || "Work launch";
          wizardBackButton.hidden = true;
          wizardBackButton.disabled = true;
          wizardSubmitButton.hidden = true;
          wizardSubmitButton.disabled = true;
          wizardCancelButton.textContent = "Cancel";
          wizardCancelButton.disabled = true;
          wizardError.hidden = true;
          wizardError.textContent = "";
          wizardSummary.innerHTML = "";
          wizardBody.innerHTML = "";
          const openingPanel = createNode("div", "launch-panel wizard-disabled");
          openingPanel.appendChild(
            createNode(
              "div",
              "launch-note launch-pending-note",
              launchWizardOpening.message || "Preparing Start Work...",
            ),
          );
          wizardBody.appendChild(openingPanel);
          return;
        }

        if (launchWizardOpenError) {
          if (wizardTitle) {
            wizardTitle.textContent = launchWizardOpenError.title || "Launch Agent";
          }
          wizardMeta.textContent =
            launchWizardOpenError.title === "Start Work"
              ? "Work launch"
              : "Launch Agent";
          wizardBackButton.hidden = true;
          wizardBackButton.disabled = false;
          wizardSubmitButton.hidden = true;
          wizardSubmitButton.disabled = true;
          wizardCancelButton.textContent = "Close";
          wizardCancelButton.disabled = false;
          wizardError.hidden = false;
          wizardError.textContent =
            launchWizardOpenError.message || "Unable to open Launch Wizard";
          wizardSummary.innerHTML = "";
          wizardBody.innerHTML = "";
          return;
        }

        wizardSubmitButton.hidden = false;
        wizardBackButton.hidden = !launchWizard.show_back_button;
        wizardBackButton.disabled = Boolean(
          isLaunchActionPending
            || launchWizard.is_hydrating
            || launchWizard.runtime_resolution_pending
            || !launchWizard.show_back_button,
        );
        wizardCancelButton.textContent = "Cancel";
        if (wizardTitle) wizardTitle.textContent = launchWizard.title || "Launch Agent";
        wizardMeta.textContent = launchWizard.show_branch_controls === false
          ? "Work launch"
          : `Selected branch · ${
            launchWizard.selected_branch_name || launchWizard.branch_name || "Work"
          }`;
        wizardSubmitButton.textContent = isLaunchSubmitPending
          ? "Launching..."
          : launchWizard.primary_action_label || (
          launchWizard.is_hydrating
            ? "Loading..."
            : launchWizard.runtime_context_resolved === false
              ? "Continue"
              : launchWizard.branch_mode === "create_new"
                ? "Create and launch"
                : "Launch"
        );
        wizardSubmitButton.disabled = Boolean(
          isLaunchActionPending
            || launchWizard.is_hydrating
            || launchWizard.runtime_resolution_pending
            || launchWizard.primary_action_enabled === false,
        );
        wizardCancelButton.disabled = false;

        if (launchWizard.error || launchWizard.hydration_error) {
          wizardError.hidden = false;
          wizardError.textContent =
            launchWizard.error || launchWizard.hydration_error;
        } else {
          wizardError.hidden = true;
          wizardError.textContent = "";
        }

        renderWizardSummary();
        // SPEC-2014 FR-126/FR-127 — the backend now drives four mutually
        // exclusive wizard phases through dedicated flags:
        //   show_manual_setup       → Settings form
        //   show_runtime_confirmation→ Runtime step
        //   show_confirm             → Confirm (read-only summary + Launch)
        //   show_start_methods       → entry (start method picker)
        // The backend already strictly clears show_manual_setup during
        // Runtime / Confirm, but we re-derive exclusive locals here so the
        // renderer never paints two phases at once.
        const showConfirm = Boolean(launchWizard.show_confirm);
        const isRuntimeConfirmation = Boolean(
          launchWizard.runtime_context_resolved
          && launchWizard.show_runtime_confirmation
          && !showConfirm
        );
        const showManualSetup =
          launchWizard.show_manual_setup !== false
          && !isRuntimeConfirmation
          && !showConfirm;
        const showStartMethods = Boolean(
          launchWizard.show_start_methods
            && !isRuntimeConfirmation
            && !showConfirm
            && !launchWizard.runtime_resolution_pending
            && (launchWizard.start_methods || []).length > 0,
        );
        const showSetupForms = showManualSetup && !isRuntimeConfirmation;
        wizardBody.innerHTML = "";
        const wizardMain = createNode("div", "wizard-main");
        wizardMain.appendChild(renderWizardProgressRail());
        const wizardContentPane = createNode("div", "wizard-content-pane");
        const panel = createNode("div", "launch-panel");
        const isRuntimeResolutionPending = Boolean(launchWizard.runtime_resolution_pending);
        panel.classList.toggle(
          "wizard-disabled",
          isRuntimeResolutionPending || isLaunchActionPending,
        );
        if (launchWizard.is_hydrating) {
          panel.appendChild(
            createNode(
              "div",
              "launch-note",
              "Loading branch workspace, recent sessions, and Docker options...",
            ),
          );
        }
        if (launchWizard.runtime_resolution_pending) {
          panel.appendChild(
            createNode(
              "div",
              "launch-note",
              launchWizard.runtime_resolution_message || "Preparing runtime...",
            ),
          );
        }
        if (isLaunchSubmitPending) {
          panel.appendChild(
            createNode(
              "div",
              "launch-note launch-pending-note",
              "Creating agent window...",
            ),
          );
        }

        // SPEC-2014 FR-127 — Confirm step: a read-only review of the
        // resolved launch configuration plus the footer Launch button.
        // No editable controls are rendered here; the user revisits an
        // earlier phase (via the progress rail or Back) to change anything.
        if (showConfirm) {
          const section = createLaunchSection(
            "Confirm",
            "Review the launch configuration. Use the steps above or Back to change anything.",
          );
          const summaryList = createNode("div", "wizard-confirm-summary");
          for (const item of launchWizard.launch_summary || []) {
            const card = createNode("div", "wizard-summary-item");
            card.appendChild(createNode("div", "wizard-summary-label", item.label));
            card.appendChild(createNode("div", "wizard-summary-value", item.value));
            summaryList.appendChild(card);
          }
          section.appendChild(summaryList);
          panel.appendChild(section);
        }

        if (showStartMethods) {
          const section = createLaunchSection(
            "Start methods",
            "Choose how this agent should start on the selected branch.",
          );

          const methodList = createNode("div", "start-method-list");
          for (const method of launchWizard.start_methods || []) {
            const button = createNode("button", "start-method-button");
            button.type = "button";
            const isStartMethodPending =
              launchWizardPendingAction?.kind === "use_start_method"
                && launchWizardPendingAction.method === method.kind;
            button.classList.toggle("is-pending", isStartMethodPending);
            button.disabled = method.enabled === false || isLaunchActionPending;
            const head = createNode("div", "start-method-head");
            head.appendChild(
              createNode(
                "div",
                "start-method-title",
                isStartMethodPending ? "Preparing..." : method.label,
              ),
            );
            if (method.badge) {
              head.appendChild(createNode("div", "start-method-badge", method.badge));
            }
            button.appendChild(head);
            button.appendChild(
              createNode("div", "start-method-summary", method.summary || ""),
            );
            const detail = method.enabled === false
              ? method.disabled_reason
              : method.detail;
            if (detail) {
              button.appendChild(createNode("div", "start-method-detail", detail));
            }
            const handleStartMethodLaunchAction = () => {
              if (
                !releaseWizardInteractionGuardForChromeAction()
                || button.disabled
                || launchWizardPendingAction
              ) {
                return;
              }
              setLaunchWizardPendingAction({
                kind: "use_start_method",
                method: method.kind,
              });
              sendWizardAction({
                kind: "use_start_method",
                method: method.kind,
              });
            };
            button.addEventListener("pointerup", (event) => {
              if (!isPrimaryPointerActivation(event)) {
                return;
              }
              event.preventDefault();
              handleStartMethodLaunchAction();
            });
            button.addEventListener("click", () => {
              handleStartMethodLaunchAction();
            });
            methodList.appendChild(button);
          }
          section.appendChild(methodList);

          panel.appendChild(section);
        }

        if (showSetupForms && launchWizard.show_branch_controls !== false) {
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

        if (showSetupForms) {
          const section = createLaunchSection(
            "Launch",
            "Choose what to launch on the selected branch.",
          );
          const grid = createNode("div", "launch-form-grid");
          appendChoiceOrSelectField(
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
            appendChoiceOrSelectField(
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
              appendReasoningField(
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
              appendChoiceOrSelectField(
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
            appendChoiceOrSelectField(
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
          showSetupForms &&
          (
            launchWizard.show_version ||
            launchWizard.show_skip_permissions ||
            launchWizard.show_fast_mode ||
            launchWizard.show_codex_fast_mode
          )
        ) {
          const showFastMode = Boolean(
            launchWizard.show_fast_mode ?? launchWizard.show_codex_fast_mode,
          );
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
            appendToggleField(
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
          if (showFastMode) {
            appendToggleField(
              grid,
              "Fast mode",
              "Use the agent's Fast mode",
              Boolean(launchWizard.fast_mode ?? launchWizard.codex_fast_mode),
              (enabled) =>
                sendWizardAction({
                  kind: "set_fast_mode",
                  enabled,
                }),
            );
          }
          section.appendChild(grid);
          panel.appendChild(section);
        }

        // SPEC-2014 Amendment 2026-05-20 (FR-057 / FR-058):
        // The Linked issue section renders only when the wizard was opened
        // through the Knowledge Issue Bridge. The issue number is shown as
        // read-only text instead of an editable input.
        if (showSetupForms && launchWizard.show_linked_issue) {
          const section = createLaunchSection(
            "Linked issue",
            "Read-only: this agent will be linked to the originating issue.",
          );
          const grid = createNode("div", "launch-form-grid");
          const field = createLaunchField("Issue number", false);
          // The launch-field label already announces "Issue number"; the
          // static value div is read alongside that label so SR users hear
          // "Issue number, #N" without needing a per-value aria-label.
          const value = createNode("div", "launch-static-value");
          value.textContent = `#${launchWizard.linked_issue_number}`;
          field.appendChild(value);
          grid.appendChild(field);
          section.appendChild(grid);
          panel.appendChild(section);
        }

        const hasRuntimeControls =
          launchWizard.show_runtime_target ||
          (launchWizard.show_docker_service &&
            (launchWizard.docker_service_options || []).length > 0) ||
          (launchWizard.show_docker_lifecycle &&
            (launchWizard.docker_lifecycle_options || []).length > 0);
        if (
          launchWizard.show_runtime_confirmation &&
          !showConfirm &&
          (hasRuntimeControls || isRuntimeConfirmation || !showManualSetup)
        ) {
          const section = createLaunchSection(
            "Runtime",
            "Choose where the session runs and how Docker services are used.",
          );
          const grid = createNode("div", "launch-form-grid");
          let appendedRuntimeControl = false;
          if (launchWizard.show_runtime_target) {
            appendChoiceOrSelectField(
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
            appendedRuntimeControl = true;
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
            appendedRuntimeControl = true;
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
            appendedRuntimeControl = true;
          }
          if (!appendedRuntimeControl) {
            const note = createLaunchField("Runtime target", true);
            note.appendChild(
              createNode(
                "div",
                "launch-note",
                launchWizard.selected_runtime_target === "docker"
                  ? "Docker"
                  : "Host",
              ),
            );
            grid.appendChild(note);
          }
          section.appendChild(grid);

          panel.appendChild(section);
        }

        setLaunchWizardPendingDisabled(
          panel,
          isRuntimeResolutionPending || isLaunchActionPending,
        );
        wizardContentPane.appendChild(panel);
        wizardMain.appendChild(wizardContentPane);
        wizardBody.appendChild(wizardMain);
      }

      function applyFileTreeSplitterRatio(split, ratio) {
        if (!split) return;
        const clamped = Math.min(0.9, Math.max(0.1, Number(ratio) || 0.4));
        const leftPercent = (clamped * 100).toFixed(2);
        split.style.setProperty("--file-tree-left-ratio", leftPercent + "%");
        split.dataset.leftRatio = String(clamped);
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
              } else {
                // SPEC-2009 amendment Phase 2: gate navigation behind the
                // Discard modal when the viewer has unsaved edits.
                if (
                  queueNavigationGuardedByDirty(windowId, {
                    kind: "switch_file",
                    path: entry.path,
                  })
                ) {
                  return;
                }
                beginViewerForFile(windowId, entry.path);
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

        const actions = document.createElement("div");
        actions.className = "branch-row-actions";
        actions.addEventListener("click", (event) => event.stopPropagation());
        actions.addEventListener("dblclick", (event) => event.stopPropagation());

        const resumeButton = document.createElement("button");
        resumeButton.type = "button";
        resumeButton.className = "branch-row-action";
        resumeButton.textContent = "Resume";
        resumeButton.setAttribute("data-branch-row-action", "resume");
        actions.appendChild(resumeButton);

        const launchButton = document.createElement("button");
        launchButton.type = "button";
        launchButton.className = "branch-row-action primary";
        launchButton.textContent = "Launch";
        launchButton.setAttribute("data-branch-row-action", "launch");
        actions.appendChild(launchButton);

        row.appendChild(actions);

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
          actions,
          resumeButton,
          launchButton,
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
        const resume = () => {
          select();
          send({
            kind: "resume_branch_latest_agent",
            id: windowId,
            branch_name: branchName,
            bounds: visibleBounds(),
          });
        };
        resumeButton.addEventListener("click", (event) => {
          event.stopPropagation();
          if (resumeButton.disabled) return;
          resume();
        });
        launchButton.addEventListener("click", (event) => {
          event.stopPropagation();
          activate();
        });
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

        const resumeAvailable = entry.resume && entry.resume.available;
        const resumeReason = entry.resume?.reason || "No resumable session";
        fields.resumeButton.disabled = !resumeAvailable;
        fields.resumeButton.title = resumeAvailable
          ? `Resume latest agent on ${entry.name}`
          : resumeReason;
        fields.resumeButton.setAttribute(
          "aria-label",
          resumeAvailable
            ? `Resume latest agent on ${entry.name}`
            : `Resume unavailable for ${entry.name}: ${resumeReason}`,
        );
        fields.launchButton.title = `Launch Agent on ${entry.name}`;
        fields.launchButton.setAttribute("aria-label", `Launch Agent on ${entry.name}`);
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
        renderBranchLoadStatusSummary(notice, branchLoadStatusSummary(state));

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

      function knowledgeSearchPlaceholder(kind) {
        switch (kind) {
          case "issue":
            return "Semantic search issues";
          case "spec":
            return "Semantic search cached SPECs";
          case "pr":
            return "Search unavailable";
          default:
            return "Search";
        }
      }

      const KNOWLEDGE_PHASES = new Set([
        "draft",
        "planning",
        "implementation",
        "review",
        "done",
      ]);

      function isKnowledgePhaseLabel(label) {
        return typeof label === "string" && label.startsWith("phase/");
      }

      function canonicalKnowledgePhase(phase) {
        const value = String(phase || "");
        return KNOWLEDGE_PHASES.has(value) ? value : null;
      }

      function knowledgePhaseFromLabels(labels = []) {
        for (const label of Array.isArray(labels) ? labels : []) {
          if (!isKnowledgePhaseLabel(label)) continue;
          const phase = canonicalKnowledgePhase(label.slice("phase/".length));
          if (phase) return phase;
        }
        return null;
      }

      function effectiveKnowledgePhase(entry) {
        if (entry?.state === "closed") return "done";
        return canonicalKnowledgePhase(entry?.phase)
          || knowledgePhaseFromLabels(entry?.labels)
          || "backlog";
      }

      function knowledgePhaseDisplayName(phase) {
        switch (phase) {
          case "draft":
            return "Draft";
          case "planning":
            return "Planning";
          case "implementation":
            return "Implementation";
          case "review":
            return "Review";
          case "done":
            return "Done";
          default:
            return "Backlog";
        }
      }

      function visibleKnowledgeLabels(labels = []) {
        return (Array.isArray(labels) ? labels : []).filter(
          (label) => !isKnowledgePhaseLabel(label),
        );
      }

      function staleKnowledgePhaseWarning(entry) {
        const storedPhase = canonicalKnowledgePhase(entry?.phase)
          || knowledgePhaseFromLabels(entry?.labels);
        if (entry?.state === "closed" && storedPhase && storedPhase !== "done") {
          return `Stored phase/${storedPhase}; lifecycle is Done`;
        }
        return "";
      }

      function knowledgeDetailChip(detail) {
        const effectivePhase = effectiveKnowledgePhase(detail);
        const rawState = String(detail?.state || "").toLowerCase();
        if (
          rawState
          && rawState !== "open"
          && rawState !== "closed"
          && effectivePhase === "backlog"
        ) {
          return {
            className: rawState,
            label: rawState,
          };
        }
        return {
          className: effectivePhase === "done" ? "closed" : "open",
          label: knowledgePhaseDisplayName(effectivePhase),
        };
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
        const effectivePhase = effectiveKnowledgePhase(entry);
        // Plain (non-spec) Issues cannot be moved through phase columns
        // because they carry no canonical phase labels. We surface a
        // (plain) chip and disable HTML5 D&D so the user understands
        // the constraint at a glance.
        const isPlain = entry.is_spec === false;
        const isClosed = String(entry?.state || "").toLowerCase() === "closed";
        card.draggable = !isPlain && !isClosed;
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
        const phaseChip = createNode(
          "span",
          `kanban-card-chip kanban-card-chip--phase-${effectivePhase}`,
          knowledgePhaseDisplayName(effectivePhase),
        );
        head.appendChild(phaseChip);
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
          // The selected card stays in the split-pane detail view. We
          // always request detail (cheap; cache-backed) so selecting the
          // same card still pulls live comment / linked-branch updates.
          requestKnowledgeDetail(windowId, state.kind, entry.number);
          renderKnowledgeBridge(windowId);
        });

        // SPEC-2017 US-8 — D&D wire-up. Plain (is_spec=false) and closed
        // cards skip these handlers entirely (draggable=false above) so
        // they can still be clicked but never picked up.
        if (!isPlain && !isClosed) {
          card.addEventListener("dragstart", (event) => {
            // Snapshot the original entry so a failed write-back can
            // restore it; the snapshot keeps the entire entry value
            // because labels / phase / state all change on success.
            state.dndSnapshot = {
              issueNumber: entry.number,
              entry: { ...entry },
              originPhase: effectiveKnowledgePhase(entry),
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
        const hideDoneToggle = element.querySelector("[data-action='kanban-hide-done']");
        if (!board || !detailPane || !status || !refreshButton || !searchInput) {
          return;
        }

        refreshButton.disabled = !state.refreshEnabled || state.loading;
        searchInput.placeholder = knowledgeSearchPlaceholder(state.kind);
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
          const phaseKey = effectiveKnowledgePhase(entry);
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
        const detailChip = knowledgeDetailChip(detail);
        headRow.appendChild(
          createNode(
            "span",
            `knowledge-state-chip ${detailChip.className}`,
            detailChip.label,
          ),
        );
        head.appendChild(headRow);
        if (detail.subtitle) {
          head.appendChild(
            createNode("div", "knowledge-detail-subtitle", detail.subtitle),
          );
        }
        const displayLabels = visibleKnowledgeLabels(detail.labels || []);
        const stalePhase = staleKnowledgePhaseWarning(detail);
        if (displayLabels.length > 0 || stalePhase) {
          const labelRow = createNode("div", "knowledge-label-row");
          for (const label of displayLabels) {
            labelRow.appendChild(createNode("span", "knowledge-chip", label));
          }
          if (stalePhase) {
            labelRow.appendChild(
              createNode("span", "kanban-card-chip kanban-card-chip--warning", stalePhase),
            );
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
            createKnowledgeMarkdownBody(section),
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

      function initialBranchCleanupProgress(branches) {
        return {
          current: null,
          items: branches.map((branch) => ({
            branch,
            status: "pending",
            message: "",
          })),
        };
      }

      function updateBranchCleanupProgress(windowId, event) {
        const state = ensureBranchListState(windowId);
        const branches = Array.from(state.cleanupSelected);
        if (!state.cleanupModal.progress) {
          state.cleanupModal.progress = initialBranchCleanupProgress(
            branches.length > 0 ? branches : [event.branch],
          );
        }
        const progress = state.cleanupModal.progress;
        progress.current = {
          branch: event.branch,
          executionBranch: event.execution_branch || null,
          index: event.index,
          total: event.total,
          phase: event.phase,
          message: event.message || "",
        };
        let item = progress.items.find((candidate) => candidate.branch === event.branch);
        if (!item) {
          item = { branch: event.branch, status: "pending", message: "" };
          progress.items.push(item);
        }
        item.executionBranch = event.execution_branch || null;
        item.status = event.phase || "running";
        item.message = event.message || "";
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
            return "Only gwt-managed work can be cleaned up";
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

      const BRANCH_DETAIL_CHECK_INTERRUPTED_NOTICE = "Branch detail check interrupted";

      function branchLoadStatusSummary(state) {
        if (!state) {
          return null;
        }
        if (state.error) {
          return {
            kind: "error",
            title: "Branches unavailable",
            detail: state.error,
            hint: "Refresh to try again.",
          };
        }
        if (state.loading && state.entries.length > 0) {
          return {
            kind: "checking",
            title: "Checking branch details",
            detail: "Loading branch details while cleanup safety is checked.",
            hint: "Cleanup selection unlocks after verification.",
          };
        }
        if (state.notice === BRANCH_DETAIL_CHECK_INTERRUPTED_NOTICE) {
          return {
            kind: "interrupted",
            title: BRANCH_DETAIL_CHECK_INTERRUPTED_NOTICE,
            detail: "Branch names are available, but cleanup safety was not verified.",
            hint: "Refresh to verify cleanup safety.",
          };
        }
        if (state.notice) {
          return {
            kind: "notice",
            title: "Branch notice",
            detail: state.notice,
            hint: "",
          };
        }
        return null;
      }

      function renderBranchLoadStatusSummary(notice, summary) {
        if (!notice) {
          return;
        }
        notice.textContent = "";
        notice.hidden = !summary;
        if (!summary) {
          notice.removeAttribute("data-branch-status");
          return;
        }
        notice.dataset.branchStatus = summary.kind;

        const title = document.createElement("div");
        title.className = "branch-notice-title";
        title.textContent = summary.title;
        notice.appendChild(title);

        if (summary.detail) {
          const detail = document.createElement("div");
          detail.className = "branch-notice-detail";
          detail.textContent = summary.detail;
          notice.appendChild(detail);
        }

        if (summary.hint) {
          const hint = document.createElement("div");
          hint.className = "branch-notice-hint";
          hint.textContent = summary.hint;
          notice.appendChild(hint);
        }
      }

      function failLoadingBranchesOnConnectionLoss(windowId, state) {
        if (!state || !state.loading) {
          return false;
        }
        state.loading = false;
        state.receivedFreshEntries = false;
        if (state.entries.length === 0) {
          state.error = "Connection lost while loading branches";
          state.notice = "";
        } else {
          state.error = "";
          state.notice = BRANCH_DETAIL_CHECK_INTERRUPTED_NOTICE;
        }
        syncBranchSelectionState(state);
        return true;
      }

      function branchCleanupPendingText(state) {
        return state.loading ? "Checking cleanup safety" : "Refresh to verify cleanup safety";
      }

      function cleanupAvailabilityForRender(entry, state) {
        if (entry.cleanup_ready) {
          return entry.cleanup.availability;
        }
        if (state.loading) {
          return "loading";
        }
        return "unknown";
      }

      function cleanupBadgeText(entry, state) {
        return entry.cleanup_ready ? entry.cleanup.availability : state.loading ? "checking" : "Safety unknown";
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
        state.cleanupModal.forceFilesystemDelete = false;
        state.cleanupModal.progress = null;
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
        state.cleanupModal.forceFilesystemDelete = false;
        state.cleanupModal.progress = null;
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
        state.cleanupModal.progress = initialBranchCleanupProgress(branches);
        state.cleanupModal.results = [];
        renderBranchCleanupModal();
        if (windowId === WORKSPACE_CLEANUP_WINDOW_ID) {
          send({
            kind: "run_workspace_cleanup",
            branch: branches[0],
            delete_remote: state.cleanupModal.deleteRemote,
            force_filesystem_delete: state.cleanupModal.forceFilesystemDelete,
          });
          return;
        }
        send({
          kind: "run_branch_cleanup",
          id: windowId,
          branches,
          delete_remote: state.cleanupModal.deleteRemote,
          force_filesystem_delete: state.cleanupModal.forceFilesystemDelete,
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
          onForceFilesystemDeleteToggle: (checked) => {
            if (state) {
              state.cleanupModal.forceFilesystemDelete = checked;
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
          "surface-index",
          "surface-work",
          "surface-profile",
          "surface-console",
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
          // SPEC-2009 amendment: File Tree window now opens with a worktree
          // picker, then renders a single window split into a left directory
          // tree pane and a right file content viewer pane. The legacy
          // `.file-tree-root` class wraps the whole composition so existing
          // styles (and embedded HTML contract tests) still hit.
          const root = makeEl("div", { className: "file-tree-root file-tree-root--split" });
          const toolbar = makeEl("div", {
            className: "file-tree-toolbar workspace-toolbar",
          });
          const pathLabel = makeEl("button", {
            className: "file-tree-path file-tree-worktree-trigger",
            attrs: { type: "button" },
            dataset: { action: "open-worktree-picker" },
            text: "Select worktree…",
          });
          const refreshBtn = makeEl("button", {
            className: "icon-button",
            attrs: { "aria-label": "Refresh tree", type: "button" },
            dataset: { action: "refresh-tree" },
            text: "↻",
          });
          toolbar.appendChild(pathLabel);
          toolbar.appendChild(refreshBtn);

          const split = makeEl("div", { className: "file-tree-split" });
          const pane = makeEl("div", { className: "file-tree-pane" });
          const scroll = makeEl("div", { className: "file-tree-scroll workspace-scroll" });
          const list = makeEl("div", { className: "file-tree-list" });
          scroll.appendChild(list);
          pane.appendChild(scroll);
          pane.appendChild(makeEl("div", { className: "file-tree-footer", text: "." }));

          const splitter = makeEl("div", {
            className: "file-tree-splitter",
            attrs: { role: "separator", "aria-orientation": "vertical", tabindex: "0" },
            dataset: { action: "drag-splitter" },
          });

          const viewer = makeEl("div", { className: "file-tree-viewer" });
          viewer.appendChild(makeEl("div", { className: "file-tree-viewer-header" }));
          viewer.appendChild(makeEl("div", { className: "file-tree-viewer-body" }));

          split.appendChild(pane);
          split.appendChild(splitter);
          split.appendChild(viewer);

          root.appendChild(toolbar);
          root.appendChild(split);
          clearChildren(body);
          body.appendChild(root);

          // Apply initial splitter ratio.
          const initialState = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
            windowData.id,
          );
          applyFileTreeSplitterRatio(split, initialState.splitterRatio);

          body.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            socketTransport.send({ kind: "focus_window", id: windowData.id });
          });

          pathLabel.addEventListener("click", (event) => {
            event.stopPropagation();
            frontendUnits.branchesFileTreeSurface.openWorktreePicker(windowData.id);
          });

          refreshBtn.addEventListener("click", (event) => {
            event.stopPropagation();
            const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
              windowData.id,
            );
            if (!state.selectedWorktreeId) {
              frontendUnits.branchesFileTreeSurface.openWorktreePicker(windowData.id);
              return;
            }
            state.loaded.clear();
            state.expanded.clear();
            state.loading.clear();
            state.error = "";
            frontendUnits.branchesFileTreeSurface.requestFileTree(windowData.id, "");
            frontendUnits.branchesFileTreeSurface.renderFileTree(windowData.id);
          });

          // Splitter drag: pointer events keep the handler small and ignore
          // the canvas pan/zoom because the modal capture absorbs them.
          splitter.addEventListener("pointerdown", (event) => {
            event.preventDefault();
            splitter.setPointerCapture(event.pointerId);
            const onMove = (moveEvent) => {
              const rect = split.getBoundingClientRect();
              if (rect.width <= 0) return;
              const ratio = (moveEvent.clientX - rect.left) / rect.width;
              const clamped = Math.min(0.9, Math.max(0.1, ratio));
              const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
                windowData.id,
              );
              state.splitterRatio = clamped;
              applyFileTreeSplitterRatio(split, clamped);
            };
            const onUp = () => {
              splitter.releasePointerCapture(event.pointerId);
              splitter.removeEventListener("pointermove", onMove);
              splitter.removeEventListener("pointerup", onUp);
              splitter.removeEventListener("pointercancel", onUp);
            };
            splitter.addEventListener("pointermove", onMove);
            splitter.addEventListener("pointerup", onUp);
            splitter.addEventListener("pointercancel", onUp);
          });

          // Initial state: prompt for worktree selection. The picker fires
          // the first directory load once the user picks.
          if (!initialState.selectedWorktreeId) {
            pathLabel.textContent = "Select worktree…";
            frontendUnits.branchesFileTreeSurface.openWorktreePicker(windowData.id);
          } else {
            pathLabel.textContent = initialState.selectedWorktreeLabel || "Worktree";
            if (!initialState.loaded.has("")) {
              frontendUnits.branchesFileTreeSurface.requestFileTree(windowData.id, "");
            }
          }
          frontendUnits.branchesFileTreeSurface.renderFileTree(windowData.id);
          frontendUnits.branchesFileTreeSurface.renderFileTreeViewer(windowData.id);

          // SPEC-2009 amendment Phase 2 FR-033/041: Ctrl+S / Cmd+S triggers
          // a save when focus is inside this File Tree window so other
          // windows' inputs are not stolen. Bound on the window body element
          // because keydown bubbles up from the textarea / hex cells.
          body.addEventListener("keydown", (event) => {
            if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
              if (event.shiftKey) return; // leave Save-As (future) alone
              event.preventDefault();
              frontendUnits.branchesFileTreeSurface.requestSaveFileContent(windowData.id);
            }
          });
          return;
        }

        if (surface === "branches") {
          body.innerHTML = `
            <div class="branch-list-root">
              <div class="branch-toolbar workspace-toolbar is-stacked">
                <div class="branch-toolbar-main workspace-toolbar-main">
                  <div class="branch-heading">Repository branches</div>
                  <div class="branch-filter-group">
                    <button class="branch-filter-button" type="button" data-branch-filter="local">Local</button>
                    <button class="branch-filter-button" type="button" data-branch-filter="remote">Remote</button>
                    <button class="branch-filter-button" type="button" data-branch-filter="all">All</button>
                  </div>
                </div>
                <div class="branch-toolbar-actions workspace-toolbar-actions">
                  <div class="branch-selection-actions">
                    <button class="wizard-button branch-cleanup-trigger" type="button" data-action="open-branch-cleanup">Clean Up</button>
                  </div>
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
                  <button class="text-button board-all-filter" data-action="toggle-board-all" type="button" aria-pressed="false">All</button>
                  <button class="text-button board-for-you-filter" data-action="toggle-board-for-you" type="button" aria-pressed="false">For you</button>
                  <button class="text-button board-workspace-filter" data-action="toggle-board-workspace" type="button" aria-pressed="false">Work</button>
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
              state.audienceFilter =
                state.audienceFilter === "for_you" ? "workspace" : "for_you";
              if (state.audienceFilter === "for_you") {
                state.forYouUnread = 0;
              }
              frontendUnits.boardSurface.renderBoard(windowData.id);
            });
          body
            .querySelector("[data-action='toggle-board-all']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              const state = frontendUnits.boardSurface.ensureBoardState(windowData.id);
              state.audienceFilter = state.audienceFilter === "all" ? "workspace" : "all";
              state.error = "";
              frontendUnits.boardSurface.requestBoard(windowData.id);
              frontendUnits.boardSurface.renderBoard(windowData.id);
            });
          // SPEC-2359 FR-101: toggle the Workspace audience filter. The
          // entry visibility itself is driven by `state.audienceFilter ===
          // "workspace"` plus `state.currentWorkspaceId` via
          // `visibleBoardEntries`; the projection wires up the workspace
          // id separately so unassigned agents see only broadcast.
          body
            .querySelector("[data-action='toggle-board-workspace']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              const state = frontendUnits.boardSurface.ensureBoardState(windowData.id);
              state.audienceFilter =
                state.audienceFilter === "workspace" ? "all" : "workspace";
              state.error = "";
              frontendUnits.boardSurface.requestBoard(windowData.id);
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
                  <span>Process</span>
                  <select class="logs-process-kind-select">
                    <option value="">All</option>
                    <option value="gh">gh</option>
                    <option value="git">git</option>
                    <option value="docker">docker</option>
                    <option value="agent">agent</option>
                    <option value="runner">runner</option>
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
            .querySelector(".logs-process-kind-select")
            .addEventListener("change", (event) => {
              state.processKind = event.target.value;
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

        if (surface === "work") {
          workspaceOverviewSurface.mount(body, windowData, {
            focusWindowLocally,
            sendFocus: (id) => socketTransport.send({ kind: "focus_window", id }),
          });
          return;
        }

        if (surface === "console") {
          // SPEC-2809 — Console window mount: register a controller for
          // this windowId and attach its DOM to the window body. The
          // controller subscribes implicitly via the shared
          // `consoleControllers` registry that the `process_line`
          // dispatcher fans out to.
          const controller = ensureConsoleController(windowData.id);
          while (body.firstChild) {
            body.removeChild(body.firstChild);
          }
          controller.mount(body);
          return;
        }

        if (surface === "index") {
          body.innerHTML = `
            <div class="index-search-root">
              <div class="index-window-header">
                <div class="index-window-title">
                  <div class="knowledge-heading">Index</div>
                  <div class="index-window-subtitle">Search indexed project content</div>
                </div>
                <div class="workspace-toolbar-actions index-window-tabs">
                  <button class="settings-tab active" type="button" role="tab" aria-selected="true" data-index-tab="search">Search</button>
                  <button class="settings-tab" type="button" role="tab" aria-selected="false" data-index-tab="health">Health</button>
                </div>
              </div>
              <section class="index-search-panel" data-index-panel="search">
                <div class="index-search-toolbar">
                  <form class="index-search-box" role="search">
                    <input class="index-search-input" type="search" placeholder="Search by meaning, e.g. workspace lifecycle" aria-label="Search indexed content" />
                    <button class="index-run-button" type="submit">Search</button>
                  </form>
                  <div class="index-filter-bar">
                    <div class="index-match-mode-list" role="group" aria-label="Search mode">
                      <button class="index-match-mode-button active" type="button" aria-pressed="true" data-match-mode="semantic">Semantic</button>
                      <button class="index-match-mode-button" type="button" aria-pressed="false" data-match-mode="all_terms">All terms</button>
                    </div>
                    <div class="index-scope-list" role="group" aria-label="Search scopes"></div>
                    <label class="index-worktree-field">
                      <span>File worktree</span>
                      <select class="index-worktree-select" aria-label="Files and Docs worktree"></select>
                    </label>
                  </div>
                </div>
                <div class="index-search-status"></div>
                <div class="index-search-layout workspace-split">
                  <div class="index-result-list workspace-scroll"></div>
                  <div class="index-result-detail workspace-scroll"></div>
                </div>
              </section>
              <section class="index-health-panel" data-index-panel="health" hidden>
                <div class="index-health-toolbar">
                  <div>
                    <div class="index-health-title">Project index health</div>
                    <div class="index-health-subtitle">Repair indexed sources without leaving this window.</div>
                  </div>
                  <button class="icon-button" data-action="refresh-index-status" aria-label="Refresh index status">↻</button>
                </div>
                <div class="index-health-table workspace-scroll"></div>
              </section>
            </div>
          `;
          body.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            socketTransport.send({ kind: "focus_window", id: windowData.id });
          });
          const state = ensureIndexSearchState(windowData.id);
          const input = body.querySelector(".index-search-input");
          input.value = state.query;
          input.addEventListener("input", () => {
            state.query = input.value;
            if (!state.query.trim()) {
              clearProjectIndexSearchState(state);
              renderProjectIndexSearch(windowData.id);
              return;
            }
            markProjectIndexSearchPending(state);
            renderProjectIndexSearch(windowData.id);
            scheduleProjectIndexSearch(windowData.id);
          });
          body.querySelector(".index-search-box").addEventListener("submit", (event) => {
            event.preventDefault();
            if (state.searchTimer) {
              clearTimeout(state.searchTimer);
              state.searchTimer = 0;
            }
            sendProjectIndexSearch(windowData.id);
          });
          body.querySelector(".index-scope-list").addEventListener("click", (event) => {
            const button = event.target.closest("[data-scope]");
            if (!button) return;
            const scope = button.dataset.scope;
            if (state.selectedScopes.has(scope)) {
              state.selectedScopes.delete(scope);
            } else {
              state.selectedScopes.add(scope);
            }
            if (state.query.trim()) {
              markProjectIndexSearchPending(state);
            }
            renderProjectIndexSearch(windowData.id);
            scheduleProjectIndexSearch(windowData.id);
          });
          body.querySelector(".index-match-mode-list").addEventListener("click", (event) => {
            const button = event.target.closest("[data-match-mode]");
            if (!button) return;
            state.matchMode = button.dataset.matchMode || "semantic";
            if (state.query.trim()) {
              markProjectIndexSearchPending(state);
            }
            renderProjectIndexSearch(windowData.id);
            scheduleProjectIndexSearch(windowData.id);
          });
          body.querySelector(".index-worktree-select").addEventListener("change", (event) => {
            state.selectedWorktreeHash = event.target.value || "";
            if (state.query.trim()) {
              markProjectIndexSearchPending(state);
              renderProjectIndexSearch(windowData.id);
            }
            scheduleProjectIndexSearch(windowData.id);
          });
          for (const tab of body.querySelectorAll("[data-index-tab]")) {
            tab.addEventListener("click", () => {
              state.activeTab = tab.dataset.indexTab || "search";
              if (state.activeTab === "health") {
                requestFullIndexStatusRefresh();
              }
              renderProjectIndexSearch(windowData.id);
            });
          }
          body
            .querySelector("[data-action='refresh-index-status']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              requestFullIndexStatusRefresh();
            });
          body.querySelector(".index-result-list").addEventListener("click", (event) => {
            const row = event.target.closest("[data-result-index]");
            if (!row) return;
            state.selectedResultIndex = Number(row.dataset.resultIndex || 0);
            renderProjectIndexSearch(windowData.id);
          });
          body.querySelector(".index-result-list").addEventListener("dblclick", (event) => {
            const row = event.target.closest("[data-result-index]");
            if (!row) return;
            state.selectedResultIndex = Number(row.dataset.resultIndex || 0);
            const result = selectedIndexSearchItem(state)?.result;
            openIndexResultTarget(result);
          });
          body.querySelector(".index-result-list").addEventListener("keydown", (event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              moveIndexResultSelection(windowData.id, 1);
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              moveIndexResultSelection(windowData.id, -1);
            } else if (event.key === "Enter") {
              event.preventDefault();
              const result = selectedIndexSearchItem(state)?.result;
              openIndexResultTarget(result);
            }
          });
          body.querySelector(".index-result-detail").addEventListener("click", (event) => {
            if (!event.target.closest("[data-action='open-index-result']")) return;
            const result = selectedIndexSearchItem(state)?.result;
            openIndexResultTarget(result);
          });
          renderProjectIndexSearch(windowData.id);
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
          const pendingIndexTarget = pendingIndexOpenTargetsByPreset.get(windowData.preset);
          if (
            pendingIndexTarget
            && pendingIndexTarget.knowledgeKind === knowledgeKind
          ) {
            state.selectedNumber = pendingIndexTarget.number;
            pendingIndexOpenTargetsByPreset.delete(windowData.preset);
          }
          const search = body.querySelector(".knowledge-search");
          search.value = state.query;
          search.addEventListener("input", () => {
            state.query = search.value;
            frontendUnits.knowledgeSettingsSurface.scheduleKnowledgeSearch(
              windowData.id,
              knowledgeKind,
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
          ensureKnowledgeAutoRefresh(windowData.id, knowledgeKind);
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
      // SPEC-1921 2026-05-18 amendment / FR-099: Settings > Agent Backends
      // per-built-in backend profile state. `backends` is keyed by
      // BuiltinAgentId string ("claudeCode" / "codex"). Mirrors
      // customAgentsState shape so dispatch + status messages can share
      // helpers like setSettingsStatus.
      const agentBackendsState = {
        backends: { claudeCode: [], codex: [] },
        loadingAgent: null,
        statusMessage: "",
        statusKind: "",
      };
      // SPEC-1933 US-4: System tab state. `language` is the raw stored value
      // (auto/en/ja); the backend `system_settings` reply seeds it.
      const systemSettingsState = {
        language: "auto",
        codexTrustManagedHooks: true,
        // SPEC-2959/2963: selected Board backend (local/slack/teams).
        boardProvider: "local",
        // SPEC-2963: remote provider sign-in state + last sign-in message.
        boardAuth: { slack: false, teams: false },
        boardAuthMessage: "",
        // SPEC-2963: editable (non-secret) provider config for the settings UI.
        // Secrets are never echoed back; `*HasSecret` flags reflect store state.
        boardConfig: {
          slackClientId: "",
          slackDefaultChannel: "",
          slackHasSecret: false,
          teamsClientId: "",
          teamsTenantId: "",
          teamsDefaultChannel: "",
          oauthRedirectPort: 8765,
        },
        autostartEnabled: false,
        autostartPreviousEnabled: false,
        autostartMechanism: "",
        autostartInstallPath: "",
        autostartLoaded: false,
        autostartPending: false,
        loaded: false,
        statusMessage: "",
        statusKind: "",
      };
      const settingsWindowBodies = new Set();
      let pendingAddFromPreset = null;
      let editingCustomAgentId = null;

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
        // SPEC-1921 2026-05-18 amendment / FR-099: Agent Backends tab is the
        // dedicated surface for Claude Code / Codex Backend Override profiles.
        // Kept distinct from `custom-agents` so External CLI rows and
        // built-in LLM redirection have separate physical UI.
        tabs.appendChild(buildSettingsTab("agent-backends", "Agent Backends", false));
        // SPEC-2970: provider usage display preferences (Claude opt-in).
        tabs.appendChild(buildSettingsTab("usage", "Usage & Limits", false));

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

        const panelBackends = document.createElement("section");
        panelBackends.className = "settings-panel hidden";
        panelBackends.setAttribute("role", "tabpanel");
        panelBackends.dataset.settingsPanel = "agent-backends";
        panelBackends.dataset.role = "settings-scroll";

        const panelUsage = document.createElement("section");
        panelUsage.className = "settings-panel hidden";
        panelUsage.setAttribute("role", "tabpanel");
        panelUsage.dataset.settingsPanel = "usage";
        panelUsage.dataset.role = "settings-scroll";

        bodyEl.appendChild(panelSystem);
        bodyEl.appendChild(panelAgents);
        bodyEl.appendChild(panelBackends);
        bodyEl.appendChild(panelUsage);

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
        renderUsagePanel(panelUsage);
        // Always request fresh system settings on open so the dropdown
        // reflects the on-disk config, even if the user changed it from a
        // different gwt instance.
        send({ kind: "get_system_settings" });
        // SPEC-2963: also fetch remote Board provider sign-in state.
        send({ kind: "get_board_auth_status" });
        send({ kind: "get_autostart_status" });

        renderSettingsAgentList();
        if (!customAgentsState.loading && customAgentsState.agents.length === 0) {
          customAgentsState.loading = true;
          send({ kind: "list_custom_agents" });
        }

        // SPEC-1921 2026-05-18 amendment / FR-099: hydrate the Agent
        // Backends panel for both built-in agents that support Backend
        // Override (Claude Code, Codex). The backend returns the redacted
        // list (api_key replaced with `***REDACTED***`).
        renderAgentBackendsPanel(panelBackends);
        for (const agent of ["claudeCode", "codex"]) {
          send({ kind: "list_agent_backends", agent });
        }

        // Honour any pending settings:open dispatch (e.g. from the badge
        // click) by switching to the requested tab once the panel is mounted.
        if (pendingSettingsTabTarget) {
          switchSettingsTab(body, pendingSettingsTabTarget);
          pendingSettingsTabTarget = null;
        }
      }

      let pendingSettingsTabTarget = null;

      function renderIndexPanel(panel) {
        const activeProjectRoot = activeProjectTab()?.project_root || "";
        const status =
          (activeProjectRoot && indexStatusByProjectRoot.get(activeProjectRoot)) || null;
        renderIndexSettingsPanel({
          panel,
          status,
          projectRoot: activeProjectRoot,
          send,
        });
      }

      // SPEC-1921 2026-05-18 amendment / FR-099: render the `Agent Backends`
      // Settings tab body. Two `[data-agent]` sections (`claudeCode` /
      // `codex`) host the per-built-in backend lists; each saved backend
      // renders as a row with the redacted profile shape. The Add /
      // Edit / Delete affordances will land alongside the protocol
      // dispatch when the inline forms move out of the legacy
      // `Custom Agents` tab (T308 follow-up). Today the panel exposes a
      // read-only mirror that confirms FR-101 silent migration produced
      // the expected `[builtinAgents.claudeCode.backends.*]` rows.
      function renderAgentBackendsPanel(panel) {
        while (panel.firstChild) panel.removeChild(panel.firstChild);

        for (const agent of ["claudeCode", "codex"]) {
          const section = createDiv("settings-section");
          section.dataset.agent = agent;

          const heading = document.createElement("h3");
          heading.className = "settings-section-heading";
          heading.textContent =
            agent === "claudeCode" ? "Claude Code" : "Codex";
          section.appendChild(heading);

          const list = createDiv("agent-backends-list");
          list.dataset.role = "agent-backends-list";
          list.dataset.agent = agent;
          renderAgentBackendsList(list, agent);
          section.appendChild(list);

          panel.appendChild(section);
        }
      }

      function renderAgentBackendsList(container, agent) {
        while (container.firstChild) container.removeChild(container.firstChild);
        const profiles = agentBackendsState.backends[agent] || [];
        if (profiles.length === 0) {
          const empty = document.createElement("p");
          empty.className = "settings-help";
          empty.textContent =
            agent === "claudeCode"
              ? "No Claude Code backend profiles saved. Default Anthropic upstream is used."
              : "No Codex backend profiles saved. Default OpenAI upstream is used.";
          container.appendChild(empty);
          return;
        }
        for (const profile of profiles) {
          const row = createDiv("agent-backend-row");
          row.dataset.backendId = profile.id;
          const title = document.createElement("strong");
          title.textContent =
            profile.display_name || profile.displayName || profile.id;
          row.appendChild(title);
          const detail = document.createElement("span");
          detail.className = "settings-help";
          const baseUrl = profile.base_url || profile.baseUrl || "";
          const model = profile.model || "";
          detail.textContent = ` · ${baseUrl} · ${model}`;
          row.appendChild(detail);
          container.appendChild(row);
        }
      }

      function renderAgentBackendsPanelInAllSettingsWindows() {
        for (const settingsBody of Array.from(settingsWindowBodies)) {
          if (!settingsBody.isConnected) {
            settingsWindowBodies.delete(settingsBody);
            continue;
          }
          const panel = settingsBody.querySelector(
            "[data-settings-panel='agent-backends']",
          );
          if (panel) renderAgentBackendsPanel(panel);
        }
      }

      function renderIndexPanelInAllSettingsWindows() {
        for (const settingsBody of Array.from(settingsWindowBodies)) {
          if (!settingsBody.isConnected) {
            settingsWindowBodies.delete(settingsBody);
            continue;
          }
          const panel = settingsBody.querySelector(
            "[data-settings-panel='index']",
          );
          if (panel) renderIndexPanel(panel);
        }
      }

      function requestFullIndexStatusRefresh() {
        const activeProjectRoot = activeProjectTab()?.project_root || "";
        if (!activeProjectRoot) return;
        send({ kind: "refresh_index_status", project_root: activeProjectRoot });
      }

      document.addEventListener("settings:open", (event) => {
        const target = event?.detail?.target || "system";
        if (target === "index") {
          focusOrSpawnPreset("index");
          return;
        }
        const existingBody = Array.from(settingsWindowBodies).find(
          (settingsBody) => settingsBody.isConnected,
        );
        if (existingBody) {
          switchSettingsTab(existingBody, target);
          return;
        }
        pendingSettingsTabTarget = target;
        focusOrSpawnPreset("settings");
      });

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
          "Used for narrative outputs (Work summaries and Board post bodies). " +
          "Settings UI text and gwtd subcommands stay English.";
        section.appendChild(help);

        const trustSection = createDiv("settings-section");
        const trustLabel = document.createElement("label");
        trustLabel.className = "settings-checkbox-label";
        trustLabel.setAttribute("for", "settings-system-codex-hooks");

        const trustCheckbox = document.createElement("input");
        trustCheckbox.type = "checkbox";
        trustCheckbox.className = "settings-checkbox";
        trustCheckbox.id = "settings-system-codex-hooks";
        trustCheckbox.checked = systemSettingsState.codexTrustManagedHooks !== false;
        trustCheckbox.addEventListener("change", (e) => {
          const next = e.target.checked === true;
          systemSettingsState.codexTrustManagedHooks = next;
          systemSettingsState.statusMessage = "Saving…";
          systemSettingsState.statusKind = "info";
          renderSystemPanelStatus(panel);
          send({
            kind: "update_system_settings",
            language: systemSettingsState.language || "auto",
            codex_trust_managed_hooks: next,
          });
        });

        const trustText = document.createElement("span");
        trustText.textContent = "Trust gwt-managed Codex hooks";
        trustLabel.appendChild(trustCheckbox);
        trustLabel.appendChild(trustText);
        trustSection.appendChild(trustLabel);

        const trustHelp = document.createElement("p");
        trustHelp.className = "settings-help";
        trustHelp.textContent =
          "Enabled by default. Registers only generated gwt hook commands in Codex hook trust state.";
        trustSection.appendChild(trustHelp);

        // SPEC-2959/2963: Board provider selector. `local` keeps the Board
        // offline; `slack` / `teams` are network-backed and selectable. Picking
        // a remote provider reveals its config form (client id / channel /
        // secret) and a sign-in affordance below.
        const boardSection = createDiv("settings-section");
        const boardLabel = document.createElement("label");
        boardLabel.className = "settings-label";
        boardLabel.setAttribute("for", "settings-system-board-provider");
        boardLabel.textContent = "Board provider";
        boardSection.appendChild(boardLabel);

        const boardSelect = document.createElement("select");
        boardSelect.className = "settings-select";
        boardSelect.id = "settings-system-board-provider";
        for (const opt of [
          { value: "local", text: "Local (offline)" },
          { value: "slack", text: "Slack" },
          { value: "teams", text: "Teams" },
        ]) {
          const option = document.createElement("option");
          option.value = opt.value;
          option.textContent = opt.text;
          boardSelect.appendChild(option);
        }
        boardSelect.value = systemSettingsState.boardProvider || "local";
        boardSelect.addEventListener("change", (e) => {
          const next = e.target.value;
          systemSettingsState.boardProvider = next;
          systemSettingsState.statusMessage = "Saving…";
          systemSettingsState.statusKind = "info";
          renderSystemPanelStatus(panel);
          send({
            kind: "update_system_settings",
            language: systemSettingsState.language || "auto",
            board_provider: next,
          });
          renderSystemPanelInAllSettingsWindows();
        });
        boardSection.appendChild(boardSelect);

        const boardHelp = document.createElement("p");
        boardHelp.className = "settings-help";
        boardHelp.textContent =
          "Where the coordination Board is stored. Local keeps the Board offline and " +
          "on this machine. Slack / Teams require sign-in and are network-backed.";
        boardSection.appendChild(boardHelp);

        // SPEC-2963 FR-011/FR-012: sign-in affordance + auth status for the
        // selected remote provider. Local needs no sign-in.
        const selectedProvider = systemSettingsState.boardProvider || "local";
        if (selectedProvider === "slack" || selectedProvider === "teams") {
          // SPEC-2963 FR-006: provider config form. Non-secret fields persist to
          // config.toml; the client secret is routed to the secure credential
          // store and never echoed back (placeholder shows "configured").
          const cfg = systemSettingsState.boardConfig || {};
          const configForm = createDiv("settings-section board-config-form");
          configForm.dataset.provider = selectedProvider;

          const makeField = (id, labelText, value, opts = {}) => {
            const wrap = createDiv("settings-field");
            const fieldLabel = document.createElement("label");
            fieldLabel.className = "settings-label";
            fieldLabel.setAttribute("for", id);
            fieldLabel.textContent = labelText;
            wrap.appendChild(fieldLabel);
            const input = document.createElement("input");
            input.className = "settings-input";
            input.id = id;
            input.type = opts.password ? "password" : "text";
            input.value = value || "";
            if (opts.placeholder) input.placeholder = opts.placeholder;
            if (opts.autocomplete) input.autocomplete = opts.autocomplete;
            wrap.appendChild(input);
            configForm.appendChild(wrap);
            return input;
          };

          let clientIdInput;
          let defaultChannelInput;
          let tenantIdInput;
          let secretInput;
          if (selectedProvider === "slack") {
            clientIdInput = makeField(
              "settings-board-slack-client-id",
              "Client ID",
              cfg.slackClientId,
              { placeholder: "e.g. 1234567890.1234567890" },
            );
            defaultChannelInput = makeField(
              "settings-board-slack-channel",
              "Default channel ID",
              cfg.slackDefaultChannel,
              { placeholder: "e.g. C0123456789" },
            );
            secretInput = makeField(
              "settings-board-slack-secret",
              "Client secret",
              "",
              {
                password: true,
                autocomplete: "new-password",
                placeholder: cfg.slackHasSecret
                  ? "configured — leave blank to keep"
                  : "required for Slack sign-in",
              },
            );
            // The secret is stored securely and never echoed back, so the
            // field intentionally clears after Save. Show an explicit saved
            // state so it is obvious the secret persisted (id used by tests).
            const secretState = createNode(
              "p",
              "settings-help board-secret-state",
              cfg.slackHasSecret
                ? "✓ A client secret is saved (the field stays blank for security)."
                : "No client secret saved yet.",
            );
            secretState.dataset.hasSecret = cfg.slackHasSecret ? "true" : "false";
            configForm.appendChild(secretState);
          } else {
            clientIdInput = makeField(
              "settings-board-teams-client-id",
              "Application (client) ID",
              cfg.teamsClientId,
              { placeholder: "Entra app id" },
            );
            tenantIdInput = makeField(
              "settings-board-teams-tenant-id",
              "Tenant ID",
              cfg.teamsTenantId,
              { placeholder: "tenant id / common / organizations" },
            );
            defaultChannelInput = makeField(
              "settings-board-teams-channel",
              "Default channel",
              cfg.teamsDefaultChannel,
              { placeholder: "team_id/channel_id" },
            );
          }

          const saveBtn = createNode(
            "button",
            "wizard-button",
            "Save configuration",
          );
          saveBtn.type = "button";
          saveBtn.addEventListener("click", () => {
            const payload = {
              kind: "update_board_provider_config",
              provider: selectedProvider,
              client_id: clientIdInput ? clientIdInput.value.trim() : "",
              default_channel: defaultChannelInput
                ? defaultChannelInput.value.trim()
                : "",
            };
            if (selectedProvider === "teams") {
              payload.tenant_id = tenantIdInput ? tenantIdInput.value.trim() : "";
            }
            if (selectedProvider === "slack" && secretInput) {
              // Only send the secret when the user typed one, so an empty box
              // does not clear an already-configured secret.
              if (secretInput.value.length > 0) {
                payload.client_secret = secretInput.value;
              }
            }
            send(payload);
          });
          configForm.appendChild(saveBtn);
          boardSection.appendChild(configForm);

          // SPEC-2963 FR-005: fixed OAuth callback port. The redirect_uri must
          // exactly match a URL registered in the provider app; gwt binds this
          // loopback port so sign-in works regardless of the (ephemeral) GUI
          // server port. Editable so a busy 8765 can be changed.
          const oauthPort = Number(cfg.oauthRedirectPort) || 8765;
          const oauthForm = createDiv("settings-section board-oauth-port-form");
          const portField = createDiv("settings-field");
          const portLabel = document.createElement("label");
          portLabel.className = "settings-label";
          portLabel.setAttribute("for", "settings-board-oauth-port");
          portLabel.textContent = "OAuth callback port";
          portField.appendChild(portLabel);
          const portInput = document.createElement("input");
          portInput.className = "settings-input";
          portInput.id = "settings-board-oauth-port";
          portInput.type = "number";
          portInput.min = "1";
          portInput.max = "65535";
          portInput.value = String(oauthPort);
          portField.appendChild(portInput);
          oauthForm.appendChild(portField);

          const redirectHint = createNode(
            "p",
            "settings-help board-oauth-redirect-hint",
            "",
          );
          const renderRedirectHint = () => {
            const p = Number(portInput.value) || oauthPort;
            redirectHint.textContent =
              "Register this exact Redirect URL in the Slack/Teams app: " +
              `http://127.0.0.1:${p}/oauth/callback`;
          };
          renderRedirectHint();
          portInput.addEventListener("input", renderRedirectHint);
          oauthForm.appendChild(redirectHint);

          const savePortBtn = createNode("button", "wizard-button", "Save port");
          savePortBtn.type = "button";
          savePortBtn.addEventListener("click", () => {
            const next = Number(portInput.value);
            send({
              kind: "update_board_oauth_port",
              port: Number.isFinite(next) && next > 0 ? Math.floor(next) : 0,
            });
          });
          oauthForm.appendChild(savePortBtn);
          boardSection.appendChild(oauthForm);

          const auth = systemSettingsState.boardAuth || { slack: false, teams: false };
          const signedIn = auth[selectedProvider] === true;
          const authRow = createDiv("settings-section board-auth-row");
          const statusText = createNode(
            "span",
            "board-auth-status",
            signedIn
              ? `Signed in to ${selectedProvider}`
              : `Not signed in to ${selectedProvider}`,
          );
          statusText.dataset.signedIn = signedIn ? "true" : "false";
          authRow.appendChild(statusText);

          const signInBtn = createNode(
            "button",
            "wizard-button",
            signedIn ? "Re-sign in" : "Sign in",
          );
          signInBtn.type = "button";
          signInBtn.addEventListener("click", () => {
            send({ kind: "board_provider_sign_in", provider: selectedProvider });
          });
          authRow.appendChild(signInBtn);

          if (signedIn) {
            const signOutBtn = createNode("button", "text-button", "Sign out");
            signOutBtn.type = "button";
            signOutBtn.addEventListener("click", () => {
              send({ kind: "board_provider_sign_out", provider: selectedProvider });
            });
            authRow.appendChild(signOutBtn);
          }

          const refreshBtn = createNode("button", "text-button", "Refresh");
          refreshBtn.type = "button";
          refreshBtn.addEventListener("click", () => {
            send({ kind: "get_board_auth_status" });
          });
          authRow.appendChild(refreshBtn);
          boardSection.appendChild(authRow);

          if (systemSettingsState.boardAuthMessage) {
            boardSection.appendChild(
              createNode("p", "settings-help", systemSettingsState.boardAuthMessage),
            );
          }
        }

        const autostartSection = createDiv("settings-section");
        const autostartLabel = document.createElement("label");
        autostartLabel.className = "settings-checkbox-label";
        autostartLabel.setAttribute("for", "settings-system-autostart");

        const autostartCheckbox = document.createElement("input");
        autostartCheckbox.type = "checkbox";
        autostartCheckbox.className = "settings-checkbox";
        autostartCheckbox.id = "settings-system-autostart";
        autostartCheckbox.checked = systemSettingsState.autostartEnabled === true;
        autostartCheckbox.disabled = systemSettingsState.autostartPending === true;
        autostartCheckbox.addEventListener("change", (e) => {
          const next = e.target.checked === true;
          systemSettingsState.autostartPreviousEnabled =
            systemSettingsState.autostartEnabled === true;
          systemSettingsState.autostartEnabled = next;
          systemSettingsState.autostartPending = true;
          systemSettingsState.statusMessage = "Saving…";
          systemSettingsState.statusKind = "info";
          renderSystemPanelInAllSettingsWindows();
          send({ kind: "update_autostart", enabled: next });
        });

        const autostartText = document.createElement("span");
        autostartText.textContent = "Launch GWT at login";
        autostartLabel.appendChild(autostartCheckbox);
        autostartLabel.appendChild(autostartText);
        autostartSection.appendChild(autostartLabel);

        const autostartHelp = document.createElement("p");
        autostartHelp.className = "settings-help";
        autostartHelp.textContent =
          "Starts GWT in the menu bar when you log in. The browser does not open automatically.";
        autostartSection.appendChild(autostartHelp);

        if (systemSettingsState.autostartLoaded) {
          const autostartDetail = document.createElement("p");
          autostartDetail.className = "settings-help";
          const mechanism = systemSettingsState.autostartMechanism || "Unknown";
          const installPath = systemSettingsState.autostartInstallPath || "";
          autostartDetail.textContent = installPath
            ? `Autostart: ${mechanism} · ${installPath}`
            : `Autostart: ${mechanism}`;
          autostartSection.appendChild(autostartDetail);
        }

        const status = document.createElement("p");
        status.className = "settings-status";
        status.dataset.role = "system-settings-status";
        autostartSection.appendChild(status);

        panel.appendChild(section);
        panel.appendChild(trustSection);
        panel.appendChild(boardSection);
        panel.appendChild(autostartSection);
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
          // SPEC-1921 Phase 63H / T326: the legacy
          // `+ Add Claude Code (OpenAI-compat backend)` button now points
          // users at the new `Agent Backends` tab. The underlying
          // `add_custom_agent_from_preset` dispatch is preserved for
          // existing callers (Phase 52 contract), but the entry point
          // visible in Custom Agents redirects to the proper surface so
          // External CLI rows and Backend Override profiles never get
          // conflated again.
          addBtn.textContent = "＋ Add Claude Code backend (moved to Agent Backends)";
          addBtn.addEventListener("click", (e) => {
            e.stopPropagation();
            // Switch the Settings window to the Agent Backends tab.
            const body = scroll.closest(".settings-body")?.parentElement;
            if (body) switchSettingsTab(body, "agent-backends");
            setSettingsStatus(
              "Backend Override moved to Agent Backends. Add your Claude Code / Codex backend there.",
              "success",
            );
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
            const editBtn = document.createElement("button");
            editBtn.className = "icon-button";
            editBtn.setAttribute("aria-label", "Edit agent environment");
            editBtn.title = "Edit environment";
            editBtn.textContent = "✎";
            editBtn.addEventListener("click", (e) => {
              e.stopPropagation();
              editingCustomAgentId =
                editingCustomAgentId === agent.id ? null : agent.id;
              renderSettingsAgentList();
            });
            row.appendChild(editBtn);
            row.appendChild(delBtn);
            section.appendChild(row);
            if (editingCustomAgentId === agent.id) {
              section.appendChild(
                renderCustomAgentEnvEditor({
                  document,
                  agent,
                  onSave: (updatedAgent) => {
                    editingCustomAgentId = null;
                    setSettingsStatus("Saving custom agent…", "info");
                    send({ kind: "update_custom_agent", agent: updatedAgent });
                  },
                  onCancel: () => {
                    editingCustomAgentId = null;
                    renderSettingsAgentList();
                  },
                }),
              );
            }
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
            trackWindowTabDragPoint(event);
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
            trackWindowTabDragPoint(event);
            windowTabDragState.docked = true;
            send({
              kind: "dock_window_tab",
              id: windowTabDragState.id,
              target_id: windowData.id,
            });
          });

          resizeHandle.addEventListener("pointerdown", (event) => {
            focusWindowRemotely(windowData.id);
            // SPEC-2014 Phase C1: Windows WebView2 occasionally fails to
            // deliver pointerup / pointercancel / lostpointercapture, leaving
            // the previous resizeState alive when the next gesture starts.
            // Force-clear the leaked state on every new pointerdown so the
            // user never has to restart the app to escape a stuck resize.
            forceResetResizeState("new resize started before previous one finished");
            const currentWindow = workspaceWindowById(windowData.id);
            const baseGeometryRevision = localGeometryBaseRevision(
              geometrySyncState,
              windowData.id,
              currentWindow || windowData,
            );
            beginLocalGeometryEdit(
              geometrySyncState,
              windowData.id,
              baseGeometryRevision,
            );
            resizeState = {
              id: windowData.id,
              pointerId: event.pointerId,
              startX: event.clientX,
              startY: event.clientY,
              latestClientX: event.clientX,
              latestClientY: event.clientY,
              width: parseNumber(element.style.width),
              height: parseNumber(element.style.height),
              baseGeometryRevision,
              fitFrame: null,
              applyFrame: null,
              startedAt: performance.now(),
              stalenessTimer: scheduleResizeStalenessGuard(event.pointerId),
            };
            tracePointer(UI_TRACE_EVENT.pointerResizeStart, event, {
              gesture: "resize",
              accepted: true,
              window_id: windowData.id,
              base_geometry_revision: baseGeometryRevision,
            });
            // SPEC-2356 Phase 9 (T-136): suppress hover-reveal peek strip
            // hits while resize is active so pointer movements that cross
            // the screen edge do not steal focus mid-resize.
            document.documentElement.dataset.opResizeActive = "true";
            try {
              resizeHandle.setPointerCapture(event.pointerId);
              tracePointer(UI_TRACE_EVENT.pointerCaptureSet, event, {
                gesture: "resize",
                accepted: true,
                window_id: windowData.id,
              });
            } catch (error) {
              // SPEC-2014 Phase C1: setPointerCapture is best-effort; on
              // Windows WebView2 it can throw when the pointer has already
              // been released by the OS between the dispatch and the
              // callback. We continue without capture so that the
              // window-bound pointermove / pointerup listeners still
              // drive the resize.
              console.warn(
                "[resize] setPointerCapture failed, falling back to window-bound pointer events",
                error,
              );
              tracePointer(UI_TRACE_EVENT.pointerCaptureFailed, event, {
                gesture: "resize",
                accepted: false,
                reason: "set_pointer_capture_failed",
                window_id: windowData.id,
                error_name: error && error.name,
              });
            }
          });
          resizeHandle.addEventListener("lostpointercapture", (event) => {
            tracePointer(UI_TRACE_EVENT.pointerLostCapture, event, {
              gesture: "resize",
              accepted: true,
              window_id: windowData.id,
            });
            finishWindowResize(event.pointerId, event);
          });
        }

        if (!element.dataset.preset || element.dataset.preset !== windowData.preset) {
          element.dataset.preset = windowData.preset;
          mountWindowBody(windowData, element);
        }

        const nextWindowElementKey = windowElementRenderKey(windowData);
        if (renderedWindowElementKeys.get(windowData.id) === nextWindowElementKey) {
          return;
        }

        element.querySelector(".title-text").textContent = windowDisplayTitle(windowData);
        const titleText = element.querySelector(".title-text");
        titleText.title = windowTitleTooltip(windowData);
        setWindowRoleBadge(element.querySelector(".window-role-badge"), windowData);
        renderWindowTabs(windowData, element);
        if (windowData.agent_color) {
          element.dataset.agentColor = windowData.agent_color;
        } else {
          delete element.dataset.agentColor;
        }
        const wasMinimized = element.classList.contains("minimized");
        const previousWidth = parseFloat(element.style.width || "0");
        const previousHeight = parseFloat(element.style.height || "0");
        const applyWorkspaceGeometry = shouldApplyWorkspaceGeometry(geometrySyncState, {
          id: windowData.id,
          geometryRevision: workspaceGeometryRevision(windowData),
        });
        // SPEC-2008: a maximized window fills THIS client's viewport locally.
        // Each client computes its own fill and never renders/persists the
        // shared geometry while maximized, so two clients with different
        // viewport sizes cannot ping-pong the shared maximized geometry (the
        // flicker bug). See syncMaximizedWindowsToViewport — it re-fills locally
        // and never broadcasts a maximize_window correction.
        const maximizedFill =
          windowData.maximized && !windowData.minimized
            ? maximizedGeometry(visibleBounds(), viewport.zoom)
            : null;
        const targetGeometry = maximizedFill || windowData.geometry;
        const dimensionsChanged =
          (applyWorkspaceGeometry || Boolean(maximizedFill)) &&
          (previousWidth !== targetGeometry.width ||
            previousHeight !== targetGeometry.height);
        const shouldPersistTerminalGeometry =
          (applyWorkspaceGeometry || Boolean(maximizedFill)) &&
          ((wasMinimized && !windowData.minimized) || dimensionsChanged);
        element.classList.toggle("minimized", Boolean(windowData.minimized));
        element.classList.toggle("maximized", Boolean(windowData.maximized));
        element.classList.toggle("tabbed", windowTabsFor(windowData).length > 1);
        if (maximizedFill) {
          element.style.left = `${maximizedFill.x}px`;
          element.style.top = `${maximizedFill.y}px`;
          element.style.width = `${maximizedFill.width}px`;
          element.style.height = `${maximizedFill.height}px`;
        } else if (applyWorkspaceGeometry) {
          element.style.left = `${windowData.geometry.x}px`;
          element.style.top = `${windowData.geometry.y}px`;
          element.style.width = `${windowData.geometry.width}px`;
          element.style.height = `${windowData.minimized ? 38 : windowData.geometry.height}px`;
        }
        element.style.zIndex = String(windowData.z_index);
        element.querySelector(".resize-handle").hidden =
          Boolean(windowData.minimized) || Boolean(windowData.maximized);
        applyStatus(windowData.id, windowData.status, detailMap.get(windowData.id));
        renderedWindowElementKeys.set(windowData.id, nextWindowElementKey);
        if (
          (applyWorkspaceGeometry || Boolean(maximizedFill)) &&
          presetSurface(windowData.preset) === "terminal" &&
          !windowData.minimized
        ) {
          scheduleTerminalFit(windowData.id, shouldPersistTerminalGeometry);
        }
      }

      function renderWorkspace(workspace) {
        return traceMeasure(
          UI_TRACE_EVENT.renderWorkspace,
          { windows: Array.isArray(workspace?.windows) ? workspace.windows.length : 0 },
          () => {
            const nextViewport = viewportSyncState.applyServerViewport(workspace.viewport, {
              scopeKey: activeViewportScopeKey(),
            });
            const viewportChanged = !sameViewportValues(viewport, nextViewport);
            viewport = nextViewport;
            if (!viewportDomApplied || viewportChanged) {
              applyViewport();
            }

            const nextWorkspaceWindowsKey = workspaceWindowsRenderKey(workspace);
            if (renderedWorkspaceWindowsKey === nextWorkspaceWindowsKey) {
              if (workspaceHasVisibleMaximizedWindow(workspace)) {
                scheduleMaximizedWindowsToViewportSync();
              }
              return;
            }
            renderedWorkspaceWindowsKey = nextWorkspaceWindowsKey;

            const activeWindowIdSet = workspaceWindowIdSet(workspace);
            const visibility = classifyProjectWindowVisibility({
              activeWindowIdSet,
              allProjectWindowIdSet: allProjectWindowIdSet(),
              mountedWindowIds: windowMap.keys(),
            });
            for (const windowId of visibility.hidden) {
              const element = windowMap.get(windowId);
              applyVisibilityTransition({
                element,
                shouldHide: true,
                hasTerminal: terminalMap.has(windowId),
                onReveal: () => {
                  terminalOutputBatcher.schedulePending(windowId);
                  rearmPendingTerminalViewportRefresh(windowId);
                  scheduleTerminalFocusActivation(windowId);
                },
              });
            }
            for (const windowId of visibility.removed) {
              const element = windowMap.get(windowId);
              if (!element) continue;
              const runtime = terminalMap.get(windowId);
              if (runtime && runtime.activationFrame !== null) {
                cancelAnimationFrame(runtime.activationFrame);
              }
              terminalViewportRefreshScheduler?.clear(windowId);
              runtime?.cleanup?.();
              runtime?.terminal.dispose();
              terminalMap.delete(windowId);
              decoderMap.delete(windowId);
              detailMap.delete(windowId);
              windowRuntimeStateMap.delete(windowId);
              renderedWindowElementKeys.delete(windowId);
              renderedRuntimeStatusKeys.delete(windowId);
              pendingOutputMap.delete(windowId);
              pendingSnapshotMap.delete(windowId);
              terminalOutputBatcher.clear(windowId);
              const profileState = profileStateMap.get(windowId);
              if (profileState) {
                clearProfileSaveTimer(profileState);
              }
              fileTreeStateMap.delete(windowId);
              branchListStateMap.delete(windowId);
              profileStateMap.delete(windowId);
              boardStateMap.delete(windowId);
              logStateMap.delete(windowId);
              indexSearchStateMap.delete(windowId);
              clearKnowledgeBridgeState(windowId);
              workspaceOverviewSurface.deleteState(windowId);
              if (branchCleanupWindowId === windowId) {
                branchCleanupWindowId = null;
                renderBranchCleanupModal();
              }
              clearLocalGeometryEdit(geometrySyncState, windowId);
              element.remove();
              windowMap.delete(windowId);
            }

            // SPEC-2008 Phase 24 / T-188: detect hidden -> visible transitions
            // for tab-grouped terminal windows so the newly visible terminal
            // gets fit + viewport refresh + focus on the same animation frame
            // cycle. Without this, scrollback wheel input requires a manual
            // OS-level resize before xterm picks up the new measurement. The
            // transition logic lives in `terminal-viewport-reflow.js` so a
            // behavior test (linkedom + element stub) can exercise the
            // hidden-to-visible activation path directly.
            for (const windowData of workspace.windows) {
              ensureWindow(windowData);
              const element = windowMap.get(windowData.id);
              if (!element) continue;
              applyVisibilityTransition({
                element,
                shouldHide: !visibleWindowData(windowData),
                hasTerminal: terminalMap.has(windowData.id),
                onReveal: () => {
                  terminalOutputBatcher.schedulePending(windowData.id);
                  rearmPendingTerminalViewportRefresh(windowData.id);
                  scheduleTerminalFocusActivation(windowData.id);
                },
              });
            }

            scheduleMaximizedWindowsToViewportSync();

            const topmostId = topmostWindowId(workspace);
            if (topmostId && activeWindowIdSet.has(topmostId)) {
              focusWindowLocally(topmostId);
              scheduleTerminalFocusActivation(topmostId, {
                shouldPersistGeometry: false,
              });
            } else {
              focusedId = null;
            }
          },
        );
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
        updateBranchCleanupProgress,
        // SPEC-2009 amendment: Worktree picker + file content viewer.
        openWorktreePicker,
        closeWorktreePicker,
        renderWorktreePicker,
        renderFileTreeViewer,
        requestFileContent,
        // SPEC-2009 amendment Phase 2: edit affordance.
        requestSaveFileContent,
        renderDiscardModal,
        renderConflictModal,
        queueNavigationGuardedByDirty,
        beginViewerForFile,
        applyAfterSaveContinuation,
      });

      const profileSurface = Object.freeze({
        ensureProfileState,
        requestProfile,
        renderProfile,
        createProfile,
        setActiveProfile,
        flushProfileSave,
        deleteProfile,
        updateProfileStatus,
        hasEditableFocus: profileHasEditableFocus,
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
        knowledgeDetailRequestMatches,
        renderKnowledgeBridge,
        renderSettingsWindow,
        renderSettingsAgentList,
        renderAgentBackendsPanel,
        renderAgentBackendsPanelInAllSettingsWindows,
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
        if (shouldDropLiveEventForTestMode(event)) {
          return;
        }
        switch (event.kind) {
          case "workspace_state": {
            projectError = "";
            frontendUnits.projectWorkspaceShell.renderAppState(event.workspace);
            sendStartupAutoResumeReady();
            break;
          }
          case "workspace_projection_prune_result": {
            // SPEC-2359 US-41 Phase 8b: minimal feedback for projection prune.
            // A richer Drawer surface lands in a follow-up; for now an alert
            // keeps the loop closed so users see the count summary.
            const modeLabel =
              event.mode === "dry_run" ? "dry-run" : "applied";
            window.alert(
              `Work Projection Prune (${modeLabel})\n` +
                `  archived: ${event.archived}\n` +
                `  deleted: ${event.deleted}\n` +
                `  skipped: ${event.skipped}`,
            );
            break;
          }
          case "workspace_projection_prune_error":
            window.alert(
              `Work Projection Prune error: ${event.message}`,
            );
            break;
          case "ui_trace_saved":
            window.alert(
              `UI trace saved\n${event.path}\nentries: ${event.entries}`,
            );
            break;
          case "ui_trace_error":
            window.alert(`UI trace error: ${event.message}`);
            break;
          case "release_notes_payload":
            releaseNotesWindow.handlePayload(event);
            break;
          case "release_notes_error":
            releaseNotesWindow.handleError(event.message);
            break;
          case "active_work_projection":
            activeWorkProjection = event.projection || null;
            cacheActiveWorkProjectionWorkspaceIds(activeWorkProjection);
            syncCurrentProjectWorkspaceIds(
              deriveCurrentProjectWorkspaceIds(activeWorkspace() || {}),
            );
            refreshBoardCurrentWorkspaceId();
            // SPEC-2359 Phase W-12 Slice 3 (FR-351): the sidebar Active Works
            // overview is removed; the Work surface lives in the Workspace
            // Overview (Kanban). Keep the projection global + telemetry update
            // so the Kanban surface and Status Strip stay in sync.
            workspaceOverviewSurface.renderWindows();
            recomputeOperatorTelemetry();
            break;
          case "window_list":
            windowListEntries = event.windows || [];
            frontendUnits.projectWorkspaceShell.renderWindowList();
            break;
          case "provider_usage":
            applyProviderUsageUi({
              accounts: event.accounts || [],
              sessions: event.sessions || [],
              consumption: event.consumption || [],
            });
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
          case "attachment_progress":
            handleAttachmentProgress(event);
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
          case "project_index_search_results":
            handleProjectIndexSearchResults(event);
            break;
          case "project_index_search_error":
            handleProjectIndexSearchError(event);
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
          case "file_tree_worktrees": {
            const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
              event.id,
            );
            state.picker.entries = Array.isArray(event.entries) ? event.entries : [];
            state.picker.loading = false;
            state.picker.error = "";
            frontendUnits.branchesFileTreeSurface.renderWorktreePicker(event.id);
            break;
          }
          case "file_tree_worktree_selected": {
            const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
              event.id,
            );
            state.selectedWorktreeId = event.worktree_id || "";
            // After selection, refresh tree contents.
            state.loaded.clear();
            state.expanded.clear();
            state.loading.clear();
            state.error = "";
            frontendUnits.branchesFileTreeSurface.requestFileTree(event.id, "");
            frontendUnits.branchesFileTreeSurface.renderFileTree(event.id);
            break;
          }
          case "file_tree_worktree_error": {
            const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
              event.id,
            );
            state.picker.loading = false;
            state.picker.error = event.message || "Unable to enumerate worktrees";
            frontendUnits.branchesFileTreeSurface.renderWorktreePicker(event.id);
            break;
          }
          case "file_content_text": {
            const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
              event.id,
            );
            const text = event.text || "";
            const newline = (event.newline || "lf").toString();
            state.viewer = {
              ...state.viewer,
              path: event.path,
              mode: "text",
              text,
              encoding: (event.encoding || "").toString().toUpperCase(),
              totalSize: event.total_size || 0,
              hexOffset: 0,
              hexBytes: "",
              error: { kind: "", message: "", size: null, limit: null },
              dirty: false,
              originalText: text,
              originalBytes: null,
              originalEncoding: (event.encoding || "utf-8").toString(),
              originalNewline: newline,
              originalHasBom: Boolean(event.has_bom),
              originalMtime: Number(event.mtime || 0),
              originalSize: Number(event.total_size || 0),
              readOnly: Boolean(event.read_only),
              savedAt: 0,
              saveInFlight: false,
              undoStack: [],
              redoStack: [],
            };
            frontendUnits.branchesFileTreeSurface.renderFileTreeViewer(event.id);
            break;
          }
          case "file_content_hex": {
            const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
              event.id,
            );
            const bytes = decodeBase64ToBytes(event.bytes_b64 || "");
            state.viewer = {
              ...state.viewer,
              path: event.path,
              mode: "hex",
              text: "",
              encoding: "",
              totalSize: event.total_size || 0,
              hexOffset: event.offset || 0,
              hexBytes: event.bytes_b64 || "",
              error: { kind: "", message: "", size: null, limit: null },
              dirty: false,
              originalText: "",
              originalBytes: bytes,
              originalEncoding: "",
              originalNewline: "lf",
              originalHasBom: false,
              originalMtime: Number(event.mtime || 0),
              originalSize: Number(event.total_size || 0),
              readOnly: Boolean(event.read_only),
              savedAt: 0,
              saveInFlight: false,
              undoStack: [],
              redoStack: [],
            };
            frontendUnits.branchesFileTreeSurface.renderFileTreeViewer(event.id);
            break;
          }
          case "file_content_saved": {
            const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
              event.id,
            );
            const v = state.viewer;
            v.dirty = false;
            v.saveInFlight = false;
            v.savedAt = Date.now();
            v.originalMtime = Number(event.new_mtime || 0);
            v.originalSize = Number(event.new_size || 0);
            // Snapshot current edit as the new baseline.
            if (v.mode === "text") {
              v.originalText = v.text;
            } else if (v.mode === "hex") {
              v.originalBytes = decodeBase64ToBytes(v.hexBytes || "");
            }
            // Resume any pending navigation queued behind the Discard modal.
            frontendUnits.branchesFileTreeSurface.applyAfterSaveContinuation(event.id);
            frontendUnits.branchesFileTreeSurface.renderFileTreeViewer(event.id);
            break;
          }
          case "file_content_save_error": {
            const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
              event.id,
            );
            const v = state.viewer;
            v.saveInFlight = false;
            const kind = (event.error_kind || "").toString();
            if (kind === "conflict") {
              state.conflictModal = {
                open: true,
                currentMtime: Number(event.current_mtime || 0),
                currentSize: Number(event.current_size || 0),
                pendingPayload: state.lastSavePayload || null,
              };
              frontendUnits.branchesFileTreeSurface.renderConflictModal(event.id);
            } else {
              v.error = {
                kind,
                message: event.message || "",
                size: event.current_size || null,
                limit: null,
              };
              frontendUnits.branchesFileTreeSurface.renderFileTreeViewer(event.id);
            }
            // Either way the queued navigation should not silently proceed
            // on failure; we keep the dirty edit and let the user retry.
            const pending = state.discardModal && state.discardModal.pendingAction;
            if (pending && pending.queuedFromDiscard) {
              state.discardModal.pendingAction = null;
            }
            break;
          }
          case "file_content_error": {
            const state = frontendUnits.branchesFileTreeSurface.ensureFileTreeState(
              event.id,
            );
            const errorKind = (event.error_kind || "").toString();
            // SPEC-2009 amendment FR-026/029: binary detection is reported as
            // an error variant from the file_content domain. The GUI flips
            // the viewer into a "binary" notice (with a hex affordance) when
            // the user attempted a text read; everything else surfaces the
            // raw notice.
            if (errorKind === "binary_not_text" && state.viewer.mode === "loading") {
              state.viewer = {
                ...state.viewer,
                path: event.path,
                mode: "binary",
                text: "",
                encoding: "",
                totalSize: event.size || state.viewer.totalSize || 0,
                hexOffset: 0,
                hexBytes: "",
                error: { kind: errorKind, message: event.message || "", size: event.size, limit: event.limit },
              };
            } else {
              state.viewer = {
                ...state.viewer,
                path: event.path,
                mode: "error",
                text: "",
                encoding: "",
                totalSize: event.size || 0,
                hexOffset: 0,
                hexBytes: "",
                error: { kind: errorKind, message: event.message || "", size: event.size, limit: event.limit },
              };
            }
            frontendUnits.branchesFileTreeSurface.renderFileTreeViewer(event.id);
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
            // SPEC-2356 — feed branch count into the Operator Status Strip WK
            // cell via develop's guarded telemetry helper. The dead Sidebar
            // Layers `git` counter was removed in the operator chrome cleanup,
            // so only `branches` is forwarded now.
            const branchesCount = Array.isArray(event.entries) ? event.entries.length : 0;
            applyOperatorTelemetryCounts({
              branches: branchesCount,
            });
            break;
          }
          case "profile_snapshot": {
            const state = frontendUnits.profileSurface.ensureProfileState(event.id);
            const previousProfile = state.selectedProfile;
            const wasSaveInFlight = state.saveInFlight;
            state.snapshot = event.snapshot || null;
            state.loading = false;
            state.saving = Boolean(state.saveTimer);
            state.saveInFlight = false;
            state.error = "";
            state.selectedProfile = event.snapshot?.selected_profile || null;
            const selectedProfileUnchanged =
              !previousProfile || previousProfile === state.selectedProfile;
            if (
              wasSaveInFlight &&
              selectedProfileUnchanged &&
              frontendUnits.profileSurface.hasEditableFocus(event.id)
            ) {
              frontendUnits.profileSurface.updateProfileStatus(event.id);
              break;
            }
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
              if ((state.composerTitle || "").trim() === (pendingSubmit.title || "")) {
                state.composerTitle = "";
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
          case "process_line":
            // SPEC-2809 Phase F1/F2 — fan out the redacted, ANSI-stripped
            // line from `ProcessConsoleHub` to every Console window
            // controller. Phase F3 also wires Logs window's Process facet
            // to consume the same event independently.
            if (event.line) {
              broadcastProcessLineToConsoles(event.line);
            }
            break;
          case "process_console_snapshot": {
            // SPEC-2809 Phase F2 — initial ring buffer replay for the
            // Console window that just mounted.
            const target = consoleControllers.get(event.id);
            if (target) {
              target.ingestSnapshot(event.lines || []);
            }
            break;
          }
          case "knowledge_entries": {
            const state = frontendUnits.knowledgeSettingsSurface.ensureKnowledgeBridgeState(
              event.id,
              event.knowledge_kind,
            );
            if (event.request_id && event.request_id !== state.loadRequestId) {
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
            if (!frontendUnits.knowledgeSettingsSurface.knowledgeDetailRequestMatches(state, event)) {
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
              workspaceOverviewSurface.renderWindows();
              break;
            }
            frontendUnits.branchesFileTreeSurface.renderBranches(event.id);
            break;
          }
          case "branch_cleanup_progress": {
            frontendUnits.branchesFileTreeSurface.updateBranchCleanupProgress(
              event.id,
              event,
            );
            branchCleanupWindowId = event.id;
            if (event.id === WORKSPACE_CLEANUP_WINDOW_ID) {
              frontendUnits.branchesFileTreeSurface.renderBranchCleanupModal();
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
            state.saveInFlight = false;
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
              if (fresh) {
                replaceKnowledgeEntry(state.entries, fresh);
                replaceKnowledgeEntry(state.baseEntries, fresh);
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
                event.query !== state.query.trim())
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
              !frontendUnits.knowledgeSettingsSurface.knowledgeDetailRequestMatches(state, event)
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
          case "launch_wizard_open_error":
            // Issue #2698 PR 1 (B7) — defer when user is mid-dropdown.
            if (
              wizardInteractionGuard.defer({
                kind: "launch_wizard_open_error",
                title: event.title,
                message: event.message,
              })
            ) {
              break;
            }
            clearLaunchWizardPendingAction();
            clearLaunchWizardOpening();
            launchWizard = null;
            launchWizardOpenError = {
              title: event.title || "Launch Agent",
              message: event.message || "Unable to open Launch Wizard",
            };
            frontendUnits.launchWizardSurface.render();
            break;
          // SPEC-2359 US-42 — Resume Picker dispatcher slots.
          case "workspace_resumable_agents":
            workspaceResumePicker.handleAgentsList(event);
            break;
          case "workspace_resume_agent_error":
            workspaceResumePicker.handleError(event);
            break;
          case "launch_wizard_state":
            // Issue #2698 PR 1 (B7) — defer when user is mid-dropdown.
            if (
              wizardInteractionGuard.defer({
                kind: "launch_wizard_state",
                wizard: event.wizard,
              })
            ) {
              break;
            }
            clearLaunchWizardPendingAction();
            clearLaunchWizardOpening();
            if (event.wizard) {
              launchWizardOpenError = null;
            }
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
          case "update_progress":
            updateCtaController.handleUpdateProgress({
              downloaded: event.downloaded,
              total: event.total,
              asset: event.asset,
              version: event.version,
            });
            break;
          case "update_ready":
            updateCtaController.handleUpdateReady({
              version: event.version,
              asset_path: event.asset_path,
            });
            break;
          case "update_apply_error":
            updateCtaController.handleUpdateApplyError({
              stage: event.stage,
              reason: event.reason || event.message,
              log_path: event.log_path,
            });
            break;
          case "update_apply_pending_persisted":
            updateCtaController.handleUpdateApplyPendingPersisted({
              version: event.version,
            });
            break;
          case "custom_agent_list":
            customAgentsState.agents = event.agents || [];
            customAgentsState.loading = false;
            frontendUnits.knowledgeSettingsSurface.renderSettingsAgentList();
            break;
          // SPEC-1921 2026-05-18 amendment / FR-099: Agent Backends WebSocket
          // events. `agent_backend_list` is a snapshot reply per
          // BuiltinAgentId; `agent_backend_saved` / `agent_backend_deleted`
          // are ephemeral mutations; `agent_backend_error` carries a stable
          // CustomAgentErrorCode tag.
          case "agent_backend_list":
            if (event.agent) {
              const key = event.agent;
              agentBackendsState.backends[key] = event.backends || [];
              agentBackendsState.loadingAgent = null;
              frontendUnits.knowledgeSettingsSurface.renderAgentBackendsPanelInAllSettingsWindows();
            }
            break;
          case "agent_backend_saved":
            if (event.agent && event.profile) {
              const key = event.agent;
              const list = agentBackendsState.backends[key] || [];
              const idx = list.findIndex((p) => p.id === event.profile.id);
              if (idx >= 0) list[idx] = event.profile;
              else list.push(event.profile);
              agentBackendsState.backends[key] = list;
              setSettingsStatus(
                `Saved backend "${event.profile.id}" for ${key}.`,
                "success",
              );
              frontendUnits.knowledgeSettingsSurface.renderAgentBackendsPanelInAllSettingsWindows();
            }
            break;
          case "agent_backend_deleted":
            if (event.agent && event.id) {
              const key = event.agent;
              agentBackendsState.backends[key] = (
                agentBackendsState.backends[key] || []
              ).filter((p) => p.id !== event.id);
              setSettingsStatus(
                `Deleted backend "${event.id}" for ${key}.`,
                "success",
              );
              frontendUnits.knowledgeSettingsSurface.renderAgentBackendsPanelInAllSettingsWindows();
            }
            break;
          case "agent_backend_error":
            setSettingsStatus(
              event.message || "Agent backend error.",
              "error",
            );
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
            if (editingCustomAgentId === event.agent_id) {
              editingCustomAgentId = null;
            }
            setSettingsStatus(`Deleted custom agent "${event.agent_id}".`, "success");
            break;
          case "board_auth_status":
            // SPEC-2963: remote provider sign-in state + editable config view.
            systemSettingsState.boardAuth = {
              slack: event.slack === true,
              teams: event.teams === true,
            };
            systemSettingsState.boardAuthMessage = event.message || "";
            systemSettingsState.boardConfig = {
              slackClientId: event.slack_client_id || "",
              slackDefaultChannel: event.slack_default_channel || "",
              slackHasSecret: event.slack_has_secret === true,
              teamsClientId: event.teams_client_id || "",
              teamsTenantId: event.teams_tenant_id || "",
              teamsDefaultChannel: event.teams_default_channel || "",
              oauthRedirectPort: event.oauth_redirect_port || 8765,
            };
            renderSystemPanelInAllSettingsWindows();
            break;
          case "system_settings":
            // SPEC-1933 US-4: backend echoed the on-disk language value.
            // Issue #2698 PR 4 — defer when user is mid-dropdown.
            if (
              systemSettingsInteractionGuard.defer({
                kind: "system_settings",
                language: event.language,
                codex_trust_managed_hooks: event.codex_trust_managed_hooks,
                board_provider: event.board_provider,
              })
            ) {
              break;
            }
            systemSettingsState.language = event.language || "auto";
            systemSettingsState.codexTrustManagedHooks =
              event.codex_trust_managed_hooks !== false;
            systemSettingsState.boardProvider =
              event.board_provider || systemSettingsState.boardProvider || "local";
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
            // Issue #2698 PR 4 — defer when user is mid-dropdown.
            if (
              systemSettingsInteractionGuard.defer({
                kind: "system_settings_updated",
                language: event.language,
                codex_trust_managed_hooks: event.codex_trust_managed_hooks,
                board_provider: event.board_provider,
              })
            ) {
              break;
            }
            systemSettingsState.language = event.language || systemSettingsState.language;
            systemSettingsState.codexTrustManagedHooks =
              event.codex_trust_managed_hooks !== false;
            systemSettingsState.boardProvider =
              event.board_provider || systemSettingsState.boardProvider || "local";
            systemSettingsState.statusMessage = "Saved system settings.";
            systemSettingsState.statusKind = "success";
            renderSystemPanelInAllSettingsWindows();
            break;
          case "system_settings_error":
            // Issue #2698 PR 4 — defer when user is mid-dropdown.
            if (
              systemSettingsInteractionGuard.defer({
                kind: "system_settings_error",
                message: event.message,
              })
            ) {
              break;
            }
            systemSettingsState.statusMessage = event.message || "Failed to update system settings.";
            systemSettingsState.statusKind = "error";
            renderSystemPanelInAllSettingsWindows();
            break;
          case "autostart_status": {
            const wasPending = systemSettingsState.autostartPending === true;
            if (
              systemSettingsInteractionGuard.defer({
                kind: "autostart_status",
                enabled: event.enabled,
                mechanism: event.mechanism,
                install_path: event.install_path,
                from_update: wasPending,
              })
            ) {
              break;
            }
            applyAutostartStatus(
              event,
              wasPending ? "Saved login launch setting." : "",
            );
            renderSystemPanelInAllSettingsWindows();
            break;
          }
          case "autostart_error":
            if (
              systemSettingsInteractionGuard.defer({
                kind: "autostart_error",
                message: event.message,
              })
            ) {
              break;
            }
            applyAutostartError(event);
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
        if (panState && panState.pointerId !== event.pointerId) {
          tracePointer(UI_TRACE_EVENT.pointerMoveIgnored, event, {
            gesture: "pan",
            accepted: false,
            reason: "pointer_id_mismatch",
            expected_pointer_id: panState.pointerId,
          });
        }
        if (panState && panState.pointerId === event.pointerId) {
          tracePointer(UI_TRACE_EVENT.pointerPanMove, event, {
            gesture: "pan",
            accepted: true,
          });
          viewport.x = panState.x + event.clientX - panState.startX;
          viewport.y = panState.y + event.clientY - panState.startY;
          recordLocalViewportEdit();
          applyViewport();
          return;
        }

        if (dragState && dragState.pointerId !== event.pointerId) {
          tracePointer(UI_TRACE_EVENT.pointerMoveIgnored, event, {
            gesture: "drag",
            accepted: false,
            reason: "pointer_id_mismatch",
            expected_pointer_id: dragState.pointerId,
            window_id: dragState.id,
          });
        }
        if (dragState && dragState.pointerId === event.pointerId) {
          const element = windowMap.get(dragState.id);
          if (!element) {
            return;
          }
          tracePointer(UI_TRACE_EVENT.pointerDragMove, event, {
            gesture: "drag",
            accepted: true,
            window_id: dragState.id,
          });
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

        if (resizeState && resizeState.pointerId !== event.pointerId) {
          tracePointer(UI_TRACE_EVENT.pointerMoveIgnored, event, {
            gesture: "resize",
            accepted: false,
            reason: "pointer_id_mismatch",
            expected_pointer_id: resizeState.pointerId,
            window_id: resizeState.id,
          });
        }
        if (resizeState && resizeState.pointerId === event.pointerId) {
          // SPEC-2014 Phase C4: store the latest pointer coordinates and
          // batch the actual DOM mutation via requestAnimationFrame. The
          // previous implementation wrote `element.style.width/height` on
          // every pointermove (potentially 200+ times per second on high
          // refresh rate displays), triggering layout reflow at the same
          // rate. On Windows WebView2 this can starve the render thread and
          // surface as the resize freeze users reported requiring an app
          // restart. By coalescing to one apply per frame we keep the visual
          // responsiveness while letting WebView2 paint between updates.
          tracePointer(UI_TRACE_EVENT.pointerResizeMove, event, {
            gesture: "resize",
            accepted: true,
            window_id: resizeState.id,
          });
          resizeState.latestClientX = event.clientX;
          resizeState.latestClientY = event.clientY;
          scheduleResizePointermoveApply();
        }
      });

      window.addEventListener("dragover", trackWindowTabDragPoint);

      window.addEventListener("pointerup", (event) => {
        if (panState && panState.pointerId === event.pointerId) {
          tracePointer(UI_TRACE_EVENT.pointerPanEnd, event, {
            gesture: "pan",
            accepted: true,
          });
          canvas.classList.remove("panning");
          // Issue #2698 PR 2 (B1) — pan-end is a definitive commit
          // point. Flush the throttle so backend receives the final
          // viewport without a tail-debounce delay.
          flushPersistViewport();
          panState = null;
        } else if (panState) {
          tracePointer(UI_TRACE_EVENT.pointerUpIgnored, event, {
            gesture: "pan",
            accepted: false,
            reason: "pointer_id_mismatch",
            expected_pointer_id: panState.pointerId,
          });
        }

        if (dragState && dragState.pointerId === event.pointerId) {
          tracePointer(UI_TRACE_EVENT.pointerDragEnd, event, {
            gesture: "drag",
            accepted: true,
            window_id: dragState.id,
          });
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
        } else if (dragState) {
          tracePointer(UI_TRACE_EVENT.pointerUpIgnored, event, {
            gesture: "drag",
            accepted: false,
            reason: "pointer_id_mismatch",
            expected_pointer_id: dragState.pointerId,
            window_id: dragState.id,
          });
        }

        if (resizeState) {
          if (resizeState.pointerId === event.pointerId) {
            tracePointer(UI_TRACE_EVENT.pointerResizeEnd, event, {
              gesture: "resize",
              accepted: true,
              window_id: resizeState.id,
            });
            finishWindowResize(event.pointerId, event);
          } else {
            // SPEC-2008 Phase 26.C / FR-059 — Windows WebView2 sometimes
            // emits a `pointerup` whose pointerId does not match the one
            // we captured at pointerdown (the OS pre-released capture
            // and re-issued a fresh pointer). When that happens neither
            // the resizeHandle's `lostpointercapture` listener nor the
            // pointerId-gated branch above ever runs, so resizeState
            // stays alive until the 30s staleness guard finally tears
            // it down. Force the cleanup immediately on any window
            // pointerup while a resize is pending so the user is never
            // wedged into a stuck resize that requires an app restart.
            console.warn(
              `[resize] window pointerup pointerId mismatch (resizeState.pointerId=${resizeState.pointerId}, event.pointerId=${event.pointerId}); forcing cleanup`,
            );
            tracePointer(UI_TRACE_EVENT.pointerUpIgnored, event, {
              gesture: "resize",
              accepted: false,
              reason: "pointer_id_mismatch_force_reset",
              expected_pointer_id: resizeState.pointerId,
              window_id: resizeState.id,
            });
            forceResetResizeState("window pointerup pointerId mismatch");
          }
        }
      });

      window.addEventListener("pointercancel", (event) => {
        if (panState) {
          tracePointer(
            panState.pointerId === event.pointerId
              ? UI_TRACE_EVENT.pointerPanCancel
              : UI_TRACE_EVENT.pointerCancelIgnored,
            event,
            {
              gesture: "pan",
              accepted: panState.pointerId === event.pointerId,
              reason: panState.pointerId === event.pointerId
                ? "pointer_cancel"
                : "pointer_id_mismatch",
              expected_pointer_id: panState.pointerId,
            },
          );
        }
        if (dragState && dragState.pointerId === event.pointerId) {
          tracePointer(UI_TRACE_EVENT.pointerDragCancel, event, {
            gesture: "drag",
            accepted: true,
            window_id: dragState.id,
          });
          clearTitlebarDockPreview();
          dragState = null;
        } else if (dragState) {
          tracePointer(UI_TRACE_EVENT.pointerCancelIgnored, event, {
            gesture: "drag",
            accepted: false,
            reason: "pointer_id_mismatch",
            expected_pointer_id: dragState.pointerId,
            window_id: dragState.id,
          });
        }
        if (resizeState && resizeState.pointerId !== event.pointerId) {
          // SPEC-2008 Phase 26.C / FR-059 — same pointerId-mismatch
          // safety as the pointerup handler above. pointercancel from a
          // different pointerId still indicates the original capture
          // is gone; do not leave resizeState alive across cancellations.
          console.warn(
            `[resize] window pointercancel pointerId mismatch (resizeState.pointerId=${resizeState.pointerId}, event.pointerId=${event.pointerId}); forcing cleanup`,
          );
          tracePointer(UI_TRACE_EVENT.pointerCancelIgnored, event, {
            gesture: "resize",
            accepted: false,
            reason: "pointer_id_mismatch_force_reset",
            expected_pointer_id: resizeState.pointerId,
            window_id: resizeState.id,
          });
          forceResetResizeState("window pointercancel pointerId mismatch");
          return;
        }
        if (resizeState) {
          tracePointer(UI_TRACE_EVENT.pointerResizeCancel, event, {
            gesture: "resize",
            accepted: true,
            window_id: resizeState.id,
          });
        }
        finishWindowResize(event.pointerId, event);
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
          tracePointer(UI_TRACE_EVENT.pointerPanStart, event, {
            gesture: "pan",
            accepted: true,
            button_mode: "middle",
          });
          canvas.classList.add("panning");
          try {
            canvas.setPointerCapture(event.pointerId);
            tracePointer(UI_TRACE_EVENT.pointerCaptureSet, event, {
              gesture: "pan",
              accepted: true,
              button_mode: "middle",
            });
          } catch (error) {
            tracePointer(UI_TRACE_EVENT.pointerCaptureFailed, event, {
              gesture: "pan",
              accepted: false,
              reason: "set_pointer_capture_failed",
              button_mode: "middle",
              error_name: error && error.name,
            });
            throw error;
          }
        },
        { capture: true },
      );

      // Left button pan: only on empty canvas area (not over windows).
      canvas.addEventListener("pointerdown", (event) => {
        if (event.button !== 0) {
          return;
        }
        if (event.target !== canvas && event.target !== stage) {
          tracePointer(UI_TRACE_EVENT.pointerDownIgnored, event, {
            gesture: "pan",
            accepted: false,
            reason: "target_not_canvas",
          });
          return;
        }
        panState = {
          pointerId: event.pointerId,
          startX: event.clientX,
          startY: event.clientY,
          x: viewport.x,
          y: viewport.y,
        };
        tracePointer(UI_TRACE_EVENT.pointerPanStart, event, {
          gesture: "pan",
          accepted: true,
          button_mode: "left",
        });
        canvas.classList.add("panning");
        try {
          canvas.setPointerCapture(event.pointerId);
          tracePointer(UI_TRACE_EVENT.pointerCaptureSet, event, {
            gesture: "pan",
            accepted: true,
            button_mode: "left",
          });
        } catch (error) {
          tracePointer(UI_TRACE_EVENT.pointerCaptureFailed, event, {
            gesture: "pan",
            accepted: false,
            reason: "set_pointer_capture_failed",
            button_mode: "left",
            error_name: error && error.name,
          });
          throw error;
        }
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
        const wheelMode = canvasWheelGestureClassifier.classify(event);
        // SPEC-2008 FR-032: terminal-only opt-out. xterm.js owns wheel inside
        // `.surface-terminal`; every other workspace-window forwards plain
        // wheel to the DOM so panel scroll regions (Knowledge / Profile /
        // Logs / Board / Issue / SPEC / Settings ...) and modal
        // content scroll natively without registering a per-class whitelist.
        if (
          wheelMode !== "zoom" &&
          targetElement.closest(".surface-terminal")
        ) {
          return;
        }
        if (
          wheelMode !== "zoom" &&
          targetElement.closest(".workspace-window")
        ) {
          return;
        }
        if (wheelMode === "zoom") {
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
          recordLocalViewportEdit();
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
      installBrowserFileDropBridge();
      installNativeFileDropBridge();

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
      pickerCloneProjectButton.addEventListener("click", openCloneProjectModal);
      onboardingOpenProjectButton.addEventListener(
        "click",
        frontendUnits.projectWorkspaceShell.sendOpenProjectDialog,
      );

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
          frontendUnits.projectWorkspaceShell.sendOpenProjectDialog();
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
      wizardCancelButton.addEventListener("click", closeLaunchWizardFromChrome);
      wizardBackButton.addEventListener("click", () => {
        if (
          !releaseWizardInteractionGuardForChromeAction()
          || launchWizardOpenError
          || wizardBackButton.disabled
        ) {
          return;
        }
        frontendUnits.launchWizardSurface.flushBranchDraft();
        frontendUnits.launchWizardSurface.sendAction({ kind: "back" });
      });
      wizardSubmitButton.addEventListener("pointerup", (event) => {
        if (!isPrimaryPointerActivation(event)) {
          return;
        }
        event.preventDefault();
        handleLaunchWizardSubmitFromChrome();
      });
      wizardSubmitButton.addEventListener("click", handleLaunchWizardSubmitFromChrome);
      // Issue #2698 PR 1 (B7) — interaction-guard wiring for native
      // <select> dropdowns inside the wizard body. We register
      // delegated listeners on `wizardBody` so they survive the
      // destructive re-render of its children. Activation on
      // pointerdown matches when the OS overlay opens; release on
      // `change` (commit), `focusout` (cancel), and `Escape` covers
      // every common termination path.
      // SPEC-2014 2026-05-29: the reasoning slider (`.launch-range__input`)
      // needs the same guard. Each value change commits set_reasoning, and the
      // backend echoes launch_wizard_state; without deferral that re-render
      // destroys and recreates the slider mid-drag (breaking the drag) and
      // drops keyboard focus between Arrow steps. Activate on focusin (covers
      // mouse press and keyboard tab-in) and release on focusout so re-renders
      // are coalesced for the whole interaction, not committed on every step.
      const isGuardedRange = (el) =>
        Boolean(el && el.classList && el.classList.contains("launch-range__input"));
      wizardBody.addEventListener("pointerdown", (event) => {
        const target = event.target;
        if (target && (target.tagName === "SELECT" || isGuardedRange(target))) {
          wizardInteractionGuard.activate();
        }
      });
      wizardBody.addEventListener("focusin", (event) => {
        if (isGuardedRange(event.target)) {
          wizardInteractionGuard.activate();
        }
      });
      wizardBody.addEventListener("change", (event) => {
        const target = event.target;
        // <select> commits on change; the range keeps the guard active across
        // multiple Arrow steps / the post-drag focused state until focusout.
        if (target && target.tagName === "SELECT") {
          wizardInteractionGuard.release();
        }
      });
      wizardBody.addEventListener("focusout", (event) => {
        const target = event.target;
        if (target && (target.tagName === "SELECT" || isGuardedRange(target))) {
          wizardInteractionGuard.release();
        }
      });
      wizardModal.addEventListener("keydown", (event) => {
        if (event.key === "Escape" && wizardInteractionGuard.isActive()) {
          wizardInteractionGuard.release();
        }
      });
      // Issue #2698 PR 4 — apply the same interaction-guard pattern
      // to the System Settings Output Language `<select>`. Settings
      // windows can stack and reflow, so we delegate from the
      // document root and filter by the unique `settings-select`
      // class. Listeners use the bubble phase to stay consistent
      // with the wizard wiring above.
      document.addEventListener("pointerdown", (event) => {
        const target = event.target;
        if (
          target
          && target.tagName === "SELECT"
          && target.classList.contains("settings-select")
        ) {
          systemSettingsInteractionGuard.activate();
        }
      });
      document.addEventListener("change", (event) => {
        const target = event.target;
        if (
          target
          && target.tagName === "SELECT"
          && target.classList.contains("settings-select")
        ) {
          systemSettingsInteractionGuard.release();
        }
      });
      document.addEventListener("focusout", (event) => {
        const target = event.target;
        if (
          target
          && target.tagName === "SELECT"
          && target.classList.contains("settings-select")
        ) {
          systemSettingsInteractionGuard.release();
        }
      });
      document.addEventListener("keydown", (event) => {
        if (
          event.key === "Escape"
          && systemSettingsInteractionGuard.isActive()
        ) {
          systemSettingsInteractionGuard.release();
        }
      });
      cloneProjectModal.addEventListener("click", (event) => {
        if (event.target === cloneProjectModal) {
          closeCloneProjectModal();
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
      if (workspaceOverviewEntry) {
        workspaceOverviewEntry.addEventListener("click", openWorkspaceOverview);
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
          if (!releaseWizardInteractionGuardForChromeAction()) {
            event.preventDefault();
            return;
          }
          if (launchWizardOpenError) {
            closeLaunchWizardLocal();
          } else {
            frontendUnits.launchWizardSurface.sendAction({ kind: "cancel" });
          }
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
        if (cloneProjectModal && cloneProjectModal.classList.contains("open")) {
          closeCloneProjectModal();
          event.preventDefault();
          return;
        }
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
      // SPEC-2008 Phase 24 / T-187: host resize must fan out `fitTerminal
      // (persist=true)` to every visible terminal so xterm cols/rows stay
      // aligned with the viewport and `UpdateWindowGeometry` reaches the
      // backend PTY. The fan-out lives in `terminal-viewport-reflow.js`
      // so the behavior is exercised by linkedom unit tests rather than
      // only source-string contract.
      attachHostResizeReflow({
        window,
        terminalIds: () => terminalMap.keys(),
        canRefreshViewport: canRefreshTerminalViewport,
        fitTerminal: scheduleTerminalFit,
        beforeFan: () => {
          frontendUnits.projectWorkspaceShell.renderWindowList();
          syncMaximizedWindowsToViewport();
        },
      });
      document.addEventListener("visibilitychange", () => {
        if (document.hidden) {
          return;
        }
        rearmVisibleTerminalViewportRefreshes();
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

      // SPEC-2356 "Surface Deck" — arrow-key roving + Enter-to-deploy. Esc is
      // already handled by the global modal close handler, so the roving
      // listener only needs the directional keys and Enter.
      modal.addEventListener("keydown", handlePresetRovingKeydown);

      // SPEC-2356 — bridge Command Palette + hotkey commands into existing
      // surface dispatch. Each command either focuses an existing window or
      // creates a new one through the same socket transport the preset
      // buttons use, so they share the legacy invariants.
      function isAutoMaximizedSurfacePreset(_preset) {
        return false;
      }

      function focusOrSpawnPreset(preset) {
        if (preset === "branches") preset = "work";
        const allWindows = activeWorkspace().windows || [];
        const existing = allWindows.find(
          (w) => w.preset === preset && !w.minimized,
        );
        if (existing) {
          const message = {
            kind: "focus_window",
            id: existing.id,
          };
          if (isAutoMaximizedSurfacePreset(preset)) {
            message.bounds = visibleBounds();
          }
          frontendUnits.socketTransport.send(message);
          return existing.id;
        }
        const message = {
          kind: "create_window",
          preset,
          bounds: visibleBounds(),
        };
        frontendUnits.socketTransport.send(message);
        return null;
      }

      document.addEventListener("op:command", (event) => {
        const id = event.detail?.id;
        if (!id) return;
        switch (id) {
          case "open-board":
            focusOrSpawnPreset("board");
            return;
          case "open-git":
            focusOrSpawnPreset("work");
            return;
          case "open-logs":
            focusOrSpawnPreset("logs");
            return;
          case "open-branches":
            focusOrSpawnPreset("work");
            return;
          case "open-files":
            focusOrSpawnPreset("file_tree");
            return;
          case "open-index":
            focusOrSpawnPreset("index");
            return;
          case "spawn-shell":
            focusOrSpawnPreset("shell");
            return;
          case "start-work":
          case "spawn-agent":
            openStartWorkPendingWizard();
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

      function installPlaywrightTestBridge() {
        if (window.__gwtPlaywrightTestBridge !== true) {
          return;
        }
        if (window.__gwtPlaywrightTestBridgeInstalled === true) {
          return;
        }
        window.__gwtPlaywrightTestBridgeInstalled = true;
        window.addEventListener("__gwt_test_inject", (event) => {
          const detail = event && event.detail;
          if (!detail || typeof detail.kind !== "string") {
            return;
          }
          receive(detail);
        });
        window.addEventListener("__gwt_test_send", (event) => {
          const detail = event && event.detail;
          if (!detail || typeof detail.kind !== "string") {
            return;
          }
          send(detail);
        });
        window.__gwtTerminalTestApi = Object.freeze({
          metrics(windowId) {
            const runtime = terminalMap.get(windowId);
            const terminal = runtime?.terminal;
            const buffer = terminal?.buffer?.active;
            const viewport = terminal?.element
              ?.parentElement
              ?.querySelector?.(".xterm-viewport");
            return {
              hasRuntime: Boolean(runtime),
              isReady: runtime?.isReady ?? null,
              viewportRefreshPending: runtime?.viewportRefreshPending === true,
              cols: terminal?.cols ?? 0,
              rows: terminal?.rows ?? 0,
              baseY: buffer?.baseY ?? 0,
              viewportY: buffer?.viewportY ?? 0,
              bufferLength: buffer?.length ?? 0,
              domScrollTop: viewport?.scrollTop ?? 0,
              domScrollHeight: viewport?.scrollHeight ?? 0,
              domClientHeight: viewport?.clientHeight ?? 0,
            };
          },
          scrollToBottom(windowId) {
            const terminal = terminalMap.get(windowId)?.terminal;
            if (terminal && typeof terminal.scrollToBottom === "function") {
              terminal.scrollToBottom();
            }
          },
        });
      }

      frontendUnits.projectWorkspaceShell.renderAppState(appState);
      installPlaywrightTestBridge();
      frontendUnits.socketTransport.connect();
