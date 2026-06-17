// SPEC-3064 Phase 3 (E3) — Project Index window surface extracted from
// app.js: the per-project index status map (setIndexStatus), the Index
// window search state/render/search/open helpers, and the Index window
// mount (Search + Health tabs). Pure movement from app.js: behavior, DOM
// output, and the WS protocol (search_project_index /
// project_index_search_results / project_index_search_error /
// refresh_index_status) are unchanged; the moved code keeps its original
// app.js indentation.
import { renderIndexSettingsPanel } from "/index-settings-panel.js";

// deps:
// - send(message): forward a frontend event over the WebSocket bridge.
// - sendWindowFocus(id): send the focus_window event for a window (wraps
//   the app.js socketTransport, which is constructed after this factory).
// - focusWindowLocally(id): local focus bookkeeping owned by app.js.
// - activeProjectTab(): active project tab lookup owned by app.js.
// - makeEl / clearChildren: DOM helpers owned by app.js.
// - focusOrSpawnPreset / knowledgeKindForPreset / requestKnowledgeDetail /
//   renderKnowledgeBridge: knowledge-window integration owned by app.js.
// - renderIndexPanelInAllSettingsWindows / refreshProjectTabStateCues: status
//   fan-out hooks owned by app.js (Settings panel + project tab state cues).
// - requestFullIndexStatusRefresh(): Health tab refresh request owned by
//   app.js (also used by Settings).
export function createProjectIndexSearchSurface({
  send,
  sendWindowFocus,
  focusWindowLocally,
  activeProjectTab,
  makeEl,
  clearChildren,
  focusOrSpawnPreset,
  knowledgeKindForPreset,
  requestKnowledgeDetail,
  renderKnowledgeBridge,
  renderIndexPanelInAllSettingsWindows,
  refreshProjectTabStateCues,
  requestFullIndexStatusRefresh,
}) {
      const indexSearchStateMap = new Map();
      const pendingIndexOpenTargetsByPreset = new Map();
      const indexStatusByProjectRoot = new Map();

      // SPEC-1939 Phase 13: project-bar Index badge withdrawn. The
      // aggregated payload now feeds only the Index Health tab and the
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
        refreshProjectTabStateCues();
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

      // SPEC-3064 Phase 3 (E3): the Index window mount moved here from the
      // app.js mountWindowBody (surface === "index") branch.
      function mountProjectIndexSurface(body, windowData) {
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
            sendWindowFocus(windowData.id);
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
      }

      return {
        setIndexStatus,
        handleProjectIndexSearchResults,
        handleProjectIndexSearchError,
        mountProjectIndexSurface,
        indexSearchStateMap,
        pendingIndexOpenTargetsByPreset,
        indexStatusByProjectRoot,
      };
}
