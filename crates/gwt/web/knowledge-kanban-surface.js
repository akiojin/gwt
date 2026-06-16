// SPEC-3064 Phase 3 (E6d) — Knowledge Bridge (Issue / SPEC / PR Kanban)
// window surface extracted from app.js. Owns the per-window knowledge
// bridge state map (cache-backed entries, semantic search coalescing,
// detail correlation, auto-refresh timer, kanban hide-done preference),
// the Kanban rendering (columns, cards, drag targets, detail pane), the
// Kanban Drawer (slide-over detail with focus trap), the Knowledge window
// mount, and the knowledge_* receive() bodies. Pure movement from app.js:
// behavior, DOM output, and WS protocol are unchanged; the moved code
// keeps its original app.js indentation. Textual changes are limited to:
// in-module self-references through
// `*` became direct local calls
// (persistKanbanHideDone → writeKanbanHideDonePreference) and the mount's
// focus_window send goes through sendWindowFocus.
//
// deps:
// - send(message): forward a frontend event over the WebSocket bridge.
// - createNode / createKnowledgeMarkdownBody: shared DOM helpers owned by
//   app.js (the markdown body renderer is shared with the Board surface).
// - windowMap / workspaceWindowById: workspace window lookups.
// - pendingIndexOpenTargetsByPreset: index-open handoff targets by preset.
// - knowledgeKindForPreset(preset): issue/spec/pr kind mapping.
// - focusWindowLocally(windowId) / sendWindowFocus(windowId): focus paths.
// - focusOrSpawnPreset(preset): focus-or-spawn used by drawer actions.
// - openIssueLaunchWizard(windowId, issueNumber): launch wizard entry.
import { createFocusTrap } from "/focus-trap.js";

export function createKnowledgeKanbanSurface({
  send,
  createNode,
  createKnowledgeMarkdownBody,
  windowMap,
  workspaceWindowById,
  pendingIndexOpenTargetsByPreset,
  knowledgeKindForPreset,
  focusWindowLocally,
  sendWindowFocus,
  focusOrSpawnPreset,
  openIssueLaunchWizard,
}) {
      const knowledgeBridgeStateMap = new Map();
      const KNOWLEDGE_AUTO_REFRESH_INTERVAL_MS = 60000;
      let nextKnowledgeLoadRequestId = 1;
      let nextKnowledgeSearchRequestId = 1;


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
            queuedLoadRefresh: false,
            loadRecoveryTimer: null,
            loadRecoveryRetryCount: 0,
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
            issueStateFilter: "open",
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
        if (!["open", "closed", "all"].includes(state.issueStateFilter)) {
          state.issueStateFilter = "open";
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
          requestKnowledgeBridge(windowId, knowledgeKind, false);
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
          state.queuedLoadRefresh = false;
          state.loadRecoveryRetryCount = 0;
          if (state.loadRecoveryTimer) {
            clearTimeout(state.loadRecoveryTimer);
            state.loadRecoveryTimer = null;
          }
          state.pendingPhaseUpdates?.clear();
          state.dndSnapshot = null;
        }
        knowledgeBridgeStateMap.delete(windowId);
      }

      function knowledgeEntriesAreEmpty(state) {
        return (
          (!Array.isArray(state.entries) || state.entries.length === 0) &&
          (!Array.isArray(state.baseEntries) || state.baseEntries.length === 0)
        );
      }

      function clearKnowledgeLoadRecoveryTimer(state) {
        if (!state.loadRecoveryTimer) {
          return;
        }
        clearTimeout(state.loadRecoveryTimer);
        state.loadRecoveryTimer = null;
      }

      function scheduleKnowledgeLoadRecovery(windowId, knowledgeKind, requestId) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        clearKnowledgeLoadRecoveryTimer(state);
        state.loadRecoveryTimer = setTimeout(() => {
          state.loadRecoveryTimer = null;
          if (!workspaceWindowById(windowId)) {
            return;
          }
          if (
            !state.loading ||
            state.loadRequestId !== requestId ||
            !knowledgeEntriesAreEmpty(state)
          ) {
            return;
          }
          if (state.loadRecoveryRetryCount < 1) {
            state.loadRecoveryRetryCount += 1;
            state.loading = false;
            state.refreshing = false;
            requestKnowledgeBridge(windowId, knowledgeKind, true);
            renderKnowledgeBridge(windowId);
            return;
          }
          state.loading = false;
          state.refreshing = false;
          state.error = "Timed out loading cache-backed data";
          renderKnowledgeBridge(windowId);
        }, 5000);
      }

      function finishKnowledgeLoad(state, windowId, knowledgeKind) {
        clearKnowledgeLoadRecoveryTimer(state);
        state.loading = false;
        state.refreshing = false;
        state.loadRecoveryRetryCount = 0;
        const queuedRefresh = state.queuedLoadRefresh;
        state.queuedLoadRefresh = false;
        if (queuedRefresh && workspaceWindowById(windowId)) {
          requestKnowledgeBridge(windowId, knowledgeKind, true);
          return true;
        }
        return false;
      }

      function requestKnowledgeBridge(windowId, knowledgeKind, refresh = false) {
        const state = ensureKnowledgeBridgeState(windowId, knowledgeKind);
        if (state.loading) {
          if (refresh && knowledgeEntriesAreEmpty(state)) {
            clearKnowledgeLoadRecoveryTimer(state);
            state.loading = false;
            state.refreshing = false;
          } else {
            state.queuedLoadRefresh = state.queuedLoadRefresh || Boolean(refresh);
            return;
          }
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
        state.queuedLoadRefresh = false;
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
        scheduleKnowledgeLoadRecovery(windowId, effectiveKind, requestId);
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

      function knowledgeDetailChip(detail, knowledgeKind = "spec") {
        if (knowledgeKind === "issue") {
          const rawState = String(detail?.state || "open").toLowerCase();
          return {
            className: rawState === "closed" ? "closed" : "open",
            label: rawState === "closed" ? "Closed" : "Open",
          };
        }
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

      function issueEntryState(entry) {
        return String(entry?.state || "open").toLowerCase() === "closed"
          ? "closed"
          : "open";
      }

      function issueEntryMatchesStateFilter(entry, filter) {
        if (filter === "all") return true;
        return issueEntryState(entry) === filter;
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

      function filteredIssueEntries(state) {
        return filteredKnowledgeEntries(state).filter((entry) =>
          issueEntryMatchesStateFilter(entry, state.issueStateFilter || "open"),
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

      function renderKnowledgeDetailPane(windowId, state, detailPane) {
        detailPane.innerHTML = "";
        const detail = state.detail;
        if (!detail) {
          detailPane.appendChild(
            createNode(
              "div",
              "knowledge-detail-empty",
              state.detailLoading ? "Loading detail" : "Select a cached item",
            ),
          );
          return;
        }

        const header = createNode("div", "knowledge-detail-header");
        const head = createNode("div", "");
        const headRow = createNode("div", "knowledge-detail-head");
        headRow.appendChild(createNode("h3", "knowledge-detail-title", detail.title));
        const detailChip = knowledgeDetailChip(detail, state.kind);
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
        const stalePhase = state.kind === "issue" ? "" : staleKnowledgePhaseWarning(detail);
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
          card.appendChild(createKnowledgeMarkdownBody(section));
          scroll.appendChild(card);
        }
        if (scroll.childElementCount === 0) {
          scroll.appendChild(
            createNode("div", "knowledge-detail-empty", "No cached detail available"),
          );
        }
        detailPane.appendChild(scroll);
      }

      function renderIssueRow(windowId, state, entry) {
        const row = createNode("button", "knowledge-row");
        row.type = "button";
        row.dataset.issueNumber = String(entry.number);
        row.setAttribute("role", "listitem");
        if (state.selectedNumber === entry.number) {
          row.classList.add("selected");
          row.setAttribute("aria-current", "true");
        }

        const main = createNode("div", "knowledge-row-main");
        const titleWrap = createNode("div", "");
        titleWrap.appendChild(
          createNode("div", "knowledge-row-title", entry.title || `Issue #${entry.number}`),
        );
        titleWrap.appendChild(
          createNode("div", "knowledge-row-number", `#${entry.number}`),
        );
        main.appendChild(titleWrap);
        const rawState = issueEntryState(entry);
        main.appendChild(
          createNode(
            "span",
            `knowledge-state-chip ${rawState}`,
            rawState === "closed" ? "Closed" : "Open",
          ),
        );
        row.appendChild(main);

        const meta = createNode("div", "knowledge-row-meta");
        for (const label of visibleKnowledgeLabels(entry.labels || [])) {
          meta.appendChild(createNode("span", "knowledge-chip", label));
        }
        if ((entry.linked_branch_count || 0) > 0) {
          meta.appendChild(
            createNode(
              "span",
              "knowledge-meta-copy",
              `${entry.linked_branch_count} branch${entry.linked_branch_count === 1 ? "" : "es"}`,
            ),
          );
        }
        if (Number.isFinite(entry.match_score)) {
          meta.appendChild(
            createNode("span", "knowledge-meta-copy", `${entry.match_score}% match`),
          );
        }
        if (entry.meta) {
          meta.appendChild(createNode("span", "knowledge-meta-copy", entry.meta));
        }
        if (meta.childElementCount > 0) {
          row.appendChild(meta);
        }

        row.addEventListener("click", () => {
          requestKnowledgeDetail(windowId, state.kind, entry.number);
          renderKnowledgeBridge(windowId);
        });
        return row;
      }

      function renderIssueKnowledgeBridge(windowId, element, state) {
        const list = element.querySelector(".knowledge-list");
        const detailPane = element.querySelector(".knowledge-detail-pane");
        const status = element.querySelector(".knowledge-status");
        const refreshButton = element.querySelector("[data-action='refresh-knowledge']");
        const searchInput = element.querySelector(".knowledge-search");
        if (!list || !detailPane || !status || !refreshButton || !searchInput) {
          return;
        }

        refreshButton.disabled =
          !state.refreshEnabled || (state.loading && !knowledgeEntriesAreEmpty(state));
        searchInput.placeholder = knowledgeSearchPlaceholder(state.kind);
        for (const button of element.querySelectorAll("[data-issue-filter]")) {
          const selected = button.dataset.issueFilter === state.issueStateFilter;
          button.classList.toggle("is-active", selected);
          button.setAttribute("aria-pressed", selected ? "true" : "false");
        }

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
            ? "Refreshing cached issues"
            : "Loading cache-backed issues";
        } else if (state.loading && state.entries.length === 0) {
          status.classList.add("visible", "info");
          status.textContent = "Loading cache-backed issues";
        } else if (state.entries.length === 0 && !state.searching) {
          status.classList.add("visible", "info");
          status.textContent = state.emptyMessage || "No cached issues";
        }

        list.innerHTML = "";
        const visibleEntries = filteredIssueEntries(state);
        if (visibleEntries.length === 0) {
          const filterLabel = state.issueStateFilter === "all"
            ? ""
            : `${state.issueStateFilter || "open"} `;
          list.appendChild(
            createNode("div", "knowledge-empty", `No ${filterLabel}issues`),
          );
        } else {
          for (const entry of visibleEntries) {
            list.appendChild(renderIssueRow(windowId, state, entry));
          }
        }
        renderKnowledgeDetailPane(windowId, state, detailPane);
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
        if (state.kind === "issue") {
          renderIssueKnowledgeBridge(windowId, element, state);
          return;
        }
        const board = element.querySelector(".kanban-board");
        const detailPane = element.querySelector(".knowledge-detail-pane");
        const status = element.querySelector(".knowledge-status");
        const refreshButton = element.querySelector("[data-action='refresh-knowledge']");
        const searchInput = element.querySelector(".knowledge-search");
        const hideDoneToggle = element.querySelector("[data-action='kanban-hide-done']");
        if (!board || !detailPane || !status || !refreshButton || !searchInput) {
          return;
        }

        refreshButton.disabled =
          !state.refreshEnabled || (state.loading && !knowledgeEntriesAreEmpty(state));
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

        renderKnowledgeDetailPane(windowId, state, detailPane);
      }
      // SPEC-3064 Phase 3 (E6d): Knowledge window mount moved verbatim from
      // app.js mountWindowBody (surface === "knowledge" branch).
      function mountKnowledgeWindow(windowData, body) {
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
          if (knowledgeKind === "issue") {
            body.innerHTML = `
              <div class="knowledge-root issue-bridge-root">
                <div class="workspace-toolbar kanban-toolbar is-stacked">
                  <div class="workspace-toolbar-main">
                    <div class="knowledge-heading">${knowledgeHeading(knowledgeKind)}</div>
                    <input class="knowledge-search" type="search" placeholder="${knowledgeSearchPlaceholder(knowledgeKind)}" />
                    <div class="knowledge-state-filter" role="group" aria-label="Issue state filter">
                      <button type="button" data-issue-filter="open">Open</button>
                      <button type="button" data-issue-filter="closed">Closed</button>
                      <button type="button" data-issue-filter="all">All</button>
                    </div>
                  </div>
                  <div class="workspace-toolbar-actions">
                    <button class="icon-button" data-action="refresh-knowledge" aria-label="Refresh cached issues">↻</button>
                  </div>
                </div>
                <div class="knowledge-status"></div>
                <div class="knowledge-split workspace-split issue-list-shell">
                  <div class="knowledge-list-pane">
                    <div class="knowledge-list" role="list" aria-label="Cached issues"></div>
                  </div>
                  <div class="knowledge-detail-pane"></div>
                </div>
              </div>
            `;
          }
          body.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            sendWindowFocus(windowData.id);
          });
          const state = ensureKnowledgeBridgeState(
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
            scheduleKnowledgeSearch(
              windowData.id,
              knowledgeKind,
            );
          });
          body
            .querySelector("[data-action='refresh-knowledge']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              requestKnowledgeBridge(
                windowData.id,
                knowledgeKind,
                true,
              );
              renderKnowledgeBridge(
                windowData.id,
              );
            });
          for (const filterButton of body.querySelectorAll("[data-issue-filter]")) {
            filterButton.addEventListener("click", (event) => {
              event.stopPropagation();
              state.issueStateFilter = filterButton.dataset.issueFilter || "open";
              renderKnowledgeBridge(
                windowData.id,
              );
            });
          }
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
              writeKanbanHideDonePreference(
                state.hideDone,
              );
              renderKnowledgeBridge(
                windowData.id,
              );
            });
          }
          if (!state.loading && (!state.detail || knowledgeEntriesAreEmpty(state))) {
            requestKnowledgeBridge(
              windowData.id,
              knowledgeKind,
              false,
            );
          }
          ensureKnowledgeAutoRefresh(windowData.id, knowledgeKind);
          renderKnowledgeBridge(
            windowData.id,
          );
          return;
      }

      // SPEC-3064 Phase 3 (E6d): receive() bodies for knowledge_* events
      // moved verbatim from app.js; the case arms in app.js delegate here.
      function applyKnowledgeReceiveEvent(event) {
        switch (event.kind) {
          case "knowledge_entries": {
            const state = ensureKnowledgeBridgeState(
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
            state.error = "";
            if (finishKnowledgeLoad(state, event.id, event.knowledge_kind)) {
              renderKnowledgeBridge(event.id);
              break;
            }
            if (queuedQuery) {
              scheduleKnowledgeSearch(
                event.id,
                event.knowledge_kind,
              );
              break;
            }
            renderKnowledgeBridge(event.id);
            break;
          }
          case "knowledge_search_results": {
            const state = ensureKnowledgeBridgeState(
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
                scheduleKnowledgeSearch(
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
              scheduleKnowledgeSearch(
                event.id,
                event.knowledge_kind,
              );
              break;
            }
            state.searching = false;
            if (state.selectedNumber) {
              state.detailLoading = true;
              requestKnowledgeDetail(
                event.id,
                event.knowledge_kind,
                state.selectedNumber,
              );
            } else {
              state.detail = null;
            }
            renderKnowledgeBridge(event.id);
            break;
          }
          case "knowledge_detail": {
            const state = ensureKnowledgeBridgeState(
              event.id,
              event.knowledge_kind,
            );
            if (!knowledgeDetailRequestMatches(state, event)) {
              break;
            }
            const matchesLoadRequest =
              !event.request_id || event.request_id === state.loadRequestId;
            state.detail = event.detail;
            state.selectedNumber = event.detail?.number ?? state.selectedNumber ?? null;
            if (matchesLoadRequest) {
              finishKnowledgeLoad(state, event.id, event.knowledge_kind);
            }
            state.detailLoading = false;
            renderKnowledgeBridge(event.id);
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
          // SPEC-3064 Phase 3 (E6b): branch cleanup state and rendering
          // live in the branches cleanup surface.
          case "branch_cleanup_result":
          case "branch_cleanup_progress":
          case "branch_error":
            applyBranchCleanupReceiveEvent(event);
            break;
          case "knowledge_bridge_phase_updated": {
            // SPEC-2017 US-8 — phase write-back response. On Ok we
            // overwrite the optimistic card with fresh_entry and clear
            // the pending marker so the spinner stops; on Error we
            // rollback from dndSnapshot and surface a toast.
            const state = ensureKnowledgeBridgeState(
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
            renderKnowledgeBridge(event.id);
            break;
          }
          case "knowledge_error": {
            const state = ensureKnowledgeBridgeState(
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
                  scheduleKnowledgeSearch(
                    event.id,
                    event.knowledge_kind,
                  );
                }
              }
              break;
            }
            if (
              !isSearchError &&
              !knowledgeDetailRequestMatches(state, event)
            ) {
              break;
            }
            const matchesLoadRequest =
              !event.request_id || event.request_id === state.loadRequestId;
            const startedQueuedRefresh = matchesLoadRequest
              ? finishKnowledgeLoad(state, event.id, event.knowledge_kind)
              : false;
            if (matchesLoadRequest) {
              state.error = startedQueuedRefresh ? "" : event.message;
            } else {
              state.error = event.message;
            }
            state.searching = false;
            state.searchInFlight = false;
            state.inFlightSearchRequestId = 0;
            state.queuedSearchQuery = "";
            state.detailLoading = false;
            renderKnowledgeBridge(event.id);
            break;
          }
          default:
            break;
        }
      }

      return {
        knowledgeBridgeStateMap,
        ensureKnowledgeBridgeState,
        clearKnowledgeBridgeState,
        requestKnowledgeBridge,
        scheduleKnowledgeSearch,
        requestKnowledgeDetail,
        knowledgeDetailRequestMatches,
        renderKnowledgeBridge,
        writeKanbanHideDonePreference,
        openKanbanDrawer,
        closeKanbanDrawer,
        renderKanbanDrawerBody,
        mountKnowledgeWindow,
        applyKnowledgeReceiveEvent,
      };
}
