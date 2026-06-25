// SPEC-3064 Phase 3 (E6c) — Board & Logs window surface extracted from
// app.js. Owns the per-window board/log state maps, the Work-id tracking
// that scopes the Board Work filter (currentProjectWorkspaceId /
// activeWorkProjectionWorkspaceIds), the board chat rendering (lanes,
// composer, mention notification, reply flow, history paging), the logs
// rendering (severity/process facets, unread jump), both window mounts,
// and the board_* / log_* receive() bodies. Pure movement from app.js:
// behavior, DOM output, and WS protocol are unchanged; the moved code
// keeps its original app.js indentation. Textual changes are limited to:
// in-module self-references through `*` /
// `*` became direct local calls,
// `activeWorkProjection` reads go through the injected
// getActiveWorkProjection accessor, and the mounts' focus_window sends go
// through sendWindowFocus.
//
// deps:
// - send(message): forward a frontend event over the WebSocket bridge.
// - createNode / createKnowledgeMarkdownBody: shared DOM helpers owned by
//   app.js (the markdown body renderer is shared with the knowledge
//   surface).
// - windowMap: workspace window element map owned by app.js.
// - focusWindowLocally(windowId) / sendWindowFocus(windowId): focus paths.
// - focusOrSpawnPreset(preset): focus-or-spawn for Board origin jumps.
// - activeWorkspace() / activeProjectTab(): workspace accessors.
// - visibleBounds(): canvas bounds payload helper.
// - getActiveWorkProjection(): read accessor for the active Work
//   projection (app.js owns the let).
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

export function createBoardLogsSurface({
  send,
  createNode,
  createKnowledgeMarkdownBody,
  windowMap,
  focusWindowLocally,
  sendWindowFocus,
  focusOrSpawnPreset,
  activeWorkspace,
  activeProjectTab,
  visibleBounds,
  getActiveWorkProjection,
}) {
      const boardStateMap = new Map();
      const logStateMap = new Map();
      let pendingBoardEntryFocusId = null;

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

      // SPEC-2963 FR-030: per-project Board routing (provider/channel/tenant +
      // resolved routing), keyed by project_root. Populated by the
      // `project_board_config` backend event; read by the Board window's
      // destination chip + config popover.
      const boardConfigByProjectRoot = new Map();
      let activeDestinationPopover = null;

      function boardDestinationLabel(config) {
        if (!config || !config.resolvedProvider) return "Board: …";
        if (config.resolvedProvider === "local") return "Board: Local";
        const name =
          config.resolvedProvider === "slack"
            ? "Slack"
            : config.resolvedProvider === "teams"
              ? "Teams"
              : config.resolvedProvider;
        const channel = config.resolvedChannel ? ` ${config.resolvedChannel}` : "";
        const warn = config.signedIn ? "" : " ⚠";
        return `Board: ${name}${channel}${warn}`;
      }

      // Short destination text for the chip (no "Board:" prefix; the gear icon
      // already signals this is the Board destination control).
      function boardDestinationText(config) {
        if (!config || !config.resolvedProvider) return "Not set";
        if (config.resolvedProvider === "local") return "Local";
        const name =
          config.resolvedProvider === "slack"
            ? "Slack"
            : config.resolvedProvider === "teams"
              ? "Teams"
              : config.resolvedProvider;
        const channel = config.resolvedChannel ? ` ${config.resolvedChannel}` : "";
        const warn = config.signedIn ? " ⚠" : "";
        return `${name}${channel}${warn}`;
      }

      function boardDestinationTitle(config) {
        const base = "Click to configure this project's Board destination.";
        if (!config || !config.resolvedProvider) return base;
        const source =
          config.resolvedSource === "project"
            ? "from this project's board.toml"
            : "inherited from global settings";
        const signedIn =
          config.resolvedProvider !== "local"
            ? config.signedIn
              ? " · signed in"
              : " · not signed in"
            : "";
        return `${boardDestinationLabel(config)} (${source})${signedIn}\n${base}`;
      }

      function closeBoardDestinationPopover() {
        if (activeDestinationPopover) {
          document.removeEventListener(
            "mousedown",
            activeDestinationPopover._onOutside,
            true,
          );
          document.removeEventListener(
            "keydown",
            activeDestinationPopover._onEsc,
            true,
          );
          activeDestinationPopover.remove();
          activeDestinationPopover = null;
        }
      }

      function openBoardDestinationPopover(anchorButton) {
        closeBoardDestinationPopover();
        const projectRoot = activeProjectTab()?.project_root || "";
        if (!projectRoot) return;
        const config = boardConfigByProjectRoot.get(projectRoot) || {};

        const pop = createNode("div", "board-destination-popover");

        const header = createNode("div", "board-destination-popover-header");
        const heads = createNode("div", "board-destination-popover-heads");
        heads.appendChild(
          createNode("div", "board-destination-popover-title", "Board destination"),
        );
        heads.appendChild(
          createNode(
            "div",
            "board-destination-popover-sub",
            "Where this project's Board posts go",
          ),
        );
        header.appendChild(heads);
        const closeBtn = createNode("button", "board-destination-popover-close", "✕");
        closeBtn.type = "button";
        closeBtn.setAttribute("aria-label", "Close");
        closeBtn.addEventListener("click", () => closeBoardDestinationPopover());
        header.appendChild(closeBtn);
        pop.appendChild(header);

        const labelWrap = (text, control, hint) => {
          const field = createNode("div", "board-destination-field");
          field.appendChild(createNode("label", "settings-label", text));
          field.appendChild(control);
          if (hint) field.appendChild(createNode("div", "settings-help", hint));
          pop.appendChild(field);
          return field;
        };

        const provSelect = document.createElement("select");
        provSelect.className = "settings-select";
        for (const [value, text] of [
          ["", "Inherit global"],
          ["local", "Local (offline)"],
          ["slack", "Slack"],
          ["teams", "Teams"],
        ]) {
          const option = document.createElement("option");
          option.value = value;
          option.textContent = text;
          provSelect.appendChild(option);
        }
        provSelect.value = config.provider || "";
        labelWrap(
          "Provider",
          provSelect,
          "Local keeps the Board offline. Slack and Teams share it with your team.",
        );

        const channelInput = document.createElement("input");
        channelInput.className = "settings-input";
        channelInput.type = "text";
        channelInput.value = config.channel || "";
        channelInput.placeholder = "Slack C0123… / Teams team_id/channel_id";
        const channelField = labelWrap("Channel", channelInput);

        const tenantInput = document.createElement("input");
        tenantInput.className = "settings-input";
        tenantInput.type = "text";
        tenantInput.value = config.tenant || "";
        tenantInput.placeholder = "team_id / tenant_id (optional)";
        const tenantField = labelWrap("Tenant", tenantInput);

        // SPEC-2963 FR-030: Channel / Tenant only apply to a remote provider.
        // Switching to Local hides them (and they are cleared on Save), so the
        // form reflects the selected provider.
        const syncProviderFields = () => {
          const remote = provSelect.value !== "local";
          channelField.style.display = remote ? "" : "none";
          tenantField.style.display = remote ? "" : "none";
        };
        provSelect.addEventListener("change", syncProviderFields);
        syncProviderFields();

        if (config.resolvedProvider) {
          const sourceText =
            config.resolvedSource === "project"
              ? "set for this project"
              : "from the global default";
          pop.appendChild(
            createNode(
              "div",
              "board-destination-popover-status",
              `Currently: ${boardDestinationText(config)} (${sourceText})`,
            ),
          );
        }
        if (config.message) {
          pop.appendChild(
            createNode("div", "board-destination-popover-hint", config.message),
          );
        }

        const actions = createNode("div", "board-destination-popover-actions");
        const cancelBtn = createNode("button", "text-button", "Cancel");
        cancelBtn.type = "button";
        cancelBtn.addEventListener("click", () => closeBoardDestinationPopover());
        const saveBtn = createNode("button", "wizard-button primary", "Save");
        saveBtn.type = "button";
        saveBtn.addEventListener("click", () => {
          // Local has no channel/tenant — clear them so they don't persist.
          const isLocal = provSelect.value === "local";
          send({
            kind: "update_project_board_config",
            project_root: projectRoot,
            provider: provSelect.value,
            channel: isLocal ? "" : channelInput.value.trim(),
            tenant: isLocal ? "" : tenantInput.value.trim(),
          });
          closeBoardDestinationPopover();
        });
        actions.appendChild(cancelBtn);
        actions.appendChild(saveBtn);
        pop.appendChild(actions);
        pop.appendChild(
          createNode(
            "div",
            "board-destination-popover-foot",
            "Saved to this repo's .gwt/work/board.toml — shared with your team.",
          ),
        );

        const rect = anchorButton.getBoundingClientRect();
        pop.style.position = "fixed";
        pop.style.top = `${Math.round(rect.bottom + 4)}px`;
        pop.style.left = `${Math.round(rect.left)}px`;
        pop.addEventListener("mousedown", (event) => event.stopPropagation());

        pop._onOutside = (event) => {
          if (!pop.contains(event.target) && event.target !== anchorButton) {
            closeBoardDestinationPopover();
          }
        };
        pop._onEsc = (event) => {
          if (event.key === "Escape") closeBoardDestinationPopover();
        };
        document.body.appendChild(pop);
        activeDestinationPopover = pop;
        setTimeout(() => {
          document.addEventListener("mousedown", pop._onOutside, true);
          document.addEventListener("keydown", pop._onEsc, true);
        }, 0);
      }

      // SPEC-2963 FR-030: apply a `project_board_config` backend event to the
      // Board windows — refreshes the destination chip for the matching project.
      function applyProjectBoardConfigEventToBoard(event) {
        if (!event || !event.project_root) return;
        boardConfigByProjectRoot.set(event.project_root, {
          provider: event.provider || "",
          channel: event.channel || "",
          tenant: event.tenant || "",
          resolvedProvider: event.resolved_provider || "",
          resolvedSource: event.resolved_source || "",
          resolvedChannel: event.resolved_channel || "",
          signedIn: event.signed_in === true,
          message: event.message || "",
        });
        for (const windowId of activeBoardWindowIds()) {
          renderBoard(windowId);
        }
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
        const assigned = Array.isArray(getActiveWorkProjection()?.agents)
          ? getActiveWorkProjection().agents
          : [];
        const unassigned = Array.isArray(getActiveWorkProjection()?.unassigned_agents)
          ? getActiveWorkProjection().unassigned_agents
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

      // SPEC-2959: the known Work lanes available to the Board, derived from
      // the active Work projection. Used for lane labels and the composer
      // "To:" selector.
      function boardLaneWorkspaces() {
        const works = Array.isArray(getActiveWorkProjection()?.active_works)
          ? getActiveWorkProjection().active_works
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
        // SPEC-2963 FR-030: reflect this project's resolved Board destination.
        const destinationChip = body.querySelector("[data-action='board-destination']");
        if (destinationChip) {
          const config = boardConfigByProjectRoot.get(
            activeProjectTab()?.project_root || "",
          );
          destinationChip.replaceChildren(
            createNode("span", "board-destination-dot"),
            createNode(
              "span",
              "board-destination-text",
              boardDestinationText(config),
            ),
            createNode("span", "board-destination-gear", "⚙"),
          );
          destinationChip.title = boardDestinationTitle(config);
          destinationChip.dataset.provider = config?.resolvedProvider || "";
          destinationChip.dataset.signedIn =
            config && config.resolvedProvider !== "local"
              ? config.signedIn
                ? "true"
                : "false"
              : "";
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
      // SPEC-3064 Phase 3 (E6c): Board window mount moved verbatim from
      // app.js mountWindowBody (surface === "board" branch).
      function mountBoardWindow(windowData, body) {
          body.innerHTML = `
            <div class="board-root">
              <div class="workspace-toolbar is-stacked">
                <div class="workspace-toolbar-main">
                  <div class="knowledge-heading">Board chat</div>
                  <div class="board-status"></div>
                </div>
                <div class="workspace-toolbar-actions">
                  <button class="board-destination-chip" data-action="board-destination" type="button" aria-label="Board destination settings"></button>
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
            sendWindowFocus(windowData.id);
          });
          body
            .querySelector("[data-action='refresh-board']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              const state = ensureBoardState(windowData.id);
              state.error = "";
              requestBoard(windowData.id);
              renderBoard(windowData.id);
            });
          body
            .querySelector("[data-action='toggle-board-for-you']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              const state = ensureBoardState(windowData.id);
              state.audienceFilter =
                state.audienceFilter === "for_you" ? "workspace" : "for_you";
              if (state.audienceFilter === "for_you") {
                state.forYouUnread = 0;
              }
              renderBoard(windowData.id);
            });
          body
            .querySelector("[data-action='toggle-board-all']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              const state = ensureBoardState(windowData.id);
              state.audienceFilter = state.audienceFilter === "all" ? "workspace" : "all";
              state.error = "";
              requestBoard(windowData.id);
              renderBoard(windowData.id);
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
              const state = ensureBoardState(windowData.id);
              state.audienceFilter =
                state.audienceFilter === "workspace" ? "all" : "workspace";
              state.error = "";
              requestBoard(windowData.id);
              renderBoard(windowData.id);
            });
          // SPEC-2963 FR-030: per-project Board destination chip → config popover.
          body
            .querySelector("[data-action='board-destination']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              openBoardDestinationPopover(event.currentTarget);
            });
          const projectRoot = activeProjectTab()?.project_root || "";
          if (projectRoot && !boardConfigByProjectRoot.has(projectRoot)) {
            send({ kind: "get_project_board_config", project_root: projectRoot });
          }
          const state = ensureBoardState(windowData.id);
          if (state.entries.length === 0 && !state.loading && !state.error) {
            requestBoard(windowData.id);
          }
          renderBoard(windowData.id);
          return;
      }

      // SPEC-3064 Phase 3 (E6c): Logs window mount moved verbatim from
      // app.js mountWindowBody (surface === "logs" branch).
      function mountLogsWindow(windowData, body) {
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
            sendWindowFocus(windowData.id);
          });
          const state = ensureLogState(windowData.id);
          body
            .querySelector("[data-action='refresh-logs']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              state.error = "";
              requestLogs(windowData.id);
              renderLogs(windowData.id);
            });
          body
            .querySelector(".logs-unread-button")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              jumpToUnread(windowData.id);
            });
          body
            .querySelector(".logs-severity-select")
            .addEventListener("change", (event) => {
              state.severity = event.target.value;
              renderLogs(windowData.id);
            });
          body
            .querySelector(".logs-process-kind-select")
            .addEventListener("change", (event) => {
              state.processKind = event.target.value;
              renderLogs(windowData.id);
            });
          body
            .querySelector(".logs-search-input")
            .addEventListener("input", (event) => {
              state.query = event.target.value;
              renderLogs(windowData.id);
            });
          if (state.entries.length === 0 && !state.loading && !state.error) {
            requestLogs(windowData.id);
          }
          renderLogs(windowData.id);
          return;
      }

      // SPEC-3064 Phase 3 (E6c): receive() bodies for board_* / log_*
      // events moved verbatim from app.js; the case arms in app.js delegate
      // here. (process_line / process_console_snapshot stay in app.js with
      // the Console window controllers.)
      function applyBoardLogsReceiveEvent(event) {
        switch (event.kind) {
          case "board_entries": {
            const state = ensureBoardState(event.id);
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
            renderBoard(event.id);
            break;
          }
          case "board_history_page": {
            const state = ensureBoardState(event.id);
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
            renderBoard(event.id);
            if (
              state.focusEntryId &&
              !state.entries.some((entry) => entry.id === state.focusEntryId) &&
              state.hasMoreBefore
            ) {
              requestOlderBoardEntries(event.id);
            }
            break;
          }
          case "log_entries": {
            const state = ensureLogState(event.id);
            state.entries = event.entries || [];
            state.loading = false;
            state.error = "";
            state.unreadAlerts = 0;
            state.unreadEntryId = null;
            if (!state.selectedEntryId || !state.entries.some((entry) => entry.id === state.selectedEntryId)) {
              state.selectedEntryId =
                state.entries.length > 0 ? state.entries[state.entries.length - 1].id : null;
            }
            renderLogs(event.id);
            break;
          }
          case "log_entry_appended":
            appendLiveLogEntry(event.entry);
            break;
          case "board_error": {
            const state = ensureBoardState(event.id);
            state.loading = false;
            state.loadingOlder = false;
            state.submitting = false;
            state.pendingSubmit = null;
            state.error = event.message;
            renderBoard(event.id);
            break;
          }
          case "log_error": {
            const state = ensureLogState(event.id);
            state.loading = false;
            state.error = event.message;
            renderLogs(event.id);
            break;
          }
          default:
            break;
        }
      }

      return {
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
      };
}
