// SPEC-3064 Phase 3 (E6b) — Branches window & branch cleanup surface
// extracted from app.js. Owns the per-window branch list state map, the
// branch rows / list rendering, the branch cleanup modal flow (selection,
// confirm/running/result stages, progress), the Workspace Overview
// cleanup entry (openWorkspaceCleanup + the synthetic
// __workspace_cleanup__ window id), the Branches window mount, and the
// branch_cleanup_* / branch_error receive() bodies. Pure movement from
// app.js: behavior, DOM output, and WS protocol are unchanged; the moved
// code keeps its original app.js indentation. Textual changes are limited
// to: in-module self-references through
// `*` became direct local calls,
// `workspaceOverviewSurface.renderWindows()` became the injected
// renderWorkspaceWindows dep (that surface is constructed after this
// factory), `activeWorkProjection` reads go through the injected
// getActiveWorkProjection accessor, and the mount's focus_window send goes
// through sendWindowFocus.
//
// deps:
// - send(message): forward a frontend event over the WebSocket bridge.
// - createNode(tag, className, textContent): shared DOM helper.
// - windowMap: workspace window element map owned by app.js.
// - focusWindowLocally(windowId) / sendWindowFocus(windowId): focus paths.
// - branchCleanupModal / branchCleanupDialog: modal chrome elements owned
//   by app.js (its Esc handler and backdrop click also reference them).
// - launchPending: shared Resume/Launch pending controller instance.
// - visibleBounds(): canvas bounds payload helper.
// - getActiveWorkProjection(): read accessor for the active Work
//   projection (app.js owns the let).
// - renderWorkspaceWindows(): late-bound Workspace Overview re-render.
import { renderBranchCleanupModal as renderBranchCleanupModalView } from "/branch-cleanup-modal.js";
import {
  markBranchDetailInterrupted,
  branchLoadStatusSummary,
} from "/branch-list-state.js";

export function createBranchesCleanupSurface({
  send,
  createNode,
  windowMap,
  focusWindowLocally,
  sendWindowFocus,
  branchCleanupModal,
  branchCleanupDialog,
  launchPending,
  visibleBounds,
  getActiveWorkProjection,
  renderWorkspaceWindows,
}) {
      const branchListStateMap = new Map();
      let branchCleanupWindowId = null;
      const WORKSPACE_CLEANUP_WINDOW_ID = "__workspace_cleanup__";

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

      function openWorkspaceCleanup(candidateOverride, sourceWindowId) {
        // The Workspace detail passes the selected row, the list header
        // passes ALL merged rows (user verification 2026-06-12: completed
        // local branches need a bulk cleanup path); without an override the
        // projection-level cleanup candidate is used. When the caller knows
        // its real window id the run rides the multi-branch
        // `run_branch_cleanup` machinery (per-branch progress + failure
        // reasons); the synthetic id keeps the legacy single-candidate wire.
        const overrides = Array.isArray(candidateOverride)
          ? candidateOverride
          : [candidateOverride];
        const candidates = (candidateOverride
          ? overrides
          : [getActiveWorkProjection()?.cleanup_candidate]
        ).filter((candidate) => candidate?.branch);
        if (candidates.length === 0) return;
        const cleanupWindowId = sourceWindowId || WORKSPACE_CLEANUP_WINDOW_ID;
        const state = ensureBranchListState(cleanupWindowId);
        state.entries = candidates.map((candidate) => workspaceCleanupEntry(candidate));
        state.cleanupSelected = new Set(candidates.map((candidate) => candidate.branch));
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
        branchCleanupWindowId = cleanupWindowId;
        renderBranchCleanupModal();
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
            // SPEC-2009 Phase 7 (FR-064..FR-067): detail-check reconnect state.
            lastHydratedByName: new Map(),
            lastLoadId: 0,
            detailCheckStale: false,
            needsResync: false,
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
          // SPEC-2359 W-17 (FR-398): guard double-clicks while the backend
          // materializes the resume; settled by the started ack / branch_error.
          if (!launchPending.begin(`branch:${branchName}`, "Resume")) {
            return;
          }
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

        // SPEC-2359 US-83 / FR-443 / FR-444: refine the remote-row launch
        // affordance from the backend-supplied eligibility. A fresh origin
        // branch becomes a "Start Work / Open" source (continue on the branch
        // itself); a protected base or non-origin remote is not offered. The
        // action reuses the existing open_launch_wizard message; the wizard
        // opens in continue-on-branch mode so launching materializes a worktree
        // tracking origin/<branch> without minting a new work/* branch.
        const startWorkEligibility = entry.start_work_eligibility;
        if (entry.scope === "remote" && startWorkEligibility === "start_work") {
          fields.launchButton.hidden = false;
          fields.launchButton.textContent = "Start Work";
          fields.launchButton.title =
            `Start work on ${entry.name} (creates a worktree tracking it)`;
          fields.launchButton.setAttribute(
            "aria-label",
            `Start work on ${entry.name}`,
          );
        } else if (entry.scope === "remote" && startWorkEligibility === "hidden") {
          fields.launchButton.hidden = true;
        } else if (entry.scope === "remote" && startWorkEligibility === "resume_existing") {
          // The branch already has a local counterpart / live Work — resuming is
          // the primary affordance, so don't mislabel the row as a fresh launch.
          fields.launchButton.hidden = false;
          fields.launchButton.textContent = "Resume";
          fields.launchButton.title = `Resume work on ${entry.name}`;
          fields.launchButton.setAttribute("aria-label", `Resume work on ${entry.name}`);
        } else {
          fields.launchButton.hidden = false;
          fields.launchButton.textContent = "Launch";
        }
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

      function cleanupInfoDetailText(cleanup) {
        if (cleanup.availability === "blocked") {
          return cleanupBlockedReasonText(cleanup.blocked_reason);
        }
        if (cleanup.risks?.length) {
          return cleanupRiskLabels(cleanup.risks).join(", ");
        }
        return cleanupMergeTargetText(cleanup.merge_target);
      }

      function cleanupDetailText(entry, state) {
        if (!entry.cleanup_ready) {
          // FR-065: keep the last verified detail visible while re-checking.
          if (entry.cleanup_stale && entry.cleanup) {
            const base = cleanupInfoDetailText(entry.cleanup);
            return base ? `${base} · re-checking` : "Re-checking cleanup safety";
          }
          return branchCleanupPendingText(state);
        }
        return cleanupInfoDetailText(entry.cleanup);
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
            case "protected_base":
              return "protected base (remote kept)";
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
        // FR-064/FR-065: keep the rows + last-known cleanup badges, flag the
        // window for auto re-hydration on reconnect, and stop the spinner —
        // never collapse every row to "Safety unknown" or a manual-refresh-only
        // banner. markBranchDetailInterrupted owns the state transition.
        const changed = markBranchDetailInterrupted(state);
        if (changed) {
          syncBranchSelectionState(state);
        }
        return changed;
      }

      function branchCleanupPendingText(state) {
        // FR-064/FR-066: the detail check now recovers automatically on
        // reconnect, so we never tell the user to manually "Refresh to verify".
        return state.loading ? "Checking cleanup safety" : "Re-checking cleanup safety";
      }

      function cleanupAvailabilityForRender(entry, state) {
        if (entry.cleanup_ready) {
          return entry.cleanup.availability;
        }
        // FR-065: show the last verified availability while a re-check is
        // pending instead of dropping to "unknown".
        if (entry.cleanup_stale && entry.cleanup) {
          return entry.cleanup.availability;
        }
        if (state.loading) {
          return "loading";
        }
        return "unknown";
      }

      function cleanupBadgeText(entry, state) {
        if (entry.cleanup_ready) {
          return entry.cleanup.availability;
        }
        // FR-065: keep showing the last verified badge during re-hydration.
        if (entry.cleanup_stale && entry.cleanup) {
          return entry.cleanup.availability;
        }
        return state.loading ? "checking" : "Safety unknown";
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

      function renderBranchCleanupOwner(windowId) {
        if (windowId === WORKSPACE_CLEANUP_WINDOW_ID) {
          renderBranchCleanupModal();
          renderWorkspaceWindows();
          return;
        }
        const element = windowMap.get(windowId);
        if (element?.querySelector(".branch-list")) {
          renderBranches(windowId);
          return;
        }
        renderBranchCleanupModal();
        renderWorkspaceWindows();
      }
      // SPEC-3064 Phase 3 (E6b): Branches window mount moved verbatim from
      // app.js mountWindowBody (surface === "branches" branch).
      function mountBranchesWindow(windowData, body) {
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
            sendWindowFocus(windowData.id);
          });
          body
            .querySelector("[data-action='refresh-branches']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              const state = ensureBranchListState(
                windowData.id,
              );
              state.error = "";
              state.notice = "";
              requestBranches(windowData.id);
              renderBranches(windowData.id);
            });
          for (const button of body.querySelectorAll("[data-branch-filter]")) {
            button.addEventListener("click", (event) => {
              event.stopPropagation();
              const state = ensureBranchListState(
                windowData.id,
              );
              state.filter = button.dataset.branchFilter;
              renderBranches(windowData.id);
            });
          }
          body
            .querySelector("[data-action='open-branch-cleanup']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              openBranchCleanupModal(
                windowData.id,
              );
            });
          const state = ensureBranchListState(
            windowData.id,
          );
          if (state.entries.length === 0 && !state.loading && !state.error) {
            requestBranches(windowData.id);
          }
          renderBranches(windowData.id);
          return;
      }

      // SPEC-3064 Phase 3 (E6b): window-cleanup hook — branchCleanupWindowId
      // is a module-level let now, so app.js delegates the reset.
      function clearBranchCleanupForWindow(windowId) {
        if (branchCleanupWindowId === windowId) {
          branchCleanupWindowId = null;
          renderBranchCleanupModal();
        }
      }

      // SPEC-3064 Phase 3 (E6b): receive() bodies for branch_cleanup_* /
      // branch_error moved verbatim from app.js; the case arms in app.js
      // delegate here. (branch_entries stays in app.js because it feeds the
      // operator telemetry helper that lives there.)
      function applyBranchCleanupReceiveEvent(event) {
        switch (event.kind) {
          case "branch_cleanup_result": {
            const state = ensureBranchListState(
              event.id,
            );
            state.cleanupSelected.clear();
            state.cleanupModal.open = true;
            state.cleanupModal.stage = "result";
            state.cleanupModal.results = event.results || [];
            branchCleanupWindowId = event.id;
            renderBranchCleanupOwner(event.id);
            break;
          }
          case "branch_cleanup_progress": {
            updateBranchCleanupProgress(
              event.id,
              event,
            );
            branchCleanupWindowId = event.id;
            renderBranchCleanupOwner(event.id);
            break;
          }
          case "branch_error": {
            // SPEC-2359 W-17 (FR-398): a failed branch resume must re-enable
            // its pending Resume control immediately (not via timeout).
            launchPending.settleWhere("branch:");
            const state = ensureBranchListState(
              event.id,
            );
            state.loading = false;
            if (state.cleanupModal.stage === "running") {
              failRunningBranchCleanup(event.id, event.message);
              renderBranchCleanupOwner(event.id);
              break;
            }
            if (state.receivedFreshEntries) {
              state.notice = event.message;
              state.error = "";
            } else {
              state.error = event.message;
            }
            renderBranches(event.id);
            break;
          }
          default:
            break;
        }
      }

      return {
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
      };
}
