      import { Terminal } from "/assets/xterm/xterm.mjs";
      import { FitAddon } from "/assets/xterm/addon-fit.mjs";
      // SPEC-3064 Phase 3 (E7): the migration-modal / project-clone-modal /
      // project-tabs-renderer / close-project-tab-confirm-modal view imports
      // moved to /project-shell-surface.js with the project shell chrome.
      import {
        initOperatorShell,
        applyTelemetryCounts,
        applyIssueMonitorStatus,
        applyProviderUsage,
        applyRuntimeHealth,
      } from "/operator-shell.js";
      import { createFocusTrap } from "/focus-trap.js";
      import {
        TITLEBAR_DOCK_HIT_HEIGHT,
        clientPointFromDragEvent,
        detachGeometryFromClientPoint,
        findTitlebarDockTarget,
        resolveDragReleasePoint,
      } from "/window-docking.js";
      import { createWorkspaceKanbanSurface as createWorkspaceOverviewSurface } from "/workspace-kanban-surface.js";
      import { createImprovementInboxSurface } from "/improvement-inbox-surface.js";
      import {
        createAgentKanbanPendingPlacementController,
        createAgentKanbanSurface,
        findAgentKanbanDropTargetAtPoint,
        isAgentKanbanEligible,
        isAgentKanbanPlacement,
        placeAgentWindowMessage,
        updateTerminalGridMessage,
      } from "/agent-kanban-surface.js";
      import { createProviderUsageSurface } from "/provider-usage-surface.js";
      import { createTerminalAttachments } from "/terminal-attachments.js";
      import { createProjectIndexSearchSurface } from "/project-index-search-surface.js";
      import { createWorkspaceResumePickerController } from "/workspace-resume-picker-modal.js";
      import { createLaunchPendingController } from "/launch-pending-controller.js";
      import { createConnectionOverlay } from "/connection-overlay.js";
      import { createUpdateCtaController } from "/update-cta.js";
      // SPEC-2356 Anshin Addendum (FR-040): the in-app attention toaster ships
      // alongside the away-only desktop notifier in the same module.
      import {
        createAgentCompletionNotifier,
        createAgentAttentionToaster,
      } from "/agent-completion-notifications.js";
      import { createReleaseNotesWindow } from "/release-notes-window.js";
      import { createConsoleWindow } from "/console-window.js";
      import {
        computeCameraFrameArea,
        computeViewportForWorldRect,
        expandWorldRectForLayoutSize,
      } from "/camera-framing.js";
      import { createTerminalWheelScrollController } from "/terminal-wheel-scroll.js";
      import { renderWindowTabs as renderWindowTabsView } from "/window-tabs-renderer.js";
      // SPEC-3038 US-3: Close Guard confirm modal renderer.
      import { renderWindowCloseConfirmModal } from "/window-close-confirm-modal.js";
      // SPEC-3064 Phase 3 (E4): the Settings windows surface (and its
      // index-settings-panel / custom-agent-env-editor imports) moved to
      // /settings-surface.js.
      import { createSettingsSurface } from "/settings-surface.js";
      // SPEC-3064 Phase 3 (E5): the Launch Wizard surface (and its
      // launch-controls / interaction-guard imports) moved to
      // /launch-wizard-surface.js.
      import { createLaunchWizardSurface } from "/launch-wizard-surface.js";
      import { createIssueMonitorSurface } from "/issue-monitor-surface.js";
      // SPEC-3064 Phase 3 (E6a): the File Tree window surface moved to
      // /file-tree-surface.js.
      import { createFileTreeSurface } from "/file-tree-surface.js";
      // SPEC-3064 Phase 3 (E6b): the Branches window & cleanup surface moved
      // to /branches-cleanup-surface.js.
      import { createBranchesCleanupSurface } from "/branches-cleanup-surface.js";
      // SPEC-3064 Phase 3 (E6c): the Board & Logs window surface (and its
      // board-surface.js helper imports) moved to /board-logs-surface.js.
      import { createBoardLogsSurface } from "/board-logs-surface.js";
      // SPEC-3064 Phase 3 (E6d): the Knowledge Bridge (Kanban) window surface
      // moved to /knowledge-kanban-surface.js.
      import { createKnowledgeKanbanSurface } from "/knowledge-kanban-surface.js";
      // SPEC-3064 Phase 3 (E6e): the Profile window surface moved to
      // /profile-window-surface.js.
      import { createProfileWindowSurface } from "/profile-window-surface.js";
      // SPEC-3064 Phase 3 (E7): the project & workspace shell chrome
      // (project tabs + close-tab confirm modal + tab cues, recent projects,
      // open-project menu, picker/onboarding renderers, action availability,
      // window list dropdown, maximized-window viewport sync, open/clone
      // project modal glue, migration modal glue) moved to
      // /project-shell-surface.js.
      import { createProjectShellSurface } from "/project-shell-surface.js";
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
        resizeGeometryFromPointerState,
        shouldApplyWorkspaceGeometry,
        syncResizeStatePointerEvent,
        workspaceGeometryRevision,
      } from "/window-geometry-sync.js";
      import { createSocketReceiveDispatcher } from "/socket-receive-dispatcher.js";
      import { createTerminalOutputBatcher } from "/terminal-output-buffer.js";
      // SPEC-3064 Phase 3 (E6b): markBranchDetailInterrupted and
      // branchLoadStatusSummary moved with the branches cleanup surface.
      import {
        branchWindowNeedsResync,
        applyBranchEntriesEvent,
      } from "/branch-list-state.js";
      import { createCanvasWheelGestureClassifier } from "/canvas-wheel-gesture.js";
      import { createViewportPersistThrottle } from "/viewport-persist-throttle.js";
      import { createViewportSyncState } from "/viewport-sync.js";
      // SPEC-2008 camera-focus / FR-094: the always-on Fleet Minimap carrier.
      import { createFleetMinimap } from "/fleet-minimap.js";
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
        applyIssueMonitorStatus: (status) => applyIssueMonitorStatus(document, status),
        applyProviderUsage: (snapshot) => applyProviderUsage(document, snapshot),
        applyRuntimeHealth: (snapshot) =>
          applyRuntimeHealth(document, snapshot, {
            focusWindow: (windowId) => focusWindowRemotely(windowId, { center: true }),
          }),
      };

      const uiTraceProfiler = createUiTraceProfiler();

      const canvas = document.getElementById("canvas");
      const stage = document.getElementById("canvas-stage");
      // SPEC-3064 Phase 3 (E7): the project-shell chrome element refs
      // (project tabs strip, open-project button + split-button menu,
      // picker/onboarding elements, recent project list, window list
      // button/panel) moved to /project-shell-surface.js; app.js keeps the
      // toolbar buttons whose click listeners stay wired here.
      const addButton = document.getElementById("add-button");
      const tileButton = document.getElementById("tile-button");
      const stackButton = document.getElementById("stack-button");
      const alignButton = document.getElementById("align-button");
      const worldGrid = document.getElementById("canvas-world-grid");
      const workspaceOverviewEntry = document.getElementById("op-workspace-overview-entry");
      const zoomOutButton = document.getElementById("zoom-out-button");
      const zoomResetButton = document.getElementById("zoom-reset-button");
      const zoomInButton = document.getElementById("zoom-in-button");
      const modal = document.getElementById("preset-modal");
      const closeModalButton = document.getElementById("close-modal");
      // SPEC-3064 Phase 3 (E5): the other wizard chrome element refs
      // (dialog/title/meta/summary/body/error/back/cancel/submit) moved to
      // /launch-wizard-surface.js; app.js keeps wizardModal for the modal
      // open-state checks shared with focus shortcuts and terminal focus.
      const wizardModal = document.getElementById("wizard-modal");
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
      const renderedAgentKanbanBodyKeys = new Map();
      // SPEC-3064 Phase 3 (E6a): fileTreeStateMap moved into
      // /file-tree-surface.js (exported and destructured below so the
      // window-cleanup call site keeps its text).
      // SPEC-3064 Phase 3 (E6d): knowledgeBridgeStateMap and the knowledge
      // request-id state moved to /knowledge-kanban-surface.js.
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
          // SPEC-3064 Phase 3 (E7): windowListOpen / windowListEntries are
          // backed by /project-shell-surface.js; the remaining state stays
          // in app.js and is exposed to the surface through accessors.
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
      // SPEC-2008 camera-focus / FR-094: the Fleet Minimap is instantiated once
      // the DOM container resolves (see the boot block). applyViewport /
      // recomputeOperatorTelemetry call it through the optional chain so the
      // early `applyViewport()` paths run safely before it exists.
      let fleetMinimap = null;
      // SPEC-3064 Phase 3 (E5): the wizard state (launchWizard /
      // launchWizardOpenError / launchWizardOpening / branch draft /
      // pending action) and wizardInteractionGuard moved to
      // /launch-wizard-surface.js.
      let activeWorkProjection = null;
      const agentKanbanPendingPlacement =
        createAgentKanbanPendingPlacementController();
      // SPEC-3064 Phase 3 (E6b): branchCleanupWindowId and the synthetic
      // workspace-cleanup window id moved to /branches-cleanup-surface.js.
      // SPEC-3064 Phase 3 (E7): windowListOpen / windowListEntries moved to
      // /project-shell-surface.js.
      // SPEC-2008 camera-focus: titlebarClickState (the double-click maximize
      // detector) was removed; a titlebar click now always frames the window.
      let appState = {
        app_version: "",
        tabs: [],
        active_tab_id: null,
        recent_projects: [],
      };
      let improvementCandidates = [];
      let improvementCandidatesRevision = 0;
      let renderedProjectTabsKey = "";
      let renderedWorkspaceWindowsKey = "";
      let renderedAppVersionLabel = null;
      let renderedOperatorTelemetryKey = "";
      // SPEC-3064 Phase 3 (E7): the rendered-key slots for the moved chrome
      // renderers (recent projects, window list, picker, onboarding, action
      // availability) and maximizedViewportSyncFrame moved to
      // /project-shell-surface.js.

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

      // SPEC-3064 Phase 3 (E7): recentProjectsRenderKey moved to
      // /project-shell-surface.js with the recent-projects renderers.
      function appendRenderKeyPart(parts, value) {
        const text = String(value ?? "");
        parts.push(String(text.length), ":", text, "\u001f");
      }

      function appendWindowPlacementRenderKey(parts, windowData) {
        const placement = windowData?.placement || {};
        appendRenderKeyPart(parts, "placement_kind");
        appendRenderKeyPart(parts, placement.kind || "canvas");
        appendRenderKeyPart(parts, "placement_board");
        appendRenderKeyPart(parts, placement.board_id || "");
        appendRenderKeyPart(parts, "placement_lane");
        appendRenderKeyPart(parts, placement.lane_id || "");
        appendRenderKeyPart(parts, "placement_order");
        appendRenderKeyPart(parts, placement.order ?? "");
        appendRenderKeyPart(parts, "placement_collapsed");
        appendRenderKeyPart(parts, Boolean(placement.collapsed));
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
        if (windows.some((windowData) => presetSurface(windowData?.preset) === "improvement")) {
          appendRenderKeyPart(parts, "improvement_candidates_revision");
          appendRenderKeyPart(parts, improvementCandidatesRevision);
        }
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
          appendWindowPlacementRenderKey(parts, windowData);
        }
        return parts.join("");
      }

      // SPEC-3064 Phase 3 (E7): windowListRenderKey moved to
      // /project-shell-surface.js with the Window List dropdown renderer.
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
        appendWindowPlacementRenderKey(parts, windowData);
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
          appendWindowPlacementRenderKey(parts, tab);
        }
        return parts.join("");
      }

      function agentKanbanBodyRenderKey(boardWindow) {
        const parts = [];
        appendRenderKeyPart(parts, "board_id");
        appendRenderKeyPart(parts, boardWindow?.id || "");
        for (const windowData of activeWorkspace().windows || []) {
          if (!isAgentKanbanPlacement(windowData)) {
            continue;
          }
          const placement = windowData.placement || {};
          if (placement.board_id !== boardWindow?.id) {
            continue;
          }
          appendRenderKeyPart(parts, "id");
          appendRenderKeyPart(parts, windowData.id || "");
          appendRenderKeyPart(parts, "preset");
          appendRenderKeyPart(parts, windowData.preset || "");
          appendRenderKeyPart(parts, "title");
          appendRenderKeyPart(parts, windowDisplayTitle(windowData));
          appendRenderKeyPart(parts, "role");
          appendRenderKeyPart(parts, windowRoleBadgeLabel(windowData));
          appendRenderKeyPart(parts, "status");
          appendRenderKeyPart(parts, runtimeStateForWindow(windowData));
          appendWindowPlacementRenderKey(parts, windowData);
        }
        return parts.join("");
      }

      // SPEC-3064 Phase 3 (E7): projectPickerRenderKey /
      // projectOnboardingRenderKey / actionAvailabilityRenderKey and the
      // migration / clone-project modal state moved to
      // /project-shell-surface.js with their renderers.
      let versionState = { current: "", latest: "" };
      let pendingAboutHashOpen = false;
      let projectError = "";
      const TERMINAL_SELECTION_DRAG_THRESHOLD = 4;

      // SPEC-3064 Phase 3 (E3): the Project Index window surface (per-project
      // index status map, Index search state, render/search/open helpers, and
      // the Index window mount) moved to /project-index-search-surface.js.
      // app.js keeps the receive(), Settings, knowledge-mount, and
      // window-cleanup call sites and wires them through this factory.
      const {
        setIndexStatus,
        handleProjectIndexSearchResults,
        handleProjectIndexSearchError,
        mountProjectIndexSurface,
        indexSearchStateMap,
        pendingIndexOpenTargetsByPreset,
        indexStatusByProjectRoot,
      } = createProjectIndexSearchSurface({
        send,
        sendWindowFocus: (id) => socketTransport.send({ kind: "focus_window", id }),
        focusWindowLocally,
        activeProjectTab,
        makeEl,
        clearChildren,
        focusOrSpawnPreset,
        knowledgeKindForPreset,
        // SPEC-3064 Phase 3 (E6d): these two live in the knowledge-kanban
        // surface, whose factory runs after this one — close over the
        // bindings instead of passing the (not yet initialized) consts.
        requestKnowledgeDetail: (...args) => requestKnowledgeDetail(...args),
        renderKnowledgeBridge: (...args) => renderKnowledgeBridge(...args),
        // SPEC-3064 Phase 3 (E4): these two live in the settings surface,
        // whose factory runs after this one — close over the bindings
        // instead of passing the (not yet initialized) consts directly.
        renderIndexPanelInAllSettingsWindows: () =>
          renderIndexPanelInAllSettingsWindows(),
        // SPEC-3064 Phase 3 (E7): refreshProjectTabStateCues lives in the
        // project shell surface, whose factory also runs after this one —
        // close over the binding for the same reason.
        refreshProjectTabStateCues: () => refreshProjectTabStateCues(),
        requestFullIndexStatusRefresh: () => requestFullIndexStatusRefresh(),
      });

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
        if (preset === "agent_kanban") {
          return "agent-kanban";
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
        if (preset === "issue_monitor") {
          return "issue-monitor";
        }
        if (preset === "index") {
          return "index";
        }
        if (preset === "work" || preset === "workspace") {
          return "work";
        }
        if (preset === "improvement" || preset === "improvements") {
          return "improvement";
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
        // SPEC-2356 Anshin Addendum (FR-042): STOP ALL is reachable from the
        // palette as well as the rail.
        window.__operatorShell.palette.register({
          id: "stop-all-agents",
          label: "Agents: Stop All Running Agents",
          group: "Agents",
          handler: () => {
            requestStopAllWindows();
          },
        });
        // SPEC-2356 Anshin Addendum (FR-043): send a line of input to the focused
        // agent pane from the palette.
        window.__operatorShell.palette.register({
          id: "send-input-focused-agent",
          label: "Agent: Send Input to Focused Agent",
          group: "Agents",
          handler: () => {
            promptSendFocusedPaneInput();
          },
        });
      }

      // SPEC-2356 Anshin Addendum (FR-043): inject one line of input into the
      // focused agent pane. Targets the window's session_id (not its window id)
      // so the backend can scope the injection to the caller's own pane. Returns
      // true when a line was actually dispatched.
      function focusedAgentWindowData() {
        if (!focusedId) return null;
        const windowData = workspaceWindowById(focusedId);
        if (!windowData || !presetSupportsWaitingStatus(windowData.preset)) {
          return null;
        }
        return windowData;
      }

      function sendPaneInput(sessionId, text) {
        const session = String(sessionId || "").trim();
        const line = String(text ?? "");
        if (!session || line.length === 0) {
          return false;
        }
        send({ kind: "pane_send_input", session_id: session, text: line });
        return true;
      }

      function sendFocusedPaneInput(text) {
        const windowData = focusedAgentWindowData();
        const sessionId = windowData?.session_id;
        return sendPaneInput(sessionId, text);
      }

      // SPEC-2356 Anshin Addendum (FR-043): the Command Palette entry prompts for
      // a single line and routes it to the focused agent pane.
      function promptSendFocusedPaneInput() {
        const windowData = focusedAgentWindowData();
        if (!windowData) {
          window.alert("Focus an agent window first to send it input.");
          return;
        }
        if (!String(windowData.session_id || "").trim()) {
          window.alert("This agent has no active session to send input to.");
          return;
        }
        const label = windowDisplayTitle(windowData);
        const text = window.prompt(`Send a line of input to ${label}:`);
        if (text === null) {
          return;
        }
        sendFocusedPaneInput(text);
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
          releaseNotesWindow.open(versionState.latest || versionState.current || null);
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

      // SPEC-2359 W-17 (FR-399): explicit full-screen overlay while the
      // WebSocket bridge is down — a tiny status-strip label alone reads as
      // a frozen app when every click needs the socket.
      const connectionOverlay = createConnectionOverlay({ document });

      function setConnectionState(connected) {
        connectionOverlay.setConnected(connected);
        // SPEC-3038 US-4: the Status Strip (plus the SPEC-2359 W-17 full-
        // screen overlay above) is the home for connection state — the
        // permanent canvas hint bar is retired. The class is set on the strip
        // element and consumed via CSS.
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
        } else {
          // FR-064: reconnect self-heal. Re-hydrate any open Branches window
          // whose detail check was interrupted by an evict/reconnect so the
          // interrupted notice clears automatically — without a manual Refresh.
          for (const [windowId, state] of branchListStateMap.entries()) {
            if (branchWindowNeedsResync(state)) {
              requestBranches(windowId);
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
      const __testInjectDropPrefixes = new Set();
      window.addEventListener("__gwt_test_inject", (event) => {
        const payload = event && event.detail;
        if (!payload || typeof payload.kind !== "string") {
          return;
        }
        if (payload.kind.startsWith("update_")) {
          __testInjectDropPrefixes.add("update_");
        }
        if (payload.kind.startsWith("issue_monitor_")) {
          __testInjectDropPrefixes.add("issue_monitor_");
        }
        payload.__injected = true;
        try {
          receive(payload);
        } catch (err) {
          console.warn("[gwt_test_inject] receive failed", err);
        }
      });
      function shouldDropLiveEventForTestMode(event) {
        if (typeof __testInjectDropPrefixes === "undefined" || __testInjectDropPrefixes.size === 0) {
          return false;
        }
        if (!event || typeof event.kind !== "string") return false;
        if (event.__injected) return false;
        for (const prefix of __testInjectDropPrefixes) {
          if (event.kind.startsWith(prefix)) return true;
        }
        return false;
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

      function projectWindowContextById(windowId) {
        for (const tab of appState?.tabs || []) {
          for (const windowData of tab.workspace?.windows || []) {
            if (windowData.id === windowId) {
              return { tab, windowData };
            }
          }
        }
        return null;
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
        if (isAgentKanbanPlacement(windowData)) {
          return false;
        }
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

      function agentKanbanDropTargetAt(event, sourceId) {
        const sourceWindow = workspaceWindowById(sourceId);
        if (!isAgentKanbanEligible(sourceWindow)) {
          return null;
        }
        return findAgentKanbanDropTargetAtPoint(
          document,
          event.clientX,
          event.clientY,
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

      // SPEC-3064 Phase 3 (E7): sendOpenProjectDialog, the clone-project
      // modal glue (open/close/render + state), and
      // updateActionAvailability moved to /project-shell-surface.js.

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
          agent_kanban: "Agent Kanban",
          issue: "Issue",
          issue_monitor: "Issue Monitor",
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
        agy: "Antigravity CLI",
        antigravity: "Antigravity CLI",
        "antigravity-cli": "Antigravity CLI",
        "antigravity cli": "Antigravity CLI",
        antigravity_cli: "Antigravity CLI",
        gemini: "Gemini CLI (legacy)",
        "gemini-cli": "Gemini CLI (legacy)",
        "gemini cli": "Gemini CLI (legacy)",
        "gemini cli legacy": "Gemini CLI (legacy)",
        "gemini-cli-legacy": "Gemini CLI (legacy)",
        gemini_cli: "Gemini CLI (legacy)",
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

      // FR-045 (anshin): a glanceable "what is this agent doing now" label.
      // Pairs the display title with the live activity detail
      // (dynamic_title_detail) so glanceable surfaces (Fleet Minimap cells,
      // switcher rows) read like "title · detail". Collapses to just the
      // title when there is no distinct detail.
      function windowActivityLabel(windowData) {
        const title = windowDisplayTitle(windowData);
        const detail = String(windowData?.dynamic_title_detail || "").trim();
        return detail && detail !== title ? `${title} · ${detail}` : title;
      }

      // SPEC-3064 Phase 3 (E7): escapeHtml moved to
      // /project-shell-surface.js with the Window List renderer (its only
      // remaining caller).

      function runtimeStateForWindow(windowData) {
        const cachedState = windowRuntimeStateMap.get(windowData.id);
        if (cachedState) {
          return cachedState;
        }
        return normalizeWindowRuntimeState(windowData.status, windowData.preset);
      }

      // SPEC-3064 Phase 3 (E7): the Window List dropdown
      // (requestWindowList / renderWindowList / toggleWindowList) and the
      // maximized-window viewport sync (geometryMatches /
      // syncMaximizedWindowsToViewport /
      // scheduleMaximizedWindowsToViewportSync /
      // workspaceHasVisibleMaximizedWindow) moved to
      // /project-shell-surface.js.

      // SPEC-3064 Phase 3 (E7): renderProjectTabs, the close-project-tab
      // confirm modal (state + render + requestCloseProjectTab), the project
      // tab cues, the Recent Projects renderers, the Open Project
      // split-button menu, renderProjectPicker, and renderProjectOnboarding
      // moved to /project-shell-surface.js; renderAppState below calls the
      // imported renderers.

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

      function openWorkspaceOverview() {
        focusOrSpawnPreset("work");
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
        // SPEC-2008 camera-focus / FR-094: every viewport change re-centres the
        // Fleet Minimap radar — the camera stays fixed at the minimap centre and
        // the world (cells) slides under it, mirroring the canvas stage.
        fleetMinimap?.update();
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

      function reserveLocalViewportTarget(target) {
        viewportSyncState.applyLocalViewport(target, {
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
        const clampedZoom = clampRange(nextZoom, VIEWPORT_ZOOM_MIN, VIEWPORT_ZOOM_MAX);
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

      // SPEC-2008 camera-focus: manual Ctrl+wheel / button zoom keeps the
      // [VIEWPORT_ZOOM_MIN, VIEWPORT_ZOOM_MAX] envelope (zoomCanvasAt). The low
      // end matches the overview floor so the user can manually pull the canvas
      // far out and see a crowded fleet small (zoom in / double-click to read).
      // Single-window framing is clamped TIGHTER on the high end (see
      // FRAME_ZOOM_MAX) so the stage scale never exceeds 1 and xterm is never
      // CSS-upscaled (which has no de-blur path).
      const VIEWPORT_ZOOM_MIN = 0.15;
      const VIEWPORT_ZOOM_MAX = 2.4;
      // Single-window framing must never magnify past 1:1. At zoom ≤ 1 the
      // terminal canvas renders at (or below) its native pixel grid, so framed
      // agent terminals stay crisp. A window smaller than the work area is
      // centered at 1:1 rather than blown up to fill (crisp > edge-fill).
      const FRAME_ZOOM_MAX = 1.0;
      // Overview (fitAll / enterOverview) must be able to pull the camera much
      // further out so a widely-scattered fleet still fits — the single-window
      // floor of 0.6 would clip it. Cells stay legible in the always-on Fleet
      // Minimap, and the soft canvas at low zoom is acceptable for overview.
      const OVERVIEW_ZOOM_MIN = 0.15;
      // Fraction of the work area a framed rect should occupy along its tighter
      // axis, leaving a small margin so the window is not flush to the edges.
      // A terminal that roughly fills the work area lands near zoom ≈ 1, which
      // keeps xterm crisp (no CSS scaling beyond the stage transform).
      const FRAME_FILL_RATIO = 0.92;

      // Commit a target viewport through the same local-edit + persist-throttle
      // path that pan/zoom use, so the framed camera is saved per viewer and
      // never broadcast to other clients (FR-095).
      function commitFramedViewport(target) {
        if (sameViewportValues(viewport, target)) {
          applyViewport();
          return;
        }
        viewport = target;
        recordLocalViewportEdit();
        applyViewport();
        persistViewport();
      }

      function canvasScreenRect() {
        const rect =
          typeof canvas.getBoundingClientRect === "function"
            ? canvas.getBoundingClientRect()
            : null;
        const left = Number.isFinite(rect?.left) ? rect.left : 0;
        const top = Number.isFinite(rect?.top) ? rect.top : 0;
        const width = Math.max(Number(rect?.width) || canvas.clientWidth || 0, 1);
        const height = Math.max(Number(rect?.height) || canvas.clientHeight || 0, 1);
        return {
          left,
          top,
          right: left + width,
          bottom: top + height,
          width,
          height,
        };
      }

      function cameraFrameArea() {
        const obstructionRects = [];
        const railRect = document
          .getElementById("op-rail")
          ?.getBoundingClientRect?.();
        if (railRect) {
          obstructionRects.push(railRect);
        }
        return computeCameraFrameArea({
          canvasRect: canvasScreenRect(),
          obstructionRects,
        });
      }

      function shouldAnimateWindowFrame() {
        const frame = cameraFrameArea();
        return frame.width >= 560 && frame.height >= 420;
      }

      function clampWindowElementToCameraFrame(windowId) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const rect = element.getBoundingClientRect();
        const canvasRect = canvas.getBoundingClientRect();
        const frame = cameraFrameArea();
        const frameLeft = canvasRect.left + frame.left;
        const frameTop = canvasRect.top + frame.top;
        const frameRight = frameLeft + frame.width;
        const frameBottom = frameTop + frame.height;
        let deltaX = 0;
        let deltaY = 0;
        if (rect.left < frameLeft) {
          deltaX = frameLeft - rect.left;
        } else if (rect.right > frameRight) {
          deltaX = frameRight - rect.right;
        }
        if (rect.top < frameTop) {
          deltaY = frameTop - rect.top;
        } else if (rect.bottom > frameBottom) {
          deltaY = frameBottom - rect.bottom;
        }
        if (Math.abs(deltaX) < 0.5 && Math.abs(deltaY) < 0.5) {
          return;
        }
        viewport = {
          x: viewport.x + deltaX,
          y: viewport.y + deltaY,
          zoom: viewport.zoom,
        };
        recordLocalViewportEdit();
        applyViewport();
        persistViewport();
      }

      let pendingWindowFrameClampTimer = null;
      let pendingWindowFrameClampFrame = null;
      let windowFrameClampToken = 0;

      function scheduleWindowFrameClamp(windowId, { animate = false } = {}) {
        windowFrameClampToken += 1;
        const clampToken = windowFrameClampToken;
        if (pendingWindowFrameClampTimer !== null) {
          clearTimeout(pendingWindowFrameClampTimer);
          pendingWindowFrameClampTimer = null;
        }
        if (
          pendingWindowFrameClampFrame !== null &&
          typeof cancelAnimationFrame === "function"
        ) {
          cancelAnimationFrame(pendingWindowFrameClampFrame);
          pendingWindowFrameClampFrame = null;
        }
        const run = () => {
          pendingWindowFrameClampTimer = null;
          if (clampToken !== windowFrameClampToken) {
            return;
          }
          if (typeof requestAnimationFrame === "function") {
            pendingWindowFrameClampFrame = requestAnimationFrame(() => {
              pendingWindowFrameClampFrame = null;
              if (clampToken !== windowFrameClampToken) {
                return;
              }
              clampWindowElementToCameraFrame(windowId);
            });
            return;
          }
          clampWindowElementToCameraFrame(windowId);
        };
        if (animate) {
          pendingWindowFrameClampTimer = setTimeout(run, 240);
          return;
        }
        run();
      }

      // Compute the viewport that fits a WORLD-space rect into the usable
      // visual camera area, centered, with a small fill margin. screen =
      // world*zoom + offset, so to center the rect's center on the frame center
      // we solve offset = frameCenter - worldCenter*zoom.
      function viewportForWorldRect(
        rect,
        { minZoom = VIEWPORT_ZOOM_MIN, maxZoom = VIEWPORT_ZOOM_MAX } = {},
      ) {
        return computeViewportForWorldRect(rect, {
          frameArea: cameraFrameArea(),
          fillRatio: FRAME_FILL_RATIO,
          minZoom,
          maxZoom,
        });
      }

      function framingRectForWindow(windowData) {
        const geometry = windowData?.geometry || {};
        const element = windowMap.get(windowData?.id);
        return expandWorldRectForLayoutSize(geometry, {
          width: element?.offsetWidth,
          height: element?.offsetHeight,
        });
      }

      // Fly the camera so `windowId`'s world geometry fills the work area. The
      // window stays fixed in world space; only the viewport moves (camera
      // model). Also applies local focus + z-order highlight (focus_window
      // stays for highlight; maximize_window is gone).
      function frameWindow(windowId, { animate = true, notifyFocus = true } = {}) {
        const windowData = workspaceWindowById(windowId);
        if (!windowData) {
          return;
        }
        // Cap framing at 1:1 so the focused terminal never CSS-upscales (blur).
        const target = viewportForWorldRect(framingRectForWindow(windowData), {
          maxZoom: FRAME_ZOOM_MAX,
        });
        reserveLocalViewportTarget(target);
        focusWindowLocally(windowId);
        if (notifyFocus) {
          send({ kind: "focus_window", id: windowId });
        }
        animateViewportTo(target, { animate });
        scheduleWindowFrameClamp(windowId, { animate });
      }

      let focusedWindowViewportReframeFrame = null;

      function frameFocusedWindowAfterViewportResize() {
        if (!focusedId) {
          return;
        }
        const windowData = workspaceWindowById(focusedId);
        if (!windowData?.geometry || !visibleWindowData(windowData)) {
          return;
        }
        frameWindow(focusedId, { animate: false, notifyFocus: false });
      }

      function scheduleFocusedWindowViewportReframe() {
        if (focusedWindowViewportReframeFrame !== null) {
          cancelAnimationFrame(focusedWindowViewportReframeFrame);
          focusedWindowViewportReframeFrame = null;
        }
        if (typeof requestAnimationFrame !== "function") {
          frameFocusedWindowAfterViewportResize();
          return;
        }
        focusedWindowViewportReframeFrame = requestAnimationFrame(() => {
          focusedWindowViewportReframeFrame = null;
          frameFocusedWindowAfterViewportResize();
        });
      }

      window.addEventListener("resize", scheduleFocusedWindowViewportReframe);

      // Frame the bounding box of every canvas window so all windows are in
      // view (overview / fit-all). Falls back to a gentle zoom-to-fit-nothing
      // when there are no framable windows.
      function fitAll({ animate = true } = {}) {
        const windows = (activeWorkspace().windows || []).filter(
          (windowData) =>
            visibleWindowData(windowData) && windowData.geometry,
        );
        if (windows.length === 0) {
          return;
        }
        let minX = Infinity;
        let minY = Infinity;
        let maxX = -Infinity;
        let maxY = -Infinity;
        for (const windowData of windows) {
          const g = windowData.geometry;
          const x = Number(g.x) || 0;
          const y = Number(g.y) || 0;
          const w = Math.max(Number(g.width) || 0, 0);
          const h = Math.max(Number(g.height) || 0, 0);
          minX = Math.min(minX, x);
          minY = Math.min(minY, y);
          maxX = Math.max(maxX, x + w);
          maxY = Math.max(maxY, y + h);
        }
        const target = viewportForWorldRect(
          {
            x: minX,
            y: minY,
            width: maxX - minX,
            height: maxY - minY,
          },
          // Overview is allowed to zoom out far below the single-window floor so
          // a scattered fleet still fits, and is capped at 1:1 on the high end
          // (a lone small window in overview is centered crisp, never blown up).
          { minZoom: OVERVIEW_ZOOM_MIN, maxZoom: FRAME_ZOOM_MAX },
        );
        animateViewportTo(target, { animate });
      }

      // Esc when framed zooms the camera out to frame all windows.
      function enterOverview({ animate = true } = {}) {
        fitAll({ animate });
      }

      // SPEC-2008 camera-focus: Esc-to-overview must not steal a bare Esc from
      // a focused terminal (vim / TUI apps) or any text entry. xterm's input
      // sink is the `.xterm-helper-textarea`; inputs / textareas /
      // contenteditable are treated the same.
      function isTextEntryFocused() {
        const active = document.activeElement;
        if (!active) {
          return false;
        }
        if (active.classList && active.classList.contains("xterm-helper-textarea")) {
          return true;
        }
        if (active.closest && active.closest(".terminal-root")) {
          return true;
        }
        const tag = active.tagName;
        if (tag === "INPUT" || tag === "TEXTAREA") {
          return true;
        }
        return Boolean(active.isContentEditable);
      }

      let viewportTweenFrame = null;

      // Short rAF tween of the viewport {x,y,zoom}. Correctness over animation:
      // an unavailable rAF (or animate=false) commits the target instantly.
      function animateViewportTo(target, { animate = true } = {}) {
        if (viewportTweenFrame !== null) {
          cancelAnimationFrame(viewportTweenFrame);
          viewportTweenFrame = null;
        }
        if (
          !animate ||
          typeof requestAnimationFrame !== "function" ||
          sameViewportValues(viewport, target)
        ) {
          commitFramedViewport(target);
          return;
        }
        const start = { x: viewport.x, y: viewport.y, zoom: viewport.zoom };
        const durationMs = 180;
        const startedAt =
          typeof performance !== "undefined" && performance.now
            ? performance.now()
            : Date.now();
        const easeOut = (t) => 1 - Math.pow(1 - t, 3);
        const step = () => {
          const nowMs =
            typeof performance !== "undefined" && performance.now
              ? performance.now()
              : Date.now();
          const progress = Math.min(1, (nowMs - startedAt) / durationMs);
          const eased = easeOut(progress);
          viewport = {
            x: start.x + (target.x - start.x) * eased,
            y: start.y + (target.y - start.y) * eased,
            zoom: start.zoom + (target.zoom - start.zoom) * eased,
          };
          recordLocalViewportEdit();
          applyViewport();
          if (progress < 1) {
            viewportTweenFrame = requestAnimationFrame(step);
            return;
          }
          viewportTweenFrame = null;
          // Land exactly on target and persist the final framed camera once.
          viewport = { x: target.x, y: target.y, zoom: target.zoom };
          recordLocalViewportEdit();
          applyViewport();
          persistViewport();
        };
        viewportTweenFrame = requestAnimationFrame(step);
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
        // SPEC-2008 camera-focus: cycling flies the local camera between
        // windows in creation order (per viewer) instead of asking the backend
        // to move a shared focus. Keep notifying the backend of the new focus
        // for z-order/highlight, but the camera move is local.
        const windows = (activeWorkspace().windows || []).filter(visibleWindowData);
        if (windows.length === 0) {
          return;
        }
        const currentIndex = windows.findIndex(
          (windowData) => windowData.id === focusedId,
        );
        const delta = direction === "backward" ? -1 : 1;
        const baseIndex = currentIndex === -1 ? 0 : currentIndex;
        const nextIndex =
          (baseIndex + delta + windows.length) % windows.length;
        frameWindow(windows[nextIndex].id);
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
        const workspaceWindow = workspaceWindowById(windowId);
        if (isAgentKanbanPlacement(workspaceWindow)) {
          const terminalHost = terminalMap.get(windowId)?.terminal?.element?.parentElement;
          return elementHasLayoutBox(terminalHost);
        }
        return viewportEligibleForRefresh({
          element: windowMap.get(windowId),
          workspaceWindow,
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

      function hasPendingTerminalViewportRefresh(windowId) {
        const runtime = terminalMap.get(windowId);
        return (
          runtime?.viewportRefreshPending === true ||
          terminalViewportRefreshScheduler?.hasPending?.(windowId) === true
        );
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
          syncGeometryOnGridChange: true,
          sendGeometry,
        });
        if (!activation.ran) {
          runtime.viewportRefreshPending = true;
          scheduleTerminalFocusActivation(windowId, {
            shouldPersistGeometry,
            reason: "force_refresh_retry",
          });
          return false;
        }
        runtime.viewportRefreshPending = false;
        refreshTerminalViewport(windowId);
        return true;
      }

      function rearmPendingTerminalViewportRefresh(
        windowId,
        { shouldPersistGeometry = true } = {},
      ) {
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
            forceTerminalViewportRefresh(windowId, { shouldPersistGeometry });
          },
        });
      }

      function traceTerminalActivation(windowId, activation, fields = {}) {
        traceUi(UI_TRACE_EVENT.terminalActivation, {
          window_id: windowId,
          ran: activation?.ran === true,
          fast_path: activation?.fastPath === true,
          grid_changed: activation?.gridChanged === true,
          geometry_sent: activation?.geometrySent === true,
          cols: activation?.cols ?? 0,
          rows: activation?.rows ?? 0,
          reason: activation?.reason || "unknown",
          ...fields,
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
        { shouldPersistGeometry = true, reason = "focus_activation" } = {},
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
          const pendingOutputCount = terminalOutputBatcher.pendingCount(windowId);
          const hasPendingRefresh = hasPendingTerminalViewportRefresh(windowId);
          const activation = runTerminalActivationSequence({
            runtime: activeRuntime,
            windowId,
            shouldFocus,
            shouldPersistGeometry,
            syncGeometryOnGridChange: true,
            allowFastPath: true,
            pendingOutputCount,
            hasPendingRefresh,
            sendGeometry,
          });
          traceTerminalActivation(windowId, activation, {
            activation_reason: reason,
            pending_output_count: pendingOutputCount,
            pending_refresh: hasPendingRefresh,
            should_focus: shouldFocus,
            should_persist_geometry: shouldPersistGeometry,
          });
          if (!activation.ran) {
            activeRuntime.activationAttempts =
              (activeRuntime.activationAttempts || 0) + 1;
            if (activeRuntime.activationAttempts <= HANDSHAKE_RETRY_LIMIT) {
              scheduleTerminalFocusActivation(windowId, {
                shouldPersistGeometry,
                reason,
              });
            }
            return;
          }
          activeRuntime.activationAttempts = 0;
          if (activation.fastPath) {
            return;
          }
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
        const windowData = workspaceWindowById(windowId);
        if (isAgentKanbanPlacement(windowData)) {
          send(updateTerminalGridMessage(windowId, cols, rows));
          return;
        }
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        send({
          kind: "update_window_geometry",
          id: windowId,
          geometry: {
            x: parseNumber(element.style.left),
            y: parseNumber(element.style.top),
            // SPEC-2008 camera-focus: windows are never minimized, so the
            // element size is always the live geometry.
            width: parseNumber(element.style.width),
            height: parseNumber(element.style.height),
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
        appendRenderKeyPart(parts, "running");
        appendRenderKeyPart(parts, counts?.running ?? null);
        appendRenderKeyPart(parts, "idle");
        appendRenderKeyPart(parts, counts?.idle ?? null);
        // FR-039 (anshin): the WAITING cell refreshes when the waiting count
        // changes; include it in the render key so the strip is not skipped.
        appendRenderKeyPart(parts, "waiting");
        appendRenderKeyPart(parts, counts?.waiting ?? null);
        appendRenderKeyPart(parts, "error");
        appendRenderKeyPart(parts, counts?.error ?? null);
        appendRenderKeyPart(parts, "done");
        appendRenderKeyPart(parts, counts?.done ?? null);
        appendRenderKeyPart(parts, "agents");
        appendRenderKeyPart(parts, counts?.agents ?? null);
        // SPEC-3038 (2026-06-20): include the window count so the rail Windows
        // badge refreshes when only surface (non-agent) windows are added or
        // removed. Agent-count fields alone do not change for surfaces, which
        // otherwise short-circuits the badge update via the shared cache key.
        appendRenderKeyPart(parts, "windows");
        appendRenderKeyPart(parts, counts?.windows ?? null);
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

      // SPEC-3038 AS-4.5: the empty-canvas call to action follows the live
      // window count, even when the operator shell is degraded.
      function updateCanvasEmptyState() {
        const emptyState = document.getElementById("canvas-empty-state");
        if (emptyState) {
          emptyState.hidden = windowMap.size > 0;
        }
      }

      function recomputeOperatorTelemetry() {
        updateCanvasEmptyState();
        // SPEC-2008 camera-focus / FR-094: rebuild the Fleet Minimap cells from
        // the live window set (geometry / agent color / telemetry). Runs on the
        // same cadence as the canvas empty-state + rail badges, before the
        // operator-shell-degraded early return so the minimap stays live.
        fleetMinimap?.renderCells();
        if (!window.__operatorShell?.applyTelemetryCounts) return;
        // SPEC-3038 AS-1.4 (2026-06-20): the rail Windows item badges the
        // open-window count across all project tabs, matching the cross-tab
        // Windows popover. windowMap only holds windows mounted in visited
        // tabs, so it undercounts; allProjectWindowIds() is the true total.
        const counts = {
          running: 0,
          idle: 0,
          // FR-039 (anshin): waiting is its own LOUD telemetry state for
          // agents waiting on the operator. It used to collapse into idle;
          // now it drives the WAITING strip cell instead.
          waiting: 0,
          error: 0,
          done: 0,
          agents: 0,
          windows: allProjectWindowIds().length,
        };
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
            counts.running = Math.max(counts.running, activeAgents || activeWorks.length);
            counts.agents = Math.max(counts.agents, totalAgents || activeAgents + blockedAgents);
            counts.branches = Math.max(Number(counts.branches || 0), activeWorks.length);
          } else {
            const category = activeWorkProjection.status_category || "unknown";
            const activeAgents = Number(activeWorkProjection.active_agents || 0);
            const blockedAgents = Number(activeWorkProjection.blocked_agents || 0);
            if (category === "active") counts.running = Math.max(counts.running, activeAgents || 1);
            if (category === "idle" && activeAgents > 0) counts.idle = Math.max(counts.idle, activeAgents);
            if (category === "done") counts.done = Math.max(counts.done, 1);
            counts.agents = Math.max(counts.agents, activeAgents + blockedAgents);
          }
        }
        applyOperatorTelemetryCounts(counts);
      }

      // ---- Provider usage & rate limits (SPEC-2970) ----
      // SPEC-3064 Phase 3 (E1): the usage snapshot state, formatters, hover
      // popover, and Settings usage panel moved to
      // /provider-usage-surface.js. app.js keeps the receive() and Settings
      // call sites and wires them through this factory.
      const { applyProviderUsageUi, renderUsagePanel } = createProviderUsageSurface({
        send,
        renderWorkspaceWindows: () => workspaceOverviewSurface.renderWindows(),
      });

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

      function applyStatus(windowId, status, detail) {
        return traceMeasure(
          UI_TRACE_EVENT.applyStatus,
          { window_id: windowId, status },
          () => {
            const windowContext = projectWindowContextById(windowId);
            const windowData =
              windowContext?.windowData || workspaceWindowById(windowId);
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
            agentCompletionNotifier.handleRuntimeState({
              windowId,
              runtimeState,
              windowData,
              projectTab: windowContext?.tab || activeProjectTab(),
              statusDetail: effectiveDetail,
            });
            // SPEC-2356 Anshin Addendum (FR-040): only agent panes (the presets
            // that carry a waiting state) raise in-app attention toasts.
            if (windowData && presetSupportsWaitingStatus(windowData.preset)) {
              agentAttentionToaster.handleRuntimeState({
                windowId,
                runtimeState,
                windowData,
                statusDetail: effectiveDetail,
              });
            }
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
              refreshWindowTabTelemetry(windowData);
              refreshProjectTabStateCues();
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
            // SPEC-2356 Anshin Addendum (FR-041/FR-044): toggle the kill-switch
            // chrome. STOP shows for a live agent runtime; RESTART replaces it
            // once the runtime is stopped or errored. Non-agent windows never
            // expose either control.
            updateWindowKillSwitchControls(element, windowData, runtimeState);
            recomputeOperatorTelemetry();
            refreshWindowTabTelemetry(windowData);
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
            refreshProjectTabStateCues();
          },
        );
      }

      // SPEC-2356 Anshin Addendum (FR-041/FR-044): a stopped/errored agent runtime
      // shows RESTART; a live one shows STOP. Non-agent windows hide both.
      const STOPPED_RUNTIME_STATES = new Set(["stopped", "exited", "error"]);

      function updateWindowKillSwitchControls(element, windowData, runtimeState) {
        const stopButton = element.querySelector("[data-action='stop']");
        const restartButton = element.querySelector("[data-action='restart']");
        if (!stopButton || !restartButton) {
          return;
        }
        const isAgentWindow = shouldShowRuntimeStatus(windowData);
        if (!isAgentWindow) {
          stopButton.hidden = true;
          restartButton.hidden = true;
          return;
        }
        const isStopped = STOPPED_RUNTIME_STATES.has(runtimeState);
        stopButton.hidden = isStopped;
        restartButton.hidden = !isStopped;
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
          raiseWindowElementLocally(targetElement);
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
          raiseWindowElementLocally(targetElement);
        }
      }

      function numericZIndex(value) {
        const numeric = Number.parseInt(value || "0", 10);
        return Number.isFinite(numeric) ? numeric : 0;
      }

      function raiseWindowElementLocally(targetElement) {
        if (!targetElement) {
          return;
        }
        const targetZ = numericZIndex(targetElement.style.zIndex);
        let maxPeerZ = 0;
        for (const element of windowMap.values()) {
          if (element === targetElement) {
            continue;
          }
          maxPeerZ = Math.max(maxPeerZ, numericZIndex(element.style.zIndex));
        }
        if (targetZ <= maxPeerZ) {
          targetElement.style.zIndex = String(maxPeerZ + 1);
        }
      }

      function focusWindowRemotely(windowId, { center = false } = {}) {
        focusWindowLocally(windowId);
        const payload = { kind: "focus_window", id: windowId };
        if (center) payload.bounds = visibleBounds();
        send(payload);
      }

      // SPEC-2008 camera-focus: toggleMinimizeWindow / toggleMaximizeWindow
      // were removed. Maximize/minimize/restore no longer exist; focusing a
      // window flies the camera to frame it in place (see frameWindow).

      // SPEC-3038 US-3: Close Guard — every window close (titlebar x and
      // tab x) confirms through one modal regardless of agent state
      // (user-confirmed decision, 2026-06-10).
      let windowCloseConfirmState = { open: false, windowId: null };

      function renderWindowCloseConfirm() {
        const modalEl = document.getElementById("window-close-confirm-modal");
        const dialogEl = modalEl?.querySelector(".window-close-confirm-shell");
        if (!modalEl || !dialogEl) return;
        renderWindowCloseConfirmModal({
          modalEl,
          dialogEl,
          state: windowCloseConfirmState,
          createNode,
          onCancel: () => closeWindowCloseConfirm(),
          onConfirm: () => {
            const id = windowCloseConfirmState.windowId;
            closeWindowCloseConfirm();
            if (id) send({ kind: "close_window", id });
          },
        });
      }

      function closeWindowCloseConfirm() {
        windowCloseConfirmState = { open: false, windowId: null };
        renderWindowCloseConfirm();
      }

      function requestCloseWindow(windowId) {
        const windowData = workspaceWindowById(windowId);
        if (!windowData) {
          // The window already left the workspace state; closing is pure
          // housekeeping and needs no confirmation.
          send({ kind: "close_window", id: windowId });
          return;
        }
        const isAgentWindow = shouldShowRuntimeStatus(windowData);
        const runtimeState =
          windowRuntimeStateMap.get(windowId) ||
          normalizeWindowRuntimeState(windowData.status, windowData.preset);
        windowCloseConfirmState = {
          open: true,
          windowId,
          windowTitle: windowDisplayTitle(windowData),
          agentLabel: isAgentWindow
            ? agentRoleLabel(windowData)
            : presetRoleLabel(windowData.preset),
          runtimeLabel: isAgentWindow ? windowRuntimeLabel(runtimeState) : "",
          running: isAgentWindow && runtimeState === "running",
        };
        renderWindowCloseConfirm();
      }

      // SPEC-2356 Anshin Addendum (FR-042): STOP ALL halts every live agent
      // runtime, leaving windows on the canvas. Count the live agent windows so
      // the confirm is specific, and no-op when nothing is running.
      function countRunningAgentWindows() {
        let count = 0;
        for (const [windowId, element] of windowMap.entries()) {
          if (!element) continue;
          const windowData = workspaceWindowById(windowId);
          if (!windowData || !presetSupportsWaitingStatus(windowData.preset)) continue;
          const runtimeState =
            windowRuntimeStateMap.get(windowId) ||
            normalizeWindowRuntimeState(windowData.status, windowData.preset);
          if (!STOPPED_RUNTIME_STATES.has(runtimeState)) {
            count += 1;
          }
        }
        return count;
      }

      function requestStopAllWindows() {
        const running = countRunningAgentWindows();
        if (running === 0) {
          return;
        }
        const plural = running === 1 ? "agent" : "agents";
        if (
          window.confirm(
            `Stop all ${running} running ${plural}? Their windows and output stay on the canvas; you can restart each one in place.`,
          )
        ) {
          send({ kind: "stop_all_windows" });
        }
      }

      // SPEC-3038 US-2: tabs carry the same Living Telemetry the window chrome
      // shows. Only agent panes (terminal surface) report a state; other
      // surfaces render plain tabs.
      function windowTabTelemetryState(tab) {
        if (!shouldShowRuntimeStatus(tab)) return "";
        const runtimeState =
          windowRuntimeStateMap.get(tab.id) ||
          normalizeWindowRuntimeState(tab.status, tab.preset);
        return runtimeState;
      }

      // AS-2.2: a runtime state change must repaint the tab strip of every
      // window in the group (the visible strip belongs to the active window's
      // element, not necessarily the one whose status changed).
      function refreshWindowTabTelemetry(windowData) {
        if (!windowData?.tab_group_id) return;
        for (const tab of windowTabsFor(windowData)) {
          const tabElement = windowMap.get(tab.id);
          if (tabElement) renderWindowTabs(tab, tabElement);
        }
      }

      function renderWindowTabs(windowData, element) {
        const strip = element.querySelector(".window-tab-strip");
        if (!strip) return;
        renderWindowTabsView({
          strip,
          tabs: windowTabsFor(windowData).map((tab) => ({
            ...tab,
            agent_state: windowTabTelemetryState(tab),
          })),
          activeWindowId: windowData.id,
          tooltipForWindow: windowTitleTooltip,
          send,
          requestClose: requestCloseWindow,
          onTabDragStart: (event, tabId) => {
            windowTabDragState = {
              id: tabId,
              docked: false,
              lastClientPoint: clientPointFromDragEvent(
                event,
                canvas.getBoundingClientRect(),
              ),
            };
            event.dataTransfer?.setData("text/plain", tabId);
            if (event.dataTransfer) {
              event.dataTransfer.effectAllowed = "move";
            }
          },
          onTabDrag: trackWindowTabDragPoint,
          onTabDragEnd: (event) => {
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
          },
        });
      }

      // SPEC-2008 camera-focus (UX fix): focus and camera move are SEPARATE.
      // A single titlebar click only focuses the window (front + highlight +
      // input routing) and never moves the camera/zoom. A DOUBLE click is the
      // deliberate "fly the camera to frame this window" gesture. Focus fires
      // immediately on the first click (no delay); a second click on the same
      // window within the threshold upgrades to framing.
      let lastTitlebarClick = null;
      const TITLEBAR_DOUBLE_CLICK_MS = 300;
      function handleTitlebarClick(windowId) {
        const now = Date.now();
        const isDoubleClick =
          lastTitlebarClick &&
          lastTitlebarClick.id === windowId &&
          now - lastTitlebarClick.at <= TITLEBAR_DOUBLE_CLICK_MS;
        if (isDoubleClick) {
          lastTitlebarClick = null;
          // Deliberate framing: fly the camera to the window.
          frameWindow(windowId);
          return;
        }
        lastTitlebarClick = { id: windowId, at: now };
        // Single click = focus only, camera unchanged (no center → no bounds).
        focusWindowRemotely(windowId);
      }

      function decodeBase64(base64) {
        return Uint8Array.from(atob(base64), (value) => value.charCodeAt(0));
      }

      function isBlinkBrowser() {
        const ua = navigator.userAgent || "";
        return /Chrome\//.test(ua);
      }

      // SPEC-3064 Phase 3 (E2): terminal attachment, clipboard, and file-drop
      // handling moved to /terminal-attachments.js. createTerminalRuntime,
      // applyStatus, mountWindowBody, receive(), and the boot-time bridge
      // installs keep their existing call sites wired through this factory.
      const {
        updateTerminalOverlayCopyState,
        copyTerminalOverlayMessage,
        installTerminalCopyHandlers,
        installTerminalImagePasteHandlers,
        installTerminalFileDropHandlers,
        installTerminalContextMenuHandlers,
        installTerminalViewportRefreshHandlers,
        handleAttachmentProgress,
        installBrowserFileDropBridge,
        installNativeFileDropBridge,
      } = createTerminalAttachments({
        send,
        terminalMap,
        windowMap,
        workspaceWindowById,
        isAgentWindowPreset,
        workspaceWindowElement,
        scheduleTerminalViewportRefresh,
        TERMINAL_SELECTION_DRAG_THRESHOLD,
      });

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

      function attachTerminalContainerBindings(windowId, terminalContainer, terminal) {
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
        const containerResizeCleanup = attachContainerResizeReflow({
          element: terminalContainer,
          windowId,
          fitTerminal,
          shouldSkip: () => !!resizeState && resizeState.id === windowId,
        });
        return () => {
          copyCleanup();
          imagePasteCleanup();
          fileDropCleanup();
          contextMenuCleanup();
          wheelScrollCleanup.dispose();
          containerResizeCleanup();
        };
      }

      function reparentTerminalRuntime(windowId, runtime, terminalContainer) {
        if (!runtime || runtime.terminalContainer === terminalContainer) {
          return runtime;
        }
        const terminalElement = runtime.terminal?.element;
        if (terminalElement && terminalElement.parentElement !== terminalContainer) {
          terminalContainer.appendChild(terminalElement);
        }
        runtime.containerBindingsCleanup?.();
        runtime.terminalContainer = terminalContainer;
        runtime.containerBindingsCleanup = attachTerminalContainerBindings(
          windowId,
          terminalContainer,
          runtime.terminal,
        );
        requestAnimationFrame(() => {
          scheduleTerminalFit(windowId, true);
        });
        return runtime;
      }

      function createTerminalRuntime(windowId, terminalContainer) {
        if (terminalMap.has(windowId)) {
          return reparentTerminalRuntime(
            windowId,
            terminalMap.get(windowId),
            terminalContainer,
          );
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
        const viewportRefreshCleanup = installTerminalViewportRefreshHandlers(windowId, terminal);
        const containerBindingsCleanup = attachTerminalContainerBindings(
          windowId,
          terminalContainer,
          terminal,
        );
        const cleanup = () => {
          terminalMap.get(windowId)?.containerBindingsCleanup?.();
          viewportRefreshCleanup();
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
          terminalContainer,
          containerBindingsCleanup,
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
            if (!canRefreshTerminalViewport(windowId)) {
              markTerminalViewportRefreshPending(windowId);
            }
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

      // SPEC-3064 Phase 3 (E6a): the File Tree window surface (file tree
      // state map, worktree picker, text/hex viewer + dirty tracking,
      // highlight.js lazy load, renderFileTree / renderFileTreeViewer, the
      // File Tree window mount, and the file_tree_* / file_content_*
      // receive bodies) moved to /file-tree-surface.js. app.js keeps the
      // receive() case arms, the frontendUnits registry entries, and the
      // window-cleanup call sites, wired through this factory.
      const {
        fileTreeStateMap,
        ensureFileTreeState,
        requestFileTree,
        renderFileTree,
        renderFileTreeViewer,
        openWorktreePicker,
        closeWorktreePicker,
        renderWorktreePicker,
        requestFileContent,
        requestSaveFileContent,
        renderDiscardModal,
        renderConflictModal,
        queueNavigationGuardedByDirty,
        beginViewerForFile,
        applyAfterSaveContinuation,
        mountFileTreeWindow,
        applyFileTreeReceiveEvent,
      } = createFileTreeSurface({
        send,
        makeEl,
        clearChildren,
        focusWindowLocally,
        sendWindowFocus: (id) => socketTransport.send({ kind: "focus_window", id }),
        windowMap,
      });

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

      // SPEC-3064 Phase 3 (E6e): the Profile window surface (profile state
      // map, draft editing + debounced save, Profile window mount, and the
      // profile_* receive bodies) moved to /profile-window-surface.js.
      // app.js keeps the receive() case arms, the frontendUnits registry
      // entries, and the window-cleanup call sites, wired through this
      // factory.
      const {
        profileStateMap,
        ensureProfileState,
        requestProfile,
        renderProfile,
        createProfile,
        setActiveProfile,
        flushProfileSave,
        deleteProfile,
        updateProfileStatus,
        profileHasEditableFocus,
        syncProfileDraftFromSelection,
        clearProfileSaveTimer,
        mountProfileWindow,
        applyProfileReceiveEvent,
      } = createProfileWindowSurface({
        send,
        createNode,
        windowMap,
        focusWindowLocally,
        sendWindowFocus: (id) => socketTransport.send({ kind: "focus_window", id }),
      });

      // SPEC-2359 W-17 (FR-398): shared pending state for Resume/Launch
      // requests. Settled by the dispatcher on workspace_resume_agent_started
      // / *_error; the timeout re-enables the UI when no reply ever arrives.
      const launchPending = createLaunchPendingController({
        onChange: () => {
          try {
            workspaceOverviewSurface.renderWindows();
          } catch {
            // Surface may not be mounted yet during bootstrap.
          }
          try {
            workspaceResumePicker.render();
          } catch {
            // Picker may not be mounted yet during bootstrap.
          }
          const notice = launchPending.consumeTimeoutNotice();
          if (notice) {
            console.warn("[launch-pending]", notice);
          }
        },
      });

      // SPEC-3064 Phase 3 (E6d): the Knowledge Bridge (Kanban) window
      // surface (knowledge bridge state map, semantic search coalescing,
      // Kanban rendering, Kanban Drawer, Knowledge window mount, and the
      // knowledge_* receive bodies) moved to /knowledge-kanban-surface.js.
      // app.js keeps the receive() case arms, the frontendUnits registry
      // entries, the drawer chrome wiring, and the window-cleanup call
      // sites, wired through this factory.
      const {
        ensureKnowledgeBridgeState,
        clearKnowledgeBridgeState,
        requestKnowledgeBridge,
        scheduleKnowledgeRelatedWorkRefresh,
        scheduleKnowledgeSearch,
        requestKnowledgeDetail,
        knowledgeDetailRequestMatches,
        renderKnowledgeBridge,
        writeKanbanHideDonePreference,
        closeKanbanDrawer,
        mountKnowledgeWindow,
        applyKnowledgeReceiveEvent,
      } = createKnowledgeKanbanSurface({
        send,
        createNode,
        createKnowledgeMarkdownBody,
        windowMap,
        workspaceWindowById,
        getWorkspaceWindows: () =>
          allProjectWindowIds()
            .map((windowId) => workspaceWindowById(windowId))
            .filter(Boolean),
        pendingIndexOpenTargetsByPreset,
        knowledgeKindForPreset,
        focusWindowLocally,
        sendWindowFocus: (id) => socketTransport.send({ kind: "focus_window", id }),
        focusOrSpawnPreset,
        openIssueLaunchWizard,
        visibleBounds,
        launchPending,
      });

      // SPEC-3064 Phase 3 (E6c): the Board & Logs window surface (board/log
      // state maps, Work-id tracking for the Board Work filter, board chat
      // + logs rendering, both window mounts, and the board_* / log_*
      // receive bodies) moved to /board-logs-surface.js. app.js keeps the
      // receive() case arms, the frontendUnits registry entries, the
      // renderAppState / active_work_projection sync call sites, and the
      // window-cleanup call sites, wired through this factory.
      const {
        boardStateMap,
        logStateMap,
        ensureBoardState,
        ensureLogState,
        requestBoard,
        requestOlderBoardEntries,
        requestLogs,
        renderBoard,
        renderLogs,
        submitBoardEntry,
        focusBoardEntry,
        handleBoardHookEvent,
        appendLiveLogEntry,
        jumpToUnreadLogs,
        cacheActiveWorkProjectionWorkspaceIds,
        deriveCurrentProjectWorkspaceIds,
        syncCurrentProjectWorkspaceIds,
        refreshBoardCurrentWorkspaceId,
        mountBoardWindow,
        mountLogsWindow,
        applyBoardLogsReceiveEvent,
        applyProjectBoardConfigEventToBoard,
      } = createBoardLogsSurface({
        send,
        createNode,
        createKnowledgeMarkdownBody,
        windowMap,
        focusWindowLocally,
        sendWindowFocus: (id) => socketTransport.send({ kind: "focus_window", id }),
        focusOrSpawnPreset,
        activeWorkspace,
        activeProjectTab,
        visibleBounds,
        getActiveWorkProjection: () => activeWorkProjection,
      });

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

      // SPEC-2359 US-80: debounced Start Work duplicate-work advisory query.
      function requestWorkAdvisory({ id, query, request_id }) {
        send({
          kind: "request_work_advisory",
          id,
          query,
          request_id,
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

      function showAgentCompletionToast(notice) {
        const existing = document.getElementById("agent-completion-toast");
        existing?.remove();
        const toast = createNode("button", "agent-completion-toast");
        toast.id = "agent-completion-toast";
        toast.type = "button";
        toast.setAttribute("aria-live", "polite");
        toast.title = notice.projectTitle || notice.title || "Agent notification";
        const title = createNode(
          "span",
          "agent-completion-toast__title",
          notice.title || "Agent notification",
        );
        const body = createNode(
          "span",
          "agent-completion-toast__body",
          notice.body || "",
        );
        toast.append(title, body);
        toast.addEventListener("click", () => {
          if (notice.projectId) {
            frontendUnits.projectWorkspaceShell.clearProjectUnread(notice.projectId);
            send({ kind: "select_project_tab", tab_id: notice.projectId });
          }
          toast.remove();
        });
        document.body.appendChild(toast);
        window.setTimeout(() => {
          if (toast.isConnected) {
            toast.remove();
          }
        }, 12_000);
      }

      // SPEC-2356 Anshin Addendum (FR-040): in-app attention toast. Distinct from
      // the away-only desktop notification above — this surfaces even while the
      // operator is present. Click flies the camera to the window (frameWindow).
      // Newest toasts stack on top; closing one lets the rest settle down. Quiet
      // flavors auto-hide, but ERROR toasts persist until the operator dismisses
      // them so a failure is never missed.
      function showAttentionToast(notice) {
        const flavor = notice.flavor || "needs_input";
        const existing = document.getElementById(`attention-toast-${notice.windowId}`);
        existing?.remove();
        const toast = createNode("div", "attention-toast");
        toast.id = `attention-toast-${notice.windowId}`;
        toast.dataset.flavor = flavor;
        toast.setAttribute("role", "status");
        toast.setAttribute("aria-live", "polite");

        const jump = createNode("button", "attention-toast__jump");
        jump.type = "button";
        jump.title = notice.title || "Agent attention";
        jump.setAttribute("aria-label", `${notice.title}: ${notice.body} (jump to window)`);
        const title = createNode("span", "attention-toast__title", notice.title || "Agent attention");
        const body = createNode("span", "attention-toast__body", notice.body || "");
        jump.append(title, body);
        jump.addEventListener("click", () => {
          frameWindow(notice.windowId);
          dismissAttentionToast(toast);
        });

        const dismiss = createNode("button", "attention-toast__dismiss", "×");
        dismiss.type = "button";
        dismiss.setAttribute("aria-label", "Dismiss notification");
        dismiss.addEventListener("click", (event) => {
          event.stopPropagation();
          dismissAttentionToast(toast);
        });

        toast.append(jump, dismiss);
        // Newest on top: prepend so the freshest attention sits above older ones
        // and closing a toast lets the stack settle downward.
        attentionToastStack().prepend(toast);
        // ERROR holds until the operator dismisses it; quieter flavors auto-hide
        // so transient notices never pile up.
        if (flavor !== "error") {
          const holdMs = flavor === "done" ? 8_000 : 14_000;
          window.setTimeout(() => {
            if (toast.isConnected) {
              dismissAttentionToast(toast);
            }
          }, holdMs);
        }
      }

      // Collapse a toast out (height + fade) so the rest of the stack settles
      // smoothly, then remove it. A fallback timer guarantees removal even when
      // the transition is skipped (reduced-motion / detached node).
      function dismissAttentionToast(toast) {
        if (!toast || toast.dataset.leaving === "true") {
          return;
        }
        toast.dataset.leaving = "true";
        toast.addEventListener(
          "transitionend",
          () => {
            toast.remove();
          },
          { once: true },
        );
        window.setTimeout(() => {
          toast.remove();
        }, 320);
      }

      // The attention toasts stack in a fixed column so multiple windows can
      // ask for attention at once without overlapping.
      function attentionToastStack() {
        let stackEl = document.getElementById("attention-toast-stack");
        if (!stackEl) {
          stackEl = createNode("div", "attention-toast-stack");
          stackEl.id = "attention-toast-stack";
          document.body.appendChild(stackEl);
        }
        return stackEl;
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
        launchPending,
      });

      // SPEC-3064 Phase 3 (E6b): the Branches window & cleanup surface
      // (branch list state map, branch rows/list rendering, cleanup modal
      // flow, Workspace Overview cleanup entry, Branches window mount, and
      // the branch_cleanup_* / branch_error receive bodies) moved to
      // /branches-cleanup-surface.js. app.js keeps the receive() case arms,
      // the frontendUnits registry entries, the connection-loss sweep, and
      // the window-cleanup call sites, wired through this factory.
      const {
        branchListStateMap,
        ensureBranchListState,
        requestBranches,
        renderBranches,
        syncBranchSelectionState,
        openBranchCleanupModal,
        closeBranchCleanupModal,
        renderBranchCleanupModal,
        updateBranchCleanupProgress,
        failRunningBranchCleanup,
        failLoadingBranchesOnConnectionLoss,
        openWorkspaceCleanup,
        mountBranchesWindow,
        clearBranchCleanupForWindow,
        applyBranchCleanupReceiveEvent,
      } = createBranchesCleanupSurface({
        send,
        createNode,
        windowMap,
        focusWindowLocally,
        sendWindowFocus: (id) => socketTransport.send({ kind: "focus_window", id }),
        branchCleanupModal,
        branchCleanupDialog,
        launchPending,
        visibleBounds,
        getActiveWorkProjection: () => activeWorkProjection,
        renderWorkspaceWindows: () => workspaceOverviewSurface.renderWindows(),
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
        focusBoardEntry,
        getResumeBounds: () => visibleBounds(),
        launchPending,
        branchesSurface: {
          ensureBranchListState: (...a) => ensureBranchListState(...a),
          requestBranches: (...a) => requestBranches(...a),
          renderBranches: (...a) => renderBranches(...a),
          openBranchCleanupModal: (...a) => openBranchCleanupModal(...a),
        },
      });
      const improvementInboxSurface = createImprovementInboxSurface({
        createNode,
        send,
      });

      // SPEC-3064 Phase 3 (E5): the Launch Wizard surface (wizard state,
      // interaction guard, field builders, state transitions,
      // renderLaunchWizard, chrome listeners, Esc-close path) moved to
      // /launch-wizard-surface.js. app.js keeps openIssueLaunchWizard /
      // sendWizardAction (transport helpers), the receive() case arms, and
      // the frontendUnits registry entries, wired through this factory.
      const {
        syncWizardDraftState,
        flushWizardBranchDraft,
        renderLaunchWizard,
        openStartWorkPendingWizard,
        openLaunchAgentPendingWizard,
        applyLaunchWizardStateEvent,
        applyLaunchWizardOpenErrorEvent,
        applyWorkAdvisoryResultEvent,
        handleWizardEscapeKeydown,
        installWizardChrome,
      } = createLaunchWizardSurface({
        createNode,
        closeModal,
        sendWizardAction,
        requestWorkAdvisory,
      });

      const issueMonitorSurface = createIssueMonitorSurface({
        document,
        send,
      });

      const agentKanbanSurface = createAgentKanbanSurface({
        activeWorkspace,
        createTerminalRuntime: (id, terminalRoot) =>
          createTerminalRuntime(id, terminalRoot),
        send,
        visibleBounds,
        windowDisplayTitle,
        windowRoleBadgeLabel,
        onLaunchAgent: ({ boardId, laneId }) => {
          const knownAgentWindowIds = new Set(
            (activeWorkspace().windows || [])
              .filter((windowData) => isAgentWindowPreset(windowData.preset))
              .map((windowData) => windowData.id),
          );
          agentKanbanPendingPlacement.begin({
            boardId,
            laneId,
            knownAgentWindowIds,
          });
          openLaunchAgentPendingWizard();
          send({
            kind: "open_agent_kanban_launch_wizard",
            board_id: boardId,
            lane_id: laneId,
          });
        },
      });

      // SPEC-3064 Phase 3 (E7): the project & workspace shell chrome surface
      // (project tabs + close-tab confirm modal + tab cues, Recent Projects,
      // Open Project split-button menu, picker/onboarding renderers, action
      // availability, Window List dropdown + render key, maximized-window
      // viewport sync, open/clone project modal glue, migration modal glue,
      // the window_list / clone_* / migration_* receive bodies, and the
      // project-shell chrome listeners) moved to /project-shell-surface.js.
      // app.js keeps renderAppState (the central render orchestrator), the
      // receive() case arms, the frontendUnits registry entries, and the
      // global Esc handler ordering, wired through this factory.
      const {
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
      } = createProjectShellSurface({
        send,
        createNode,
        // appState / projectError are reassigned in app.js, so the surface
        // reads them through accessors.
        getAppState: () => appState,
        getProjectError: () => projectError,
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
        // SPEC-2008 camera-focus: window-list rows fly the camera to a window
        // via frameWindow (teleport) instead of maximizing/restoring it.
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
        isModalOpen: () =>
          modal.classList.contains("open") ||
          wizardModal.classList.contains("open") ||
          branchCleanupModal?.classList.contains("open"),
      });

      function mountWindowBody(windowData, element) {
        const body = element.querySelector(".window-body");
        body.innerHTML = "";
        const surface = presetSurface(windowData.preset);
        element.classList.remove(
          "surface-terminal",
          "surface-agent-kanban",
          "surface-file-tree",
          "surface-branches",
          "surface-board",
          "surface-logs",
          "surface-knowledge",
          "surface-issue-monitor",
          "surface-index",
          "surface-work",
          "surface-improvement",
          "surface-profile",
          "surface-console",
          "surface-mock",
        );
        element.classList.add(`surface-${surface}`);

        if (surface === "agent-kanban") {
          agentKanbanSurface.mount(body, windowData);
          return;
        }

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
          // SPEC-2008 camera-focus (UX fix): clicking a terminal focuses only —
          // front + highlight + input routing — and never moves the camera
          // (focusWindowRemotely omits center, so no bounds/zoom change). The
          // camera only moves on the deliberate double-click / minimap / cycle
          // framing gestures.
          overlay.addEventListener("mousedown", () => {
            focusWindowRemotely(windowData.id);
          });
          overlay.appendChild(spinner);
          overlay.appendChild(message);
          overlay.appendChild(copyButton);
          updateTerminalOverlayCopyState(overlay);
          terminalRoot.addEventListener("mousedown", () => {
            focusWindowRemotely(windowData.id);
          });
          terminalRoot.addEventListener("click", () => {
            const runtime = terminalMap.get(windowData.id);
            runtime?.terminal.focus();
          });
          frontendUnits.terminalHost.createRuntime(windowData.id, terminalRoot);
          return;
        }

        if (surface === "file-tree") {
          // SPEC-3064 Phase 3 (E6a): the File Tree window mount moved to
          // the file tree surface.
          mountFileTreeWindow(windowData, body);
          return;
        }

        if (surface === "branches") {
          // SPEC-3064 Phase 3 (E6b): the Branches window mount moved to
          // the branches cleanup surface.
          mountBranchesWindow(windowData, body);
          return;
        }

        if (surface === "profile") {
          // SPEC-3064 Phase 3 (E6e): the Profile window mount moved to the
          // profile window surface.
          mountProfileWindow(windowData, body);
          return;
        }

        if (surface === "board") {
          // SPEC-3064 Phase 3 (E6c): the Board window mount moved to the
          // board & logs surface.
          mountBoardWindow(windowData, body);
          return;
        }

        if (surface === "logs") {
          // SPEC-3064 Phase 3 (E6c): the Logs window mount moved to the
          // board & logs surface.
          mountLogsWindow(windowData, body);
          return;
        }

        if (surface === "work") {
          workspaceOverviewSurface.mount(body, windowData, {
            focusWindowLocally,
            sendFocus: (id) => socketTransport.send({ kind: "focus_window", id }),
          });
          // SPEC-2359 US-83: fetch the eligible remote branches for this
          // Workspace window so they fold into the unified Workspace list as
          // Remote-tagged rows. Sent from app.js (not the surface's mount) so
          // the surface stays a pure renderer and its unit tests keep their
          // exact-message contracts.
          send({ kind: "request_remote_start_work_branches", id: windowData.id });
          return;
        }

        if (surface === "improvement") {
          improvementInboxSurface.mount(body, {
            ...windowData,
            improvement_candidates: improvementCandidates,
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
          mountProjectIndexSurface(body, windowData);
          return;
        }

        if (surface === "knowledge") {
          // SPEC-3064 Phase 3 (E6d): the Knowledge window mount moved to
          // the knowledge kanban surface.
          mountKnowledgeWindow(windowData, body);
          return;
        }

        if (surface === "issue-monitor") {
          issueMonitorSurface.mount(body, windowData);
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
        // SPEC-2008 camera-focus (UX fix): body click focuses only, no camera
        // move (focusWindowRemotely without center).
        body.addEventListener("mousedown", () => {
          focusWindowRemotely(windowData.id);
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

      function refreshMountedImprovementInboxWindows() {
        for (const element of document.querySelectorAll(
          '.workspace-window[data-preset="improvement"]',
        )) {
          const body = element.querySelector(".window-body");
          if (!body) continue;
          improvementInboxSurface.mount(body, {
            improvement_candidates: improvementCandidates,
          });
        }
      }

      // SPEC-3064 Phase 3 (E4): the Settings windows surface (tabbed
      // Settings body, customAgentsState / agentBackendsState /
      // systemSettingsState, Teams channel converters, add-from-preset
      // flow, autostart appliers, and the system-settings interaction
      // guard) moved to /settings-surface.js. app.js keeps the receive()
      // case arms, the frontendUnits registry entries, and the delegated
      // document-level <select> guard listeners, wired through this
      // factory.
      const {
        customAgentsState,
        agentBackendsState,
        systemSettingsState,
        systemSettingsInteractionGuard,
        applyAutostartStatus,
        applyAutostartError,
        applyCustomAgentDeleted,
        applyCustomAgentError,
        renderSettingsWindow,
        renderSettingsAgentList,
        renderAgentBackendsPanel,
        renderAgentBackendsPanelInAllSettingsWindows,
        renderSystemPanelInAllSettingsWindows,
        renderIndexPanelInAllSettingsWindows,
        requestFullIndexStatusRefresh,
        setSettingsStatus,
        completeAddFromPreset,
      } = createSettingsSurface({
        send,
        createNode,
        focusWindowLocally,
        activeProjectTab,
        focusOrSpawnPreset,
        renderUsagePanel,
        indexStatusByProjectRoot,
      });

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
                <button class="icon-button" data-action="restart" aria-label="Restart agent" title="Restart agent" hidden>↻</button>
                <button class="icon-button" data-action="stop" aria-label="Stop agent" title="Stop agent" hidden>■</button>
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
          const closeButton = element.querySelector("[data-action='close']");
          const stopButton = element.querySelector("[data-action='stop']");
          const restartButton = element.querySelector("[data-action='restart']");
          const resizeHandle = element.querySelector(".resize-handle");

          // SPEC-2356 Anshin Addendum (FR-041/FR-044): the kill-switch lives in
          // the window chrome next to close. STOP halts the agent runtime but
          // keeps the window + its output; RESTART relaunches the same preset
          // in place. Both target the window id; visibility is driven per
          // render from the runtime state by the status-apply path.
          stopButton.addEventListener("click", (event) => {
            event.stopPropagation();
            send({ kind: "stop_window", id: windowData.id });
          });
          restartButton.addEventListener("click", (event) => {
            event.stopPropagation();
            send({ kind: "restart_window", id: windowData.id });
          });

          // SPEC-2008 camera-focus: minimize/maximize buttons were removed
          // (focusing a window flies the camera to frame it). The close (×)
          // button and the manual resize handle remain — framing fits the
          // CAMERA to the window, but the user can still resize the WINDOW
          // itself (e.g. to grow a tiled window back up).
          closeButton.addEventListener("click", (event) => {
            event.stopPropagation();
            requestCloseWindow(windowData.id);
          });

          titlebar.addEventListener("pointerdown", (event) => {
            if (event.target.closest("button")) {
              return;
            }
            focusWindowRemotely(windowData.id);
            dragState = {
              id: windowData.id,
              pointerId: event.pointerId,
              startX: event.clientX,
              startY: event.clientY,
              left: parseNumber(element.style.left),
              top: parseNumber(element.style.top),
              moved: false,
              // SPEC-2008 camera-focus: windows are no longer maximized, so
              // drag-to-move is always allowed.
              allowMove: true,
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
            event.stopPropagation();
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

        const surface = presetSurface(windowData.preset);
        if (!element.dataset.preset || element.dataset.preset !== windowData.preset) {
          element.dataset.preset = windowData.preset;
          mountWindowBody(windowData, element);
          if (surface === "agent-kanban") {
            renderedAgentKanbanBodyKeys.set(
              windowData.id,
              agentKanbanBodyRenderKey(windowData),
            );
          }
        } else if (surface === "agent-kanban") {
          const nextAgentKanbanBodyKey = agentKanbanBodyRenderKey(windowData);
          if (renderedAgentKanbanBodyKeys.get(windowData.id) !== nextAgentKanbanBodyKey) {
            mountWindowBody(windowData, element);
            renderedAgentKanbanBodyKeys.set(windowData.id, nextAgentKanbanBodyKey);
          }
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
        const previousWidth = parseFloat(element.style.width || "0");
        const previousHeight = parseFloat(element.style.height || "0");
        const applyWorkspaceGeometry = shouldApplyWorkspaceGeometry(geometrySyncState, {
          id: windowData.id,
          geometryRevision: workspaceGeometryRevision(windowData),
        });
        // SPEC-2008 camera-focus: maximize/minimize were removed. Windows always
        // render at their own world `geometry`; "focusing" a window flies the
        // camera to frame it (frameWindow), so there is no per-client fill
        // branch and no ping-pong of a shared maximized geometry anymore.
        const dimensionsChanged =
          applyWorkspaceGeometry &&
          (previousWidth !== windowData.geometry.width ||
            previousHeight !== windowData.geometry.height);
        const shouldPersistTerminalGeometry =
          applyWorkspaceGeometry && dimensionsChanged;
        element.classList.toggle("tabbed", windowTabsFor(windowData).length > 1);
        if (applyWorkspaceGeometry) {
          element.style.left = `${windowData.geometry.x}px`;
          element.style.top = `${windowData.geometry.y}px`;
          element.style.width = `${windowData.geometry.width}px`;
          element.style.height = `${windowData.geometry.height}px`;
        }
        element.style.zIndex = String(windowData.z_index);
        applyStatus(windowData.id, windowData.status, detailMap.get(windowData.id));
        renderedWindowElementKeys.set(windowData.id, nextWindowElementKey);
        if (
          applyWorkspaceGeometry &&
          presetSurface(windowData.preset) === "terminal"
        ) {
          scheduleTerminalFit(windowData.id, shouldPersistTerminalGeometry);
        }
      }

      function renderWorkspace(workspace) {
        return traceMeasure(
          UI_TRACE_EVENT.renderWorkspace,
          { windows: Array.isArray(workspace?.windows) ? workspace.windows.length : 0 },
          () => {
            const pendingAgentKanbanPlacement =
              agentKanbanPendingPlacement.consumePlacementMessage(
                workspace?.windows || [],
              );
            if (pendingAgentKanbanPlacement) {
              send(pendingAgentKanbanPlacement);
            }

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
              // SPEC-2008 camera-focus: nothing to re-sync when the window set is
              // unchanged — windows render at their own geometry and the camera
              // is driven locally (frameWindow), not per-render.
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
                  const pendingOutputScheduled =
                    terminalOutputBatcher.schedulePending(windowId);
                  const pendingRefreshRearmed = rearmPendingTerminalViewportRefresh(
                    windowId,
                    { shouldPersistGeometry: false },
                  );
                  traceUi(UI_TRACE_EVENT.terminalVisibilityReveal, {
                    window_id: windowId,
                    pending_output_scheduled: pendingOutputScheduled,
                    pending_refresh_rearmed: pendingRefreshRearmed,
                  });
                  scheduleTerminalFocusActivation(windowId, {
                    shouldPersistGeometry: false,
                    reason: "visibility_reveal",
                  });
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
              agentCompletionNotifier.forgetWindow(windowId);
              agentAttentionToaster.forgetWindow(windowId);
              document.getElementById(`attention-toast-${windowId}`)?.remove();
              renderedWindowElementKeys.delete(windowId);
              renderedRuntimeStatusKeys.delete(windowId);
              renderedAgentKanbanBodyKeys.delete(windowId);
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
              clearBranchCleanupForWindow(windowId);
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
                  const pendingOutputScheduled =
                    terminalOutputBatcher.schedulePending(windowData.id);
                  const pendingRefreshRearmed = rearmPendingTerminalViewportRefresh(
                    windowData.id,
                    { shouldPersistGeometry: false },
                  );
                  traceUi(UI_TRACE_EVENT.terminalVisibilityReveal, {
                    window_id: windowData.id,
                    pending_output_scheduled: pendingOutputScheduled,
                    pending_refresh_rearmed: pendingRefreshRearmed,
                  });
                  scheduleTerminalFocusActivation(windowData.id, {
                    shouldPersistGeometry: false,
                    reason: "visibility_reveal",
                  });
                },
              });
            }

            // SPEC-2008 camera-focus: keep the rail window-count badge and the
            // empty-canvas state in sync with window mounts/unmounts, not only
            // with agent status events.
            recomputeOperatorTelemetry();

            const topmostId = topmostWindowId(workspace);
            if (topmostId && activeWindowIdSet.has(topmostId)) {
              focusWindowLocally(topmostId);
              scheduleTerminalFocusActivation(topmostId, {
                shouldPersistGeometry: false,
                reason: "topmost_focus",
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
        markProjectUnread,
        clearProjectUnread,
        renderProjectSwitcher,
        renderRecentProjects,
        renderProjectPicker,
        renderProjectOnboarding,
        renderAppState,
      });

      const agentCompletionNotifier = createAgentCompletionNotifier({
        document,
        window,
        showToast: showAgentCompletionToast,
        onProjectUnread: (projectId) => {
          projectWorkspaceShell.markProjectUnread(projectId);
        },
      });

      // SPEC-2356 Anshin Addendum (FR-040): the always-on, in-app counterpart.
      const agentAttentionToaster = createAgentAttentionToaster({
        showToast: showAttentionToast,
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
        focusBoardEntry,
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
        agentKanbanSurface,
        issueMonitorSurface,
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
            scheduleKnowledgeRelatedWorkRefresh();
            recomputeOperatorTelemetry();
            break;
          // SPEC-3064 Phase 3 (E7): window list entries and rendering live
          // in the project shell surface.
          case "window_list":
            applyWindowListEvent(event);
            break;
          case "improvement_candidates":
            improvementCandidates = Array.isArray(event.candidates) ? event.candidates : [];
            improvementCandidatesRevision += 1;
            {
              const workspace = activeWorkspace() || emptyWorkspace();
              renderWorkspace(workspace);
              refreshMountedImprovementInboxWindows();
            }
            break;
          case "improvement_action_result":
            // Candidate list refresh is delivered as a separate
            // improvement_candidates snapshot; no extra UI state is needed here.
            break;
          case "improvement_action_error":
            window.alert(`Improvement action error: ${event.message}`);
            break;
          case "provider_usage":
            applyProviderUsageUi({
              accounts: event.accounts || [],
              sessions: event.sessions || [],
              consumption: event.consumption || [],
            });
            break;
          case "runtime_health":
            window.__operatorShell?.applyRuntimeHealth?.(event.snapshot || {});
            break;
          case "issue_monitor_status":
            frontendUnits.issueMonitorSurface.applyStatus(event.status || {});
            window.__operatorShell?.applyIssueMonitorStatus?.(event.status || {});
            break;
          case "issue_monitor_inbox":
            frontendUnits.issueMonitorSurface.applyInbox(event.items || []);
            break;
          case "issue_monitor_launch_failed":
            frontendUnits.issueMonitorSurface.applyLaunchFailed(event);
            break;
          case "issue_monitor_toast":
            frontendUnits.issueMonitorSurface.showToast(event);
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
          // SPEC-3064 Phase 3 (E6a): file tree / file content state and
          // rendering live in the file tree surface.
          case "file_tree_entries":
          case "file_tree_error":
          case "file_tree_worktrees":
          case "file_tree_worktree_selected":
          case "file_tree_worktree_error":
          case "file_content_text":
          case "file_content_hex":
          case "file_content_saved":
          case "file_content_save_error":
          case "file_content_error":
            applyFileTreeReceiveEvent(event);
            break;
          case "branch_entries": {
            const state = frontendUnits.branchesFileTreeSurface.ensureBranchListState(
              event.id,
            );
            // FR-065/FR-067: ingest via the shared state helper so a stale
            // (older load_id) event is dropped and inventory phases keep the
            // last verified cleanup badges instead of flashing "Safety unknown".
            // Single trailing break (no early break) so the SPEC-2356 telemetry
            // contract test can extract this whole case body.
            const { applied } = applyBranchEntriesEvent(state, event);
            if (applied) {
              syncBranchSelectionState(state);
              frontendUnits.branchesFileTreeSurface.renderBranches(event.id);
              // SPEC-2356 — feed branch count into the Operator Status Strip WK
              // cell via develop's guarded telemetry helper. The dead Sidebar
              // Layers `git` counter was removed in the operator chrome cleanup,
              // so only `branches` is forwarded now. Count from state.entries so
              // the telemetry reflects the post-ingest list (FR-065 carry-over).
              const branchesCount = Array.isArray(state.entries) ? state.entries.length : 0;
              applyOperatorTelemetryCounts({
                branches: branchesCount,
              });
            }
            break;
          }
          // SPEC-3064 Phase 3 (E6e): profile state and rendering live in
          // the profile window surface.
          case "profile_snapshot":
            applyProfileReceiveEvent(event);
            break;
          // SPEC-3064 Phase 3 (E6c): board/log snapshot, history paging,
          // and live append state live in the board & logs surface.
          case "board_entries":
          case "board_history_page":
          case "log_entries":
          case "log_entry_appended":
            applyBoardLogsReceiveEvent(event);
            break;
          case "project_board_config":
            // SPEC-2963 FR-030: per-project Board routing → Board window chip.
            applyProjectBoardConfigEventToBoard(event);
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
          // SPEC-3064 Phase 3 (E6d): knowledge bridge state and rendering
          // live in the knowledge kanban surface.
          case "knowledge_entries":
          case "knowledge_search_results":
          case "knowledge_detail":
            applyKnowledgeReceiveEvent(event);
            break;
          // SPEC-3064 Phase 3 (E6e): profile state and rendering live in
          // the profile window surface.
          case "profile_error":
            applyProfileReceiveEvent(event);
            break;
          // SPEC-3064 Phase 3 (E6c): board/log error state and rendering
          // live in the board & logs surface.
          case "board_error":
          case "log_error":
            applyBoardLogsReceiveEvent(event);
            break;
          // SPEC-3064 Phase 3 (E6d): knowledge bridge state and rendering
          // live in the knowledge kanban surface.
          case "knowledge_bridge_phase_updated":
          case "knowledge_error":
            applyKnowledgeReceiveEvent(event);
            break;
          case "project_open_error":
            projectError = event.message;
            frontendUnits.projectWorkspaceShell.renderProjectPicker();
            frontendUnits.projectWorkspaceShell.renderProjectOnboarding(
              frontendUnits.projectWorkspaceShell.activeProjectTab(),
            );
            break;
          // SPEC-3064 Phase 3 (E7): clone-project modal state and rendering
          // live in the project shell surface.
          case "clone_project_parent_selected":
          case "github_repository_search_results":
          case "github_repository_search_error":
          case "clone_project_progress":
          case "clone_project_done":
          case "clone_project_error":
            applyCloneProjectReceiveEvent(event);
            break;
          case "launch_wizard_open_error":
            // SPEC-3064 Phase 3 (E5): guard defer + wizard state mutation
            // live in the launch wizard surface.
            agentKanbanPendingPlacement.clear();
            applyLaunchWizardOpenErrorEvent(event);
            break;
          // SPEC-2359 US-42 — Resume Picker dispatcher slots.
          case "workspace_resumable_agents":
            workspaceResumePicker.handleAgentsList(event);
            break;
          // SPEC-2359 US-83 — eligible remote branches render as "Start work on
          // a branch" rows in the Workspace list.
          case "remote_start_work_branches":
            workspaceOverviewSurface.applyRemoteStartWorkBranches(event);
            break;
          case "workspace_resume_agent_error":
            launchPending.settleAck(event);
            workspaceResumePicker.handleError(event);
            break;
          // SPEC-2359 W-17 (FR-398): backend ack that the Resume request was
          // accepted — settle pending UI and dismiss the picker.
          case "workspace_resume_agent_started":
            launchPending.settleAck(event);
            workspaceResumePicker.handleStarted(event);
            scheduleKnowledgeRelatedWorkRefresh();
            break;
          case "launch_wizard_state":
            // SPEC-3064 Phase 3 (E5): guard defer + wizard state mutation
            // live in the launch wizard surface.
            agentKanbanPendingPlacement.clear();
            applyLaunchWizardStateEvent(event);
            break;
          case "work_advisory_result":
            // SPEC-2359 US-80: duplicate-work advisory results for the Start
            // Work intake prompt.
            applyWorkAdvisoryResultEvent(event);
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
            // SPEC-3064 Phase 3 (E4): body reassigns editingCustomAgentId,
            // which now lives in the settings surface.
            applyCustomAgentDeleted(event);
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
            // SPEC-3064 Phase 3 (E4): body reassigns pendingAddFromPreset,
            // which now lives in the settings surface.
            applyCustomAgentError(event);
            break;
          // SPEC-3064 Phase 3 (E7): migration modal state and rendering live
          // in the project shell surface.
          case "migration_detected":
          case "migration_progress":
          case "migration_done":
          case "migration_error":
            applyMigrationReceiveEvent(event);
            break;
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
            const agentKanbanTarget = dragState.allowMove
              ? agentKanbanDropTargetAt(event, dragState.id)
              : null;
            dragState.dockTargetId = dragState.allowMove
              ? titlebarDockTargetAt(event, dragState.id)
              : null;
            clearTitlebarDockPreview();
            if (agentKanbanTarget) {
              send(
                placeAgentWindowMessage(
                  dragState.id,
                  agentKanbanTarget.boardId,
                  agentKanbanTarget.laneId,
                  agentKanbanTarget.order,
                ),
              );
            } else if (dragState.dockTargetId) {
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

      // SPEC-2008 camera-focus / FR-094: instantiate the always-on Fleet
      // Minimap. It lives in `#fleet-minimap` (canvas-area, OUTSIDE the stage)
      // so it is unaffected by the camera transform. Cells reuse the shared
      // data-agent-color → --current-agent map and the Living Telemetry
      // semantic state. Cell click flies the camera (frameWindow).
      fleetMinimap = createFleetMinimap({
        container: document.getElementById("fleet-minimap"),
        getWindows: () => activeWorkspace().windows || [],
        getVisibleBounds: visibleBounds,
        getFocusedId: () => focusedId,
        frameWindow,
        windowDisplayTitle,
        // FR-045 (anshin): the cell tooltip/aria-label surfaces each agent's
        // live activity (title · detail) so the operator can read what every
        // pane is doing now without focusing it.
        cellTooltip: windowActivityLabel,
        // windowData.agent_color already IS the data-agent-color value.
        cellAgentColor: (windowData) => windowData?.agent_color || "",
        // Only agent panes carry a Living Telemetry state; other surfaces
        // render a neutral cell with no telemetry dot.
        cellTelemetryState: (windowData) =>
          shouldShowRuntimeStatus(windowData)
            ? mapAgentTelemetryState(runtimeStateForWindow(windowData))
            : "",
      });
      // First paint from whatever windows already exist at boot.
      fleetMinimap.renderCells();

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

      // SPEC-3064 Phase 3 (E7): the project-shell chrome listeners (Open
      // Project button + picker/onboarding entry points, the Open Project
      // split-button menu wiring, the Windows dropdown trigger, and the
      // dropdown outside-click close) moved into the project shell surface.
      // Installed here so the menu Esc listener keeps registering before the
      // global Esc handler below.
      installProjectShellChrome();
      addButton.addEventListener("click", () => {
        if (addButton.disabled) {
          return;
        }
        openModal();
      });

      // SPEC-3038 AS-4.5: empty-canvas call to action mirrors the rail items.
      document
        .getElementById("canvas-empty-start-work")
        ?.addEventListener("click", () => {
          document.dispatchEvent(
            new CustomEvent("op:command", { detail: { id: "start-work" } }),
          );
        });
      document
        .getElementById("canvas-empty-add-window")
        ?.addEventListener("click", () => {
          if (addButton.disabled) {
            return;
          }
          openModal();
        });
      tileButton.addEventListener("click", () => arrangeWindows("tile"));
      stackButton.addEventListener("click", () => arrangeWindows("stack"));
      alignButton.addEventListener("click", () => arrangeWindows("align"));
      zoomOutButton.addEventListener("click", () => zoomCanvasByFactor(0.9));
      zoomResetButton.addEventListener("click", resetCanvasZoom);
      zoomInButton.addEventListener("click", () => zoomCanvasByFactor(1.1));
      closeModalButton.addEventListener("click", closeModal);
      modal.addEventListener("click", (event) => {
        if (event.target === modal) {
          closeModal();
        }
      });
      // SPEC-3064 Phase 3 (E5): the wizard chrome listeners (footer
      // Cancel/Back/Submit + the wizardBody/wizardModal interaction-guard
      // wiring) moved into the launch wizard surface.
      installWizardChrome();
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
        // SPEC-3064 Phase 3 (E5): the wizard Esc-close path (guard release,
        // error-only local close, cancel dispatch) lives in the launch
        // wizard surface; it preventDefaults and returns true when the
        // wizard modal consumed the event.
        if (handleWizardEscapeKeydown(event)) {
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
        // SPEC-3064 Phase 3 (E7): the migration-modal Esc path (Accept-only
        // swallow at the confirm stage, error-stage dismissal without
        // flipping migration_pending) lives in the project shell surface; it
        // preventDefaults and returns true when the modal consumed the
        // event.
        if (handleMigrationModalEscape(event)) {
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
        // SPEC-3064 Phase 3 (E7): the Windows dropdown Esc path (close +
        // focus restore to the trigger button) lives in the project shell
        // surface. Returns true when it consumed the event.
        if (handleWindowListEscape(event)) {
          return;
        }
        // SPEC-2008 camera-focus: Esc, when nothing else consumed it, zooms the
        // camera out to frame all windows (overview). Guard on
        // defaultPrevented so a modal/dropdown that already handled Esc above
        // does not also trigger overview, and never steal a bare Esc from a
        // focused terminal / text input (vim, TUI apps rely on it).
        if (event.defaultPrevented || isTextEntryFocused()) {
          return;
        }
        enterOverview();
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
        },
      });
      document.addEventListener("visibilitychange", () => {
        if (document.hidden) {
          return;
        }
        rearmVisibleTerminalViewportRefreshes();
      });
      for (const button of modal.querySelectorAll("[data-preset]")) {
        button.addEventListener("click", () => {
          focusOrSpawnPreset(button.dataset.preset);
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

      function isSingletonSurfacePreset(preset) {
        const sessionPresets = new Set(["agent", "shell", "claude", "codex"]);
        return !sessionPresets.has(preset);
      }

      function normalizeSurfacePreset(preset) {
        if (preset === "branches" || preset === "workspace") {
          return "work";
        }
        return preset;
      }

      function openExistingSurfaceWindow(windowData) {
        focusWindowLocally(windowData.id);
        if (windowData.tab_group_id) {
          frontendUnits.socketTransport.send({
            kind: "activate_window_tab",
            id: windowData.id,
          });
        }
        // SPEC-2008 camera-focus: reopening an existing surface flies the local
        // camera to frame it (frameWindow sends focus_window for highlight);
        // restore_window/minimize no longer exist.
        frameWindow(windowData.id, { animate: shouldAnimateWindowFrame() });
      }

      function focusOrSpawnPreset(preset) {
        preset = normalizeSurfacePreset(preset);
        const allWindows = activeWorkspace().windows || [];
        const existing = isSingletonSurfacePreset(preset)
          ? allWindows.find((w) => normalizeSurfacePreset(w.preset) === preset)
          : null;
        if (existing) {
          openExistingSurfaceWindow(existing);
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
          case "open-issue-monitor":
            focusOrSpawnPreset("issue_monitor");
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
          case "stop-all-windows":
            requestStopAllWindows();
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
