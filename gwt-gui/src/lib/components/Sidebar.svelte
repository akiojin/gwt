<script lang="ts">
  import type {
    BranchInfo,
    WorktreeInfo,
    CleanupProgress,
    LaunchAgentRequest,
    PrStatusLite,
    GhCliStatus,
    SettingsData,
  } from "../types";
  import {
    applyPrStatusUpdate,
    buildFilterCacheKey as buildFilterCacheKeyHelper,
    buildWorktreeMap as buildWorktreeMapHelper,
    clampSidebarWidth as clampSidebarWidthHelper,
    decideRefreshFailureAction,
    divergenceClass as divergenceClassHelper,
    divergenceIndicator as divergenceIndicatorHelper,
    getSafetyLevel as getSafetyLevelHelper,
    getSafetyTitle as getSafetyTitleHelper,
    isBranchProtected as isBranchProtectedHelper,
    normalizeBranchForPrLookup as normalizeBranchForPrLookupHelper,
    normalizeTabBranch as normalizeTabBranchHelper,
    resolveEventListen,
    sortBranches as sortBranchesHelper,
    toErrorMessage as toErrorMessageHelper,
    toolUsageClass as toolUsageClassHelper,
    type SidebarEventListen,
  } from "./sidebarHelpers";
  import WorktreeSummaryPanel from "./WorktreeSummaryPanel.svelte";

  type FilterType = "Local" | "Remote" | "All";
  type BranchSortMode = "name" | "updated";
  type SidebarMode = "branch";
  type FilterCacheEntry = {
    branches: BranchInfo[];
    remoteBranchNames: Set<string>;
    worktreeMap: Map<string, WorktreeInfo>;
    cacheKey: string;
    fetchedAtMs: number;
    dirty: boolean;
  };
  type FetchSnapshotResult =
    | { ok: true; snapshot: FilterCacheEntry }
    | { ok: false; errorMessage: string };
  type TauriInvoke = <T>(
    command: string,
    args?: Record<string, unknown>,
  ) => Promise<T>;
  type TauriEventListen = SidebarEventListen;

  let {
    projectPath,
    onBranchSelect,
    onBranchActivate,
    onCleanupRequest,
    onLaunchAgent,
    onQuickLaunch,
    onNewTerminal,
    onOpenDocsEditor,
    onResize,
    onOpenCiLog,
    onDisplayNameChanged,
    widthPx = 260,
    minWidthPx = 220,
    maxWidthPx = 520,
    refreshKey = 0,
    mode = "branch",
    onModeChange,
    selectedBranch = null,
    currentBranch = "",
    agentTabBranches = [],
    activeAgentTabBranch = null,
    appLanguage = "auto",
    prStatuses = {},
    ghCliStatus = null,
  }: {
    projectPath: string;
    onBranchSelect: (branch: BranchInfo) => void;
    onBranchActivate?: (branch: BranchInfo) => void;
    onCleanupRequest?: (preSelectedBranch?: string) => void;
    onLaunchAgent?: () => void;
    onQuickLaunch?: (request: LaunchAgentRequest) => Promise<void>;
    onNewTerminal?: () => void;
    onOpenDocsEditor?: (worktreePath: string) => Promise<void> | void;
    onResize?: (nextWidthPx: number) => void;
    onOpenCiLog?: (runId: number) => void;
    onDisplayNameChanged?: () => void;
    widthPx?: number;
    minWidthPx?: number;
    maxWidthPx?: number;
    refreshKey?: number;
    mode?: SidebarMode;
    onModeChange?: (next: SidebarMode) => void;
    selectedBranch?: BranchInfo | null;
    currentBranch?: string;
    agentTabBranches?: string[];
    activeAgentTabBranch?: string | null;
    appLanguage?: SettingsData["app_language"];
    prStatuses?: Record<string, PrStatusLite | null>;
    ghCliStatus?: GhCliStatus | null;
  } = $props();

  const SIDEBAR_SUMMARY_HEIGHT_STORAGE_KEY =
    "gwt.sidebar.worktreeSummaryHeight";
  const DEFAULT_WORKTREE_SUMMARY_HEIGHT_PX = 360;
  const MIN_WORKTREE_SUMMARY_HEIGHT_PX = 160;
  const MIN_BRANCH_LIST_HEIGHT_PX = 120;
  const SUMMARY_RESIZE_HANDLE_HEIGHT_PX = 8;
  const FILTER_BACKGROUND_REFRESH_TTL_MS = 10_000;
  const SEARCH_FILTER_DEBOUNCE_MS = 120;
  const SELECTED_BRANCH_SCROLL_GAP_PX = 24;
  const SELECTED_BRANCH_SCROLL_PX_PER_SECOND = 36;
  const SELECTED_BRANCH_SCROLL_MIN_DURATION_MS = 5_000;

  // PR Polling — inline to avoid .svelte.ts import issues in tests
  const PR_POLL_INTERVAL_MS = 30_000;
  const PR_POLL_BACKOFF_MAX_MS = 120_000;
  const PR_POLL_VISIBILITY_REFRESH_MIN_GAP_MS = 5_000;
  let pollingStatuses: Record<string, PrStatusLite | null> = $state({});
  let pollingGhCliStatus: GhCliStatus | null = $state(null);
  let pollingRepoKey: string | null = $state(null);
  let prPollingBootstrappedPath: string | null = null;
  let prPollingActivePath: string | null = null;
  const prPollingInFlightPaths = new Set<string>();

  $effect(() => {
    const path = projectPath;
    const branchCount = branches.length;

    if (path !== prPollingActivePath) {
      prPollingActivePath = path || null;
      prPollingBootstrappedPath = null;
      pollingStatuses = {};
      pollingGhCliStatus = null;
      pollingRepoKey = null;
    }

    if (!path) {
      return;
    }
    if (mode !== "branch") {
      return;
    }

    let destroyed = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let pollIntervalMs = PR_POLL_INTERVAL_MS;
    let consecutiveFailures = 0;
    let lastRefreshAt = 0;

    function clearTimer() {
      if (timer === null) return;
      clearTimeout(timer);
      timer = null;
    }

    function updatePollingInterval(failed: boolean) {
      if (!failed) {
        consecutiveFailures = 0;
        pollIntervalMs = PR_POLL_INTERVAL_MS;
        return;
      }
      consecutiveFailures = Math.min(consecutiveFailures + 1, 2);
      pollIntervalMs = Math.min(
        PR_POLL_INTERVAL_MS * 2 ** consecutiveFailures,
        PR_POLL_BACKOFF_MAX_MS,
      );
    }

    function scheduleNextRefresh() {
      if (destroyed) return;
      if (timer !== null) return;
      if (branchCount === 0) return;
      timer = setTimeout(() => {
        timer = null;
        if (destroyed) return;
        if (isTextEntryFocused()) {
          scheduleNextRefresh();
          return;
        }
        void refresh();
      }, pollIntervalMs);
    }

    async function refresh(markBootstrap = false) {
      if (destroyed) return;
      if (prPollingInFlightPaths.has(path)) {
        scheduleNextRefresh();
        return;
      }
      lastRefreshAt = Date.now();
      prPollingInFlightPaths.add(path);
      let failed = false;
      try {
        const branchKeyByName = new Map<string, string>();
        const queryBranches: string[] = [];
        const seen = new Set<string>();
        for (const branch of branches) {
          const queryBranch = normalizeBranchForPrLookup(branch.name);
          branchKeyByName.set(branch.name, queryBranch);
          if (!queryBranch || seen.has(queryBranch)) continue;
          seen.add(queryBranch);
          queryBranches.push(queryBranch);
        }

        if (queryBranches.length === 0) {
          pollingStatuses = {};
          pollingRepoKey = null;
          return;
        }
        if (markBootstrap) {
          prPollingBootstrappedPath = path;
        }
        const invoke = await getInvoke();
        const result = await invoke<{
          statuses: Record<string, PrStatusLite | null>;
          ghStatus: GhCliStatus;
          repoKey?: string | null;
        }>("fetch_pr_status", { projectPath: path, branches: queryBranches });
        if (!destroyed) {
          const statuses = result.statuses ?? {};
          const mappedStatuses: Record<string, PrStatusLite | null> = {};
          for (const branch of branches) {
            const key = branchKeyByName.get(branch.name) ?? branch.name;
            mappedStatuses[branch.name] = statuses[key] ?? null;
          }
          pollingStatuses = mappedStatuses;
          pollingGhCliStatus = result.ghStatus ?? null;
          pollingRepoKey = result.repoKey ?? null;
        }
      } catch {
        // Polling failure is silent — keep stale data
        failed = true;
      } finally {
        prPollingInFlightPaths.delete(path);
        updatePollingInterval(failed);
        scheduleNextRefresh();
      }
    }

    function start() {
      if (destroyed) return;
      clearTimer();
      if (branchCount === 0) return;
      if (prPollingBootstrappedPath !== path) {
        void refresh(true);
        return;
      }
      scheduleNextRefresh();
    }

    function onVisibility() {
      if (document.hidden) {
        clearTimer();
      } else {
        start();
        if (prPollingBootstrappedPath !== path) return;
        if (isTextEntryFocused()) return;
        if (Date.now() - lastRefreshAt < PR_POLL_VISIBILITY_REFRESH_MIN_GAP_MS)
          return;
        clearTimer();
        void refresh(false);
      }
    }

    document.addEventListener("visibilitychange", onVisibility);
    start();

    return () => {
      destroyed = true;
      clearTimer();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  });

  // Listen to pr-status-updated events from Rust backend (T008)
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;

    (async () => {
      try {
        const listen = await getEventListen();
        const unlistenFn = await listen<{
          repoKey: string;
          branch: string;
          status: PrStatusLite;
        }>("pr-status-updated", (event) => {
          const { repoKey, branch: eventBranch, status } = event.payload;
          if (!pollingRepoKey || repoKey !== pollingRepoKey) return;
          if (status.retrying) return;
          const next = applyPrStatusUpdate(
            pollingStatuses,
            eventBranch,
            status,
          );
          if (next) pollingStatuses = next;
        });
        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch {
        /* Tauri not available */
      }
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  });

  // Effective values: prefer polling data, fall back to props
  let activePrStatuses = $derived.by(() => {
    if (pollingStatuses && Object.keys(pollingStatuses).length > 0) {
      return pollingStatuses;
    }
    return prStatuses;
  });
  let effectiveGhCliStatus = $derived.by(() => {
    if (pollingGhCliStatus) return pollingGhCliStatus;
    return ghCliStatus;
  });

  // Derived selected PR status/number for WorktreeSummaryPanel
  let selectedPrStatus = $derived.by(() => {
    if (!selectedBranch) return null;
    return activePrStatuses[selectedBranch.name] ?? null;
  });

  let selectedPrNumber = $derived.by(() => {
    return selectedPrStatus?.number ?? null;
  });

  function prBadgeClass(status: PrStatusLite): string {
    if (status.state === "MERGED") return "merged";
    if (status.state === "CLOSED") return "closed";
    const uiState = (status.mergeUiState ?? "").toLowerCase();
    if (uiState === "blocked") return "blocked";
    if (uiState === "conflicting") return "conflicting";
    if (
      status.retrying === true ||
      uiState === "checking" ||
      (!uiState && status.mergeable === "UNKNOWN")
    ) {
      return "checking";
    }
    if (status.mergeable === "CONFLICTING") {
      return "conflicting";
    }
    return "open";
  }

  function isTextEntryFocused(): boolean {
    if (typeof document === "undefined") return false;
    const active = document.activeElement;
    if (!active) return false;
    if (active instanceof HTMLInputElement) return true;
    if (active instanceof HTMLTextAreaElement) return true;
    if (active instanceof HTMLSelectElement) return true;
    return (active as HTMLElement).isContentEditable;
  }

  let activeFilter: FilterType = $state("Local");
  let branches: BranchInfo[] = $state([]);
  let remoteBranchNames: Set<string> = $state(new Set());
  let loading: boolean = $state(false);
  let searchInput: string = $state("");
  let searchQuery: string = $state("");
  let errorMessage: string | null = $state(null);
  let sortMode: BranchSortMode = $state("updated");
  let agentTabBranchSet = $derived.by(() => {
    const set = new Set<string>();
    for (const branchName of agentTabBranches) {
      const normalized = normalizeTabBranch(branchName);
      if (normalized) set.add(normalized);
    }
    return set;
  });
  let lastFetchKey = "";
  let lastForceKey = "";
  let lastProjectPath = "";
  let fetchToken = 0;
  let localRefreshKey = $state(0);
  let filterCache: Map<FilterType, FilterCacheEntry> = $state(new Map());
  const inflightFetches = new Map<string, Promise<FetchSnapshotResult>>();
  let tauriEventListenPromise: Promise<TauriEventListen> | null = null;

  // Worktree safety info
  let worktreeMap: Map<string, WorktreeInfo> = $state(new Map());

  function normalizeTabBranch(name: string): string {
    return normalizeTabBranchHelper(name);
  }

  function sortBranches(list: BranchInfo[], filter: FilterType): BranchInfo[] {
    return sortBranchesHelper(list, filter, remoteBranchNames, sortMode);
  }

  function toggleSortMode() {
    sortMode = sortMode === "name" ? "updated" : "name";
  }

  function getSortModeLabel(): string {
    return sortMode === "name" ? "Name" : "Updated";
  }

  function normalizeBranchForPrLookup(branchName: string): string {
    return normalizeBranchForPrLookupHelper(branchName, remoteBranchNames);
  }

  function isSelectedBranch(branch: BranchInfo): boolean {
    return (
      selectedBranch !== null &&
      selectedBranch !== undefined &&
      selectedBranch.name === branch.name
    );
  }

  function resetSelectedBranchAutoScroll() {
    selectedBranchAutoScrollEnabled = false;
    selectedBranchAutoScrollDistancePx = 0;
    selectedBranchAutoScrollDurationMs = SELECTED_BRANCH_SCROLL_MIN_DURATION_MS;
  }

  function cancelSelectedBranchAutoScrollMeasure() {
    if (selectedBranchMeasureTimer === null) return;
    clearTimeout(selectedBranchMeasureTimer);
    selectedBranchMeasureTimer = null;
  }

  function disconnectSelectedBranchResizeObserver() {
    if (selectedBranchResizeObserver === null) return;
    selectedBranchResizeObserver.disconnect();
    selectedBranchResizeObserver = null;
  }

  function applySelectedBranchAutoScrollMeasure() {
    if (
      selectedBranch === null ||
      selectedBranchNameViewportEl === null ||
      selectedBranchNameLabelEl === null
    ) {
      resetSelectedBranchAutoScroll();
      return;
    }

    const viewportWidth = Math.ceil(selectedBranchNameViewportEl.clientWidth);
    const labelWidth = Math.ceil(selectedBranchNameLabelEl.scrollWidth);
    if (
      !Number.isFinite(viewportWidth) ||
      !Number.isFinite(labelWidth) ||
      viewportWidth <= 0 ||
      labelWidth <= 0
    ) {
      resetSelectedBranchAutoScroll();
      return;
    }

    if (labelWidth <= viewportWidth + 1) {
      resetSelectedBranchAutoScroll();
      return;
    }

    const distancePx = Math.max(
      0,
      labelWidth - viewportWidth + SELECTED_BRANCH_SCROLL_GAP_PX
    );
    const durationMs = Math.max(
      SELECTED_BRANCH_SCROLL_MIN_DURATION_MS,
      Math.round((distancePx / SELECTED_BRANCH_SCROLL_PX_PER_SECOND) * 1000)
    );
    selectedBranchAutoScrollDistancePx = distancePx;
    selectedBranchAutoScrollDurationMs = durationMs;
    selectedBranchAutoScrollEnabled = true;
  }

  function queueSelectedBranchAutoScrollMeasure() {
    cancelSelectedBranchAutoScrollMeasure();
    selectedBranchMeasureTimer = setTimeout(() => {
      selectedBranchMeasureTimer = null;
      applySelectedBranchAutoScrollMeasure();
    }, 0);
  }

  function observeSelectedBranchAutoScrollTargets() {
    disconnectSelectedBranchResizeObserver();
    if (
      typeof ResizeObserver === "undefined" ||
      selectedBranchNameViewportEl === null ||
      selectedBranchNameLabelEl === null
    ) {
      return;
    }

    const observer = new ResizeObserver(() => {
      queueSelectedBranchAutoScrollMeasure();
    });
    observer.observe(selectedBranchNameViewportEl);
    observer.observe(selectedBranchNameLabelEl);
    selectedBranchResizeObserver = observer;
  }

  function selectedBranchViewportAction(
    node: HTMLSpanElement,
    branchName: string | null
  ) {
    function sync(nextBranchName: string | null) {
      if (nextBranchName !== null) {
        selectedBranchNameViewportEl = node;
        queueSelectedBranchAutoScrollMeasure();
        return;
      }
      if (selectedBranchNameViewportEl === node) {
        selectedBranchNameViewportEl = null;
      }
    }

    sync(branchName);

    return {
      update(nextBranchName: string | null) {
        sync(nextBranchName);
      },
      destroy() {
        if (selectedBranchNameViewportEl === node) {
          selectedBranchNameViewportEl = null;
        }
      },
    };
  }

  function selectedBranchLabelAction(
    node: HTMLSpanElement,
    branchName: string | null
  ) {
    function sync(nextBranchName: string | null) {
      if (nextBranchName !== null) {
        selectedBranchNameLabelEl = node;
        queueSelectedBranchAutoScrollMeasure();
        return;
      }
      if (selectedBranchNameLabelEl === node) {
        selectedBranchNameLabelEl = null;
      }
    }

    sync(branchName);

    return {
      update(nextBranchName: string | null) {
        sync(nextBranchName);
      },
      destroy() {
        if (selectedBranchNameLabelEl === node) {
          selectedBranchNameLabelEl = null;
        }
      },
    };
  }

  function isSelectedBranchAutoScrollActive(branch: BranchInfo): boolean {
    return isSelectedBranch(branch) && selectedBranchAutoScrollEnabled;
  }

  function selectedBranchAutoScrollStyle(branch: BranchInfo): string | undefined {
    if (!isSelectedBranchAutoScrollActive(branch)) return undefined;
    return [
      `--branch-scroll-distance: ${selectedBranchAutoScrollDistancePx}px`,
      `--branch-scroll-duration: ${selectedBranchAutoScrollDurationMs}ms`,
      `--branch-scroll-gap: ${SELECTED_BRANCH_SCROLL_GAP_PX}px`,
    ].join("; ");
  }

  function loadSummaryHeight(): number {
    if (typeof window === "undefined")
      return DEFAULT_WORKTREE_SUMMARY_HEIGHT_PX;
    try {
      const raw = window.localStorage.getItem(
        SIDEBAR_SUMMARY_HEIGHT_STORAGE_KEY,
      );
      if (!raw) return DEFAULT_WORKTREE_SUMMARY_HEIGHT_PX;
      const parsed = Number(raw);
      if (!Number.isFinite(parsed) || parsed <= 0) {
        return DEFAULT_WORKTREE_SUMMARY_HEIGHT_PX;
      }
      return Math.max(MIN_WORKTREE_SUMMARY_HEIGHT_PX, Math.round(parsed));
    } catch {
      return DEFAULT_WORKTREE_SUMMARY_HEIGHT_PX;
    }
  }

  function persistSummaryHeight(heightPx: number) {
    if (typeof window === "undefined") return;
    try {
      window.localStorage.setItem(
        SIDEBAR_SUMMARY_HEIGHT_STORAGE_KEY,
        String(Math.max(MIN_WORKTREE_SUMMARY_HEIGHT_PX, Math.round(heightPx))),
      );
    } catch {
      // Ignore localStorage failures.
    }
  }

  function clampSummaryHeight(nextHeightPx: number): number {
    const stackHeight = branchSummaryStackEl?.clientHeight ?? 0;
    const minSummaryHeight = Math.max(
      MIN_WORKTREE_SUMMARY_HEIGHT_PX,
      Math.round(nextHeightPx),
    );

    if (!Number.isFinite(stackHeight) || stackHeight <= 0) {
      return minSummaryHeight;
    }

    const availableSummaryHeight = Math.max(
      0,
      stackHeight - MIN_BRANCH_LIST_HEIGHT_PX - SUMMARY_RESIZE_HANDLE_HEIGHT_PX,
    );

    if (availableSummaryHeight < MIN_WORKTREE_SUMMARY_HEIGHT_PX) {
      return Math.round(availableSummaryHeight);
    }

    return Math.min(minSummaryHeight, Math.round(availableSummaryHeight));
  }

  function setSummaryHeight(nextHeightPx: number, persist = true) {
    const clamped = clampSummaryHeight(nextHeightPx);
    summaryHeightPx = clamped;
    if (persist) {
      persistSummaryHeight(clamped);
    }
  }

  // Branches currently being deleted
  let deletingBranches: Set<string> = $state(new Set());
  let branchSummaryStackEl: HTMLElement | null = $state(null);
  let branchListEl: HTMLDivElement | null = $state(null);
  let summaryHeightPx = $state(loadSummaryHeight());
  let summaryResizing = $state(false);
  let summaryResizePointerId: number | null = $state(null);
  let summaryResizeStartY = 0;
  let summaryResizeStartHeight = DEFAULT_WORKTREE_SUMMARY_HEIGHT_PX;
  let previousSummaryBodyCursor = "";
  let previousSummaryBodyUserSelect = "";

  // Context menu state
  let contextMenu: { x: number; y: number; branch: BranchInfo } | null =
    $state(null);

  // Inline rename state
  let renamingBranch: string | null = $state(null);
  let renameValue: string = $state("");
  let renameInputEl: HTMLInputElement | null = $state(null);

  let resizing = false;
  let resizePointerId: number | null = null;
  let resizeStartX = 0;
  let resizeStartWidth = 0;
  let previousBodyCursor = "";
  let previousBodyUserSelect = "";
  let selectedBranchNameViewportEl: HTMLSpanElement | null = $state(null);
  let selectedBranchNameLabelEl: HTMLSpanElement | null = $state(null);
  let selectedBranchAutoScrollEnabled = $state(false);
  let selectedBranchAutoScrollDistancePx = $state(0);
  let selectedBranchAutoScrollDurationMs = $state(
    SELECTED_BRANCH_SCROLL_MIN_DURATION_MS
  );
  let selectedBranchMeasureTimer: ReturnType<typeof setTimeout> | null = null;
  let selectedBranchResizeObserver: ResizeObserver | null = null;

  const filters: FilterType[] = ["Local", "Remote", "All"];

  let filteredBranches = $derived.by(() => {
    const sortedBranches = sortBranches(branches, activeFilter);
    if (!searchQuery) return sortedBranches;

    const normalizedQuery = searchQuery.toLowerCase();
    return sortedBranches.filter(
      (b) =>
        b.name.toLowerCase().includes(normalizedQuery) ||
        (b.display_name &&
          b.display_name.toLowerCase().includes(normalizedQuery)),
    );
  });
  let selectedBranchIndex = $derived.by(() => {
    if (selectedBranch === null || filteredBranches.length === 0) return -1;
    return filteredBranches.findIndex(
      (branch) => branch.name === selectedBranch!.name,
    );
  });
  let clampedWidthPx = $derived(clampSidebarWidth(widthPx));

  $effect(() => {
    const nextQuery = searchInput;
    const timer = setTimeout(() => {
      searchQuery = nextQuery;
    }, SEARCH_FILTER_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    if (!branchSummaryStackEl) return;
    setSummaryHeight(summaryHeightPx, false);
  });

  $effect(() => {
    const selectedBranchName = selectedBranch?.name ?? "";
    const selectedBranchDisplayName =
      selectedBranch?.display_name ?? selectedBranch?.name ?? "";
    void clampedWidthPx;
    void selectedBranchName;
    void selectedBranchDisplayName;

    if (
      selectedBranchName === "" ||
      selectedBranchNameViewportEl === null ||
      selectedBranchNameLabelEl === null
    ) {
      disconnectSelectedBranchResizeObserver();
      cancelSelectedBranchAutoScrollMeasure();
      resetSelectedBranchAutoScroll();
      return;
    }

    observeSelectedBranchAutoScrollTargets();
    queueSelectedBranchAutoScrollMeasure();

    return () => {
      disconnectSelectedBranchResizeObserver();
      cancelSelectedBranchAutoScrollMeasure();
    };
  });

  $effect(() => {
    return () => {
      stopResizing();
    };
  });

  $effect(() => {
    // Re-fetch when filter changes. Force refresh keys also trigger all-filter background updates.
    if (mode !== "branch") {
      lastFetchKey = "";
      lastForceKey = "";
      lastProjectPath = "";
      return;
    }
    const forceKey = `${projectPath}::${refreshKey}::${localRefreshKey}`;
    const key = `${forceKey}::${activeFilter}`;
    if (key === lastFetchKey) return;

    const isInitialRun = lastForceKey === "";
    const projectChanged =
      lastProjectPath !== "" && projectPath !== lastProjectPath;
    const forceRefreshTriggered = !isInitialRun && forceKey !== lastForceKey;

    lastFetchKey = key;
    lastForceKey = forceKey;
    lastProjectPath = projectPath;

    const token = ++fetchToken;
    if (projectChanged) {
      clearFilterCache();
      fetchBranches(token);
      return;
    }
    if (forceRefreshTriggered) {
      markAllFilterCachesDirty();
      refreshAllFilterCaches(token);
      return;
    }
    fetchBranches(token);
  });

  $effect(() => {
    if (mode === "branch") return;
    contextMenu = null;
  });

  function handleModeChange(next: SidebarMode) {
    if (mode === next) return;
    onModeChange?.(next);
  }

  // Listen to cleanup-progress events for deletion state tracking
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;

    (async () => {
      try {
        const listen = await getEventListen();
        const unlistenFn = await listen<CleanupProgress>(
          "cleanup-progress",
          (event) => {
            const { branch, status } = event.payload;
            if (status === "deleting") {
              deletingBranches = new Set([...deletingBranches, branch]);
            } else {
              const next = new Set(deletingBranches);
              next.delete(branch);
              deletingBranches = next;
            }
          },
        );
        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch {
        /* Tauri not available */
      }
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  });

  // Listen to cleanup-completed to clear state and refresh
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;

    (async () => {
      try {
        const listen = await getEventListen();
        const unlistenFn = await listen("cleanup-completed", () => {
          deletingBranches = new Set();
          localRefreshKey++;
        });
        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch {
        /* Tauri not available */
      }
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  });

  // Listen to agent-status-changed to refresh branch list (gwt-spec issue FR-821)
  $effect(() => {
    let unlisten: null | (() => void) = null;
    let cancelled = false;

    (async () => {
      try {
        const listen = await getEventListen();
        const unlistenFn = await listen("agent-status-changed", () => {
          refreshAgentStatusCachesInBackground();
        });
        if (cancelled) {
          unlistenFn();
          return;
        }
        unlisten = unlistenFn;
      } catch {
        /* Tauri not available */
      }
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  });

  // Polling fallback for agent status (gwt-spec issue FR-822)
  // Only active when there are agent tabs open (avoids unnecessary polling).
  $effect(() => {
    if (mode !== "branch") return;
    if (agentTabBranches.length === 0) return;
    const interval = setInterval(() => {
      refreshAgentStatusCachesInBackground();
    }, 10_000);
    return () => clearInterval(interval);
  });

  // Close context menu on outside click / Escape
  $effect(() => {
    if (!contextMenu) return;
    const handleClick = () => {
      contextMenu = null;
    };
    const handleKeydown = (e: KeyboardEvent) => {
      if (e.key === "Escape") contextMenu = null;
    };
    // Defer so the opening right-click doesn't immediately close it
    const id = setTimeout(() => {
      document.addEventListener("click", handleClick);
      document.addEventListener("keydown", handleKeydown);
    }, 0);
    return () => {
      clearTimeout(id);
      document.removeEventListener("click", handleClick);
      document.removeEventListener("keydown", handleKeydown);
    };
  });

  function fetchBranches(token: number, forceRefresh = false) {
    const filter = activeFilter;
    const path = projectPath;
    const cacheKey = buildFilterCacheKey(filter, path);
    const cached = filterCache.get(filter);

    if (cached && cached.cacheKey === cacheKey) {
      applyCacheEntry(cached);
      const ttlElapsed = Date.now() - cached.fetchedAtMs;
      const shouldRefresh =
        forceRefresh ||
        cached.dirty ||
        ttlElapsed >= FILTER_BACKGROUND_REFRESH_TTL_MS;
      if (!shouldRefresh) return;
      void refreshFilterSnapshot(filter, path, cacheKey, token, true, true);
      return;
    }

    loading = true;
    errorMessage = null;
    void refreshFilterSnapshot(filter, path, cacheKey, token, false, true);
  }

  function refreshAllFilterCaches(token: number) {
    fetchBranches(token, true);
    const path = projectPath;
    for (const filter of filters) {
      if (filter === activeFilter) continue;
      const cacheKey = buildFilterCacheKey(filter, path);
      void refreshFilterSnapshot(filter, path, cacheKey, token, true, false);
    }
  }

  function refreshAgentStatusCachesInBackground() {
    if (mode !== "branch") return;
    const path = projectPath;
    if (!path) return;
    const token = fetchToken;
    const targetFilters: FilterType[] = ["Local", "All"];

    for (const filter of targetFilters) {
      const cacheKey = buildFilterCacheKey(filter, path);
      const cached = filterCache.get(filter);
      if (!cached || cached.cacheKey !== cacheKey) continue;
      void refreshFilterSnapshot(
        filter,
        path,
        cacheKey,
        token,
        true,
        filter === activeFilter,
      );
    }
  }

  async function refreshFilterSnapshot(
    filter: FilterType,
    path: string,
    cacheKey: string,
    token: number,
    background: boolean,
    applyToActiveView: boolean,
  ) {
    const hadFallbackCache = !!(filterCache.get(filter)?.cacheKey === cacheKey);
    const result = await loadFilterSnapshot(filter, path, cacheKey);

    if (token !== fetchToken) return;
    if (path !== projectPath) return;
    if (cacheKey !== buildFilterCacheKey(filter, path)) return;

    if (result.ok) {
      setFilterCacheEntry(filter, result.snapshot);
      if (applyToActiveView && filter === activeFilter) {
        applyCacheEntry(result.snapshot);
      }
      return;
    }

    const failureAction = decideRefreshFailureAction(
      background,
      hadFallbackCache,
      applyToActiveView,
      filter === activeFilter,
    );
    if (failureAction === "ignore") {
      return;
    }
    if (failureAction === "clear-loading") {
      loading = false;
      return;
    }

    errorMessage = result.errorMessage;
    branches = [];
    remoteBranchNames = new Set();
    worktreeMap = new Map();
    loading = false;
  }

  function loadFilterSnapshot(
    filter: FilterType,
    path: string,
    cacheKey: string,
  ): Promise<FetchSnapshotResult> {
    const inflightKey = `${filter}::${cacheKey}`;
    const inflight = inflightFetches.get(inflightKey);
    if (inflight) return inflight;

    const promise = fetchFilterSnapshot(filter, path, cacheKey).finally(() => {
      inflightFetches.delete(inflightKey);
    });
    inflightFetches.set(inflightKey, promise);
    return promise;
  }

  async function fetchFilterSnapshot(
    filter: FilterType,
    path: string,
    cacheKey: string,
  ): Promise<FetchSnapshotResult> {
    try {
      const invoke = await getInvoke();

      if (filter === "Local") {
        const next = await invoke<BranchInfo[]>("list_worktree_branches", {
          projectPath: path,
        });
        const worktrees = await invoke<WorktreeInfo[]>("list_worktrees", {
          projectPath: path,
        }).catch(() => [] as WorktreeInfo[]);
        return {
          ok: true,
          snapshot: {
            branches: next,
            remoteBranchNames: new Set(),
            worktreeMap: buildWorktreeMap(worktrees),
            cacheKey,
            fetchedAtMs: Date.now(),
            dirty: false,
          },
        };
      }

      if (filter === "Remote") {
        const next = await invoke<BranchInfo[]>("list_remote_branches", {
          projectPath: path,
        });
        return {
          ok: true,
          snapshot: {
            branches: next,
            remoteBranchNames: new Set(
              next.map((branch) => branch.name.trim()),
            ),
            worktreeMap: new Map(),
            cacheKey,
            fetchedAtMs: Date.now(),
            dirty: false,
          },
        };
      }

      const [local, remote] = await Promise.all([
        invoke<BranchInfo[]>("list_worktree_branches", { projectPath: path }),
        invoke<BranchInfo[]>("list_remote_branches", { projectPath: path }),
      ]);

      const seen = new Set<string>();
      const merged: BranchInfo[] = [];
      for (const branch of local) {
        seen.add(branch.name);
        merged.push(branch);
      }
      for (const branch of remote) {
        if (!seen.has(branch.name)) {
          merged.push(branch);
        }
      }

      const worktrees = await invoke<WorktreeInfo[]>("list_worktrees", {
        projectPath: path,
      }).catch(() => [] as WorktreeInfo[]);

      return {
        ok: true,
        snapshot: {
          branches: merged,
          remoteBranchNames: new Set(
            remote.map((branch) => branch.name.trim()),
          ),
          worktreeMap: buildWorktreeMap(worktrees),
          cacheKey,
          fetchedAtMs: Date.now(),
          dirty: false,
        },
      };
    } catch (err) {
      return {
        ok: false,
        errorMessage: `Failed to fetch branches: ${toErrorMessage(err)}`,
      };
    }
  }

  function buildFilterCacheKey(filter: FilterType, path: string): string {
    return buildFilterCacheKeyHelper(filter, path, refreshKey, localRefreshKey);
  }

  function buildWorktreeMap(
    worktrees: WorktreeInfo[],
  ): Map<string, WorktreeInfo> {
    return buildWorktreeMapHelper(worktrees);
  }

  function applyCacheEntry(entry: FilterCacheEntry) {
    branches = [...entry.branches];
    remoteBranchNames = new Set(entry.remoteBranchNames);
    worktreeMap = new Map(entry.worktreeMap);
    loading = false;
    errorMessage = null;
  }

  function setFilterCacheEntry(filter: FilterType, entry: FilterCacheEntry) {
    const next = new Map(filterCache);
    next.set(filter, entry);
    filterCache = next;
  }

  function markAllFilterCachesDirty() {
    if (filterCache.size === 0) return;
    const next = new Map<FilterType, FilterCacheEntry>();
    for (const [filter, entry] of filterCache) {
      next.set(filter, { ...entry, dirty: true });
    }
    filterCache = next;
  }

  function clearFilterCache() {
    filterCache = new Map();
    inflightFetches.clear();
  }

  function toErrorMessage(err: unknown): string {
    return toErrorMessageHelper(err);
  }

  async function getInvoke(): Promise<TauriInvoke> {
    const { invoke } = await import("$lib/tauriInvoke");
    return invoke as TauriInvoke;
  }

  async function getEventListen(): Promise<TauriEventListen> {
    if (!tauriEventListenPromise) {
      tauriEventListenPromise = import("@tauri-apps/api/event").then((mod) =>
        resolveEventListen(
          mod as {
            listen?: TauriEventListen;
            default?: { listen?: TauriEventListen };
          },
        ),
      );
    }

    try {
      return await tauriEventListenPromise;
    } catch (error) {
      tauriEventListenPromise = null;
      throw error;
    }
  }

  function getSafetyLevel(branch: BranchInfo): string {
    return getSafetyLevelHelper(branch, worktreeMap);
  }

  function getSafetyTitle(branch: BranchInfo): string {
    return getSafetyTitleHelper(branch, worktreeMap);
  }

  function isBranchProtected(branch: BranchInfo): boolean {
    return isBranchProtectedHelper(branch, worktreeMap);
  }

  function divergenceIndicator(branch: BranchInfo): string {
    return divergenceIndicatorHelper(branch);
  }

  function divergenceClass(status: string): string {
    return divergenceClassHelper(status);
  }

  function toolUsageClass(usage: string | null | undefined): string {
    return toolUsageClassHelper(usage);
  }

  // --- Context menu ---

  function clampSidebarWidth(width: number): number {
    return clampSidebarWidthHelper(width, minWidthPx, maxWidthPx);
  }

  function emitSidebarWidth(nextWidthPx: number) {
    onResize?.(clampSidebarWidth(nextWidthPx));
  }

  function stopResizing() {
    if (!resizing) return;
    resizing = false;
    resizePointerId = null;
    window.removeEventListener("pointermove", handleResizePointerMove);
    window.removeEventListener("pointerup", handleResizePointerUp);
    window.removeEventListener("pointercancel", handleResizePointerUp);
    document.body.style.cursor = previousBodyCursor;
    document.body.style.userSelect = previousBodyUserSelect;
  }

  function stopSummaryResize() {
    if (!summaryResizing) return;
    summaryResizing = false;
    summaryResizePointerId = null;
    window.removeEventListener("pointermove", handleSummaryResizePointerMove);
    window.removeEventListener("pointerup", handleSummaryResizePointerUp);
    window.removeEventListener("pointercancel", handleSummaryResizePointerUp);
    document.body.style.cursor = previousSummaryBodyCursor;
    document.body.style.userSelect = previousSummaryBodyUserSelect;
    persistSummaryHeight(summaryHeightPx);
  }

  $effect(() => {
    if (typeof window === "undefined") return;

    const handleResize = () => {
      setSummaryHeight(summaryHeightPx, false);
    };
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  });

  function handleResizePointerMove(event: PointerEvent) {
    if (!resizing) return;
    if (resizePointerId !== null && event.pointerId !== resizePointerId) return;
    const delta = event.clientX - resizeStartX;
    emitSidebarWidth(resizeStartWidth + delta);
  }

  function handleResizePointerUp(event: PointerEvent) {
    if (resizePointerId !== null && event.pointerId !== resizePointerId) return;
    stopResizing();
  }

  function handleResizePointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();

    resizeStartX = event.clientX;
    resizeStartWidth = clampedWidthPx;
    resizePointerId = event.pointerId;
    resizing = true;

    previousBodyCursor = document.body.style.cursor;
    previousBodyUserSelect = document.body.style.userSelect;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";

    window.addEventListener("pointermove", handleResizePointerMove);
    window.addEventListener("pointerup", handleResizePointerUp);
    window.addEventListener("pointercancel", handleResizePointerUp);
  }

  function handleSummaryResizePointerMove(event: PointerEvent) {
    if (!summaryResizing) return;
    if (
      summaryResizePointerId !== null &&
      event.pointerId !== summaryResizePointerId
    )
      return;
    const delta = event.clientY - summaryResizeStartY;
    setSummaryHeight(summaryResizeStartHeight - delta);
  }

  function handleSummaryResizePointerUp(event: PointerEvent) {
    if (
      summaryResizePointerId !== null &&
      event.pointerId !== summaryResizePointerId
    )
      return;
    stopSummaryResize();
  }

  function handleSummaryResizePointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();

    summaryResizing = true;
    summaryResizeStartY = event.clientY;
    summaryResizeStartHeight = summaryHeightPx;
    summaryResizePointerId = event.pointerId;

    previousSummaryBodyCursor = document.body.style.cursor;
    previousSummaryBodyUserSelect = document.body.style.userSelect;
    document.body.style.cursor = "row-resize";
    document.body.style.userSelect = "none";

    window.addEventListener("pointermove", handleSummaryResizePointerMove);
    window.addEventListener("pointerup", handleSummaryResizePointerUp);
    window.addEventListener("pointercancel", handleSummaryResizePointerUp);
  }

  function handleSummaryResizeKeydown(event: KeyboardEvent) {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    const step = event.shiftKey ? 32 : 16;
    const delta = event.key === "ArrowDown" ? -step : step;
    setSummaryHeight(summaryHeightPx + delta);
  }

  function focusBranchButtonByIndex(index: number) {
    queueMicrotask(() => {
      const button = branchListEl?.querySelector<HTMLButtonElement>(
        `[data-branch-index="${index}"]`,
      );
      if (!button) return;
      button.focus();
      if (typeof button.scrollIntoView === "function") {
        button.scrollIntoView({ block: "nearest" });
      }
    });
  }

  function isAgentBranchActive(branch: BranchInfo): boolean {
    if (activeFilter === "Remote") return false;
    if (activeFilter === "All" && remoteBranchNames.has(branch.name))
      return false;
    return agentTabBranchSet.has(normalizeTabBranch(branch.name));
  }

  function isAgentRunning(branch: BranchInfo): boolean {
    return isAgentBranchActive(branch) && branch.agent_status === "running";
  }

  function handleBranchItemKeydown(event: KeyboardEvent) {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    if (filteredBranches.length === 0) return;

    const focusedBranchIndex = Array.from(
      branchListEl?.querySelectorAll<HTMLButtonElement>(".branch-item") ?? [],
    ).findIndex((el) => el === document.activeElement);
    const currentIndex =
      selectedBranchIndex >= 0
        ? selectedBranchIndex
        : focusedBranchIndex >= 0
          ? focusedBranchIndex
          : -1;
    const maxIndex = filteredBranches.length - 1;
    const nextIndex =
      event.key === "ArrowDown"
        ? Math.min(maxIndex, currentIndex + 1)
        : Math.max(0, currentIndex - 1);

    if (nextIndex === currentIndex) return;

    const nextBranch = filteredBranches[nextIndex];
    onBranchSelect(nextBranch);
    focusBranchButtonByIndex(nextIndex);
  }

  $effect(() => {
    const list = branchListEl;
    if (!list || filteredBranches.length === 0) return;

    const listener = (event: KeyboardEvent) => {
      if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
      const target = event.target;
      if (!(target instanceof Element)) return;
      if (target !== list && !list.contains(target)) return;
      handleBranchItemKeydown(event);
    };

    window.addEventListener("keydown", listener, true);
    return () => {
      window.removeEventListener("keydown", listener, true);
    };
  });

  $effect(() => {
    return () => {
      stopSummaryResize();
    };
  });

  function handleResizeKeydown(event: KeyboardEvent) {
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      const step = event.shiftKey ? 24 : 12;
      const delta = event.key === "ArrowRight" ? step : -step;
      emitSidebarWidth(clampedWidthPx + delta);
      return;
    }
    if (event.key === "Home") {
      event.preventDefault();
      emitSidebarWidth(minWidthPx);
      return;
    }
    if (event.key === "End") {
      event.preventDefault();
      emitSidebarWidth(maxWidthPx);
    }
  }

  function canLaunchBranch(branch: BranchInfo): boolean {
    return !deletingBranches.has(branch.name) && !!onBranchActivate;
  }

  function handleContextMenu(e: MouseEvent, branch: BranchInfo) {
    if (deletingBranches.has(branch.name)) return;
    e.preventDefault();
    contextMenu = { x: e.clientX, y: e.clientY, branch };
  }

  function handleLaunchAgent() {
    if (!contextMenu) return;
    const branch = contextMenu.branch;
    contextMenu = null;
    if (!canLaunchBranch(branch)) return;
    onBranchActivate?.(branch);
  }

  function handleCleanupThisBranch() {
    if (!contextMenu) return;
    const branchName = contextMenu.branch.name;
    contextMenu = null;
    onCleanupRequest?.(branchName);
  }

  function handleCleanupWorktrees() {
    if (!contextMenu) return;
    const branchName = contextMenu.branch.name;
    contextMenu = null;
    onCleanupRequest?.(branchName);
  }

  function handleRenameBranch() {
    if (!contextMenu) return;
    const branch = contextMenu.branch;
    contextMenu = null;
    renamingBranch = branch.name;
    renameValue = branch.display_name ?? branch.name;
  }

  async function commitRename() {
    if (!renamingBranch) return;
    const branchName = renamingBranch;
    const newName = renameValue.trim();
    renamingBranch = null;
    renameValue = "";

    // If the user entered the branch name itself, treat as clearing display_name
    const displayName = newName === branchName ? "" : newName;

    try {
      const invoke = await getInvoke();
      await invoke("set_branch_display_name", {
        projectPath: projectPath,
        branch: branchName,
        displayName: displayName,
      });
      // Refresh branch list
      fetchBranches(fetchToken, true);
      onDisplayNameChanged?.();
    } catch (err) {
      console.error("Failed to set display name:", err);
    }
  }

  function cancelRename() {
    renamingBranch = null;
    renameValue = "";
  }

  $effect(() => {
    if (!renamingBranch || !renameInputEl) return;
    queueMicrotask(() => {
      if (!renamingBranch || !renameInputEl) return;
      renameInputEl.focus();
      renameInputEl.select();
    });
  });
</script>

<aside
  class="sidebar"
  style="width: {clampedWidthPx}px; min-width: {clampedWidthPx}px;"
>
  <div class="mode-toggle">
    <button
      class="mode-btn"
      class:active={mode === "branch"}
      aria-pressed={mode === "branch"}
      title="Branch Mode"
      onclick={() => handleModeChange("branch")}
    >
      <span class="mode-icon">B</span>
      <span class="mode-label">Branch</span>
    </button>
  </div>
  {#if mode === "branch"}
    <div class="filter-bar">
      {#each filters as filter}
        <button
          class="filter-btn"
          class:active={activeFilter === filter}
          onclick={() => (activeFilter = filter)}
        >
          {filter}
        </button>
      {/each}
      <button
        class="cleanup-btn"
        onclick={() => onCleanupRequest?.()}
        title="Cleanup Worktrees..."
      >
        Cleanup
      </button>
    </div>
    <div class="search-bar">
      <input
        type="text"
        autocapitalize="off"
        autocorrect="off"
        autocomplete="off"
        spellcheck="false"
        class="search-input"
        placeholder="Filter branches..."
        bind:value={searchInput}
      />
      <button
        type="button"
        class="sort-mode-toggle"
        aria-label="Sort mode"
        title={`Sort by ${getSortModeLabel()}`}
        onclick={toggleSortMode}
      >
        <span class="sort-mode-icon" aria-hidden="true">
          {sortMode === "name" ? "A↕" : "🕒"}
        </span>
        <span class="sort-mode-text">{getSortModeLabel()}</span>
      </button>
    </div>
    <div class="branch-summary-stack" bind:this={branchSummaryStackEl}>
      <div
        class="branch-list"
        bind:this={branchListEl}
        tabindex={filteredBranches.length === 0 ? -1 : 0}
        role="listbox"
        aria-label="Worktree branches"
      >
        {#if loading}
          <div class="loading-indicator">Loading...</div>
        {:else if errorMessage}
          <div class="error-indicator">{errorMessage}</div>
        {:else if filteredBranches.length === 0}
          <div class="empty-indicator">No branches found.</div>
        {:else}
          {#each filteredBranches as branch, index}
            <button
              data-branch-index={index}
              data-branch-name={branch.name}
              class="branch-item"
              class:agent-active={isAgentBranchActive(branch)}
              class:active={isSelectedBranch(branch)}
              class:deleting={deletingBranches.has(branch.name)}
              onclick={() => {
                if (!deletingBranches.has(branch.name)) onBranchSelect(branch);
              }}
              ondblclick={() => {
                if (!deletingBranches.has(branch.name))
                  onBranchActivate?.(branch);
              }}
              oncontextmenu={(e) => handleContextMenu(e, branch)}
            >
              <span
                class="agent-indicator-slot"
                class:agent-active={isAgentBranchActive(branch)}
                class:agent-running={isAgentRunning(branch)}
                aria-hidden="true"
                title={isAgentBranchActive(branch)
                  ? isAgentRunning(branch)
                    ? "Agent is running"
                    : "Agent tab is open"
                  : ""}
              >
                {#if isAgentRunning(branch)}
                  <span class="agent-pulse-dot"></span>
                  <span class="agent-fallback">@</span>
                {:else if isAgentBranchActive(branch)}
                  <span class="agent-static-dot"></span>
                  <span class="agent-fallback">@</span>
                {/if}
              </span>
              <span class="branch-icon">{branch.is_current ? "*" : " "}</span>
              {#if deletingBranches.has(branch.name)}
                <span class="safety-spinner"></span>
              {:else if getSafetyLevel(branch)}
                <span
                  class="safety-dot {getSafetyLevel(branch)}"
                  title={getSafetyTitle(branch)}
                ></span>
              {/if}
              {#if renamingBranch === branch.name}
                <input
                  bind:this={renameInputEl}
                  class="branch-rename-input"
                  type="text"
                  bind:value={renameValue}
                  onblur={commitRename}
                  onkeydown={(e) => {
                    if (e.key === "Enter") commitRename();
                    if (e.key === "Escape") cancelRename();
                  }}
                  onclick={(e) => e.stopPropagation()}
                />
              {:else}
                <span
                  class="branch-name"
                  class:auto-scroll={isSelectedBranchAutoScrollActive(branch)}
                  title={branch.display_name ? branch.name : undefined}
                  style={selectedBranchAutoScrollStyle(branch)}
                  use:selectedBranchViewportAction={isSelectedBranch(branch)
                    ? branch.name
                    : null}
                >
                  <span
                    class="branch-name-label"
                    use:selectedBranchLabelAction={isSelectedBranch(branch)
                      ? branch.name
                      : null}
                  >
                    {branch.display_name ?? branch.name}
                  </span>
                </span>
              {/if}
              {#if branch.last_tool_usage}
                <span
                  class="tool-usage {toolUsageClass(branch.last_tool_usage)}"
                >
                  {branch.last_tool_usage}
                </span>
              {/if}
              {#if divergenceIndicator(branch)}
                <span
                  class="divergence {divergenceClass(branch.divergence_status)}"
                >
                  {divergenceIndicator(branch)}
                </span>
              {/if}
              {#if activePrStatuses[branch.name]}
                {@const prSt = activePrStatuses[branch.name]!}
                <span
                  class="pr-badge {prBadgeClass(prSt)}{prSt.retrying
                    ? ' pulse'
                    : ''}"
                  title="PR #{prSt.number}"
                >
                  #{prSt.number}
                </span>
              {/if}
            </button>
          {/each}
        {/if}
      </div>
      <button
        type="button"
        class="summary-resize-handle"
        aria-label="Resize session summary"
        title="Resize session summary"
        onpointerdown={handleSummaryResizePointerDown}
        onkeydown={handleSummaryResizeKeydown}
      ></button>
      <div class="worktree-summary-wrap" style="height: {summaryHeightPx}px;">
        <WorktreeSummaryPanel
          {projectPath}
          {selectedBranch}
          {agentTabBranches}
          {activeAgentTabBranch}
          preferredLanguage={appLanguage}
          prNumber={selectedPrNumber}
          {selectedPrStatus}
          ghCliStatus={effectiveGhCliStatus}
          {onLaunchAgent}
          {onQuickLaunch}
          {onNewTerminal}
          {onOpenDocsEditor}
          {onOpenCiLog}
          onDisplayNameChanged={() => {
            fetchBranches(fetchToken, true);
            onDisplayNameChanged?.();
          }}
        />
      </div>
    </div>
  {:else}
    <div class="project-mode-sidebar">
      <div class="project-mode-sidebar-title">Assistant</div>
      <div class="project-mode-sidebar-body">
        Open the <code>Assistant</code> tab in the main area and send your first instruction.
      </div>
    </div>
  {/if}
  <button
    type="button"
    class="resize-handle"
    aria-label="Resize sidebar"
    onpointerdown={handleResizePointerDown}
    onkeydown={handleResizeKeydown}
  ></button>
</aside>

<!-- Context menu (fixed position, outside sidebar overflow) -->
{#if mode === "branch"}
  {#if contextMenu}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="context-menu"
      style="left: {contextMenu.x}px; top: {contextMenu.y}px;"
      onclick={(e) => e.stopPropagation()}
    >
      <button class="context-menu-item" onclick={handleRenameBranch}>
        Rename
      </button>
      <button
        class="context-menu-item"
        class:disabled={!canLaunchBranch(contextMenu.branch)}
        disabled={!canLaunchBranch(contextMenu.branch)}
        onclick={handleLaunchAgent}
      >
        Launch Agent...
      </button>
      <button
        class="context-menu-item"
        class:disabled={isBranchProtected(contextMenu.branch)}
        onclick={() => {
          if (contextMenu && !isBranchProtected(contextMenu.branch))
            handleCleanupThisBranch();
        }}
      >
        Cleanup this branch
      </button>
      <button class="context-menu-item" onclick={handleCleanupWorktrees}>
        Cleanup Worktrees...
      </button>
    </div>
  {/if}
{/if}

<style>
  .sidebar {
    position: relative;
    flex-shrink: 0;
    background-color: var(--bg-secondary);
    border-right: 1px solid var(--border-color);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .resize-handle {
    position: absolute;
    top: 0;
    right: 0;
    width: 8px;
    height: 100%;
    border: none;
    background: transparent;
    cursor: col-resize;
    padding: 0;
    touch-action: none;
    z-index: 1;
  }

  .resize-handle::after {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: 3px;
    width: 2px;
    background: transparent;
    transition: background-color 120ms ease;
  }

  .sidebar:hover .resize-handle::after,
  .resize-handle:focus-visible::after {
    background: var(--border-color);
  }

  .resize-handle:focus-visible {
    outline: none;
  }

  .filter-bar {
    display: flex;
    padding: 8px;
    gap: 4px;
    border-bottom: 1px solid var(--border-color);
  }

  .mode-toggle {
    display: flex;
    gap: 6px;
    padding: 10px 8px 8px;
    border-bottom: 1px solid var(--border-color);
  }

  .mode-btn {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    background: none;
    border: 1px solid var(--border-color);
    color: var(--text-secondary);
    padding: 6px 8px;
    font-size: var(--ui-font-sm);
    cursor: pointer;
    border-radius: 6px;
    font-family: inherit;
  }

  .project-mode-sidebar {
    margin: 12px;
    padding: 12px;
    border: 1px solid var(--border-color);
    border-radius: 8px;
    background: var(--bg-surface);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .project-mode-sidebar-title {
    font-size: var(--ui-font-md);
    font-weight: 600;
    color: var(--text-primary);
  }

  .project-mode-sidebar-body {
    font-size: var(--ui-font-sm);
    color: var(--text-secondary);
    line-height: 1.4;
  }

  .mode-btn.active {
    background-color: var(--accent);
    color: var(--bg-primary);
    border-color: var(--accent);
  }

  .mode-icon {
    width: 18px;
    height: 18px;
    border-radius: 4px;
    border: 1px solid var(--border-color);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: var(--ui-font-xs);
    font-family: monospace;
  }

  .mode-btn.active .mode-icon {
    border-color: rgba(0, 0, 0, 0.15);
  }

  .mode-label {
    font-size: var(--ui-font-xs);
    letter-spacing: 0.02em;
    text-transform: uppercase;
  }

  .filter-btn {
    flex: 1;
    background: none;
    border: 1px solid var(--border-color);
    color: var(--text-secondary);
    padding: 4px 8px;
    font-size: var(--ui-font-sm);
    cursor: pointer;
    border-radius: 4px;
    font-family: inherit;
  }

  .filter-btn.active {
    background-color: var(--accent);
    color: var(--bg-primary);
    border-color: var(--accent);
  }

  .cleanup-btn {
    background: none;
    border: 1px solid var(--border-color);
    color: var(--text-secondary);
    padding: 4px 8px;
    font-size: 11px;
    cursor: pointer;
    border-radius: 4px;
    font-family: inherit;
    white-space: nowrap;
  }

  .cleanup-btn:hover {
    background-color: var(--bg-hover);
    color: var(--text-primary);
  }

  .search-bar {
    padding: 6px 8px;
    border-bottom: 1px solid var(--border-color);
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .search-input {
    flex: 1;
    min-width: 0;
    width: auto;
    padding: 5px 8px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 4px;
    color: var(--text-primary);
    font-size: var(--ui-font-md);
    font-family: inherit;
    outline: none;
  }

  .search-input:focus {
    border-color: var(--accent);
  }

  .search-input::placeholder {
    color: var(--text-muted);
  }

  .sort-mode-toggle {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    border: 1px solid var(--border-color);
    background: none;
    color: var(--text-secondary);
    padding: 4px 8px;
    border-radius: 4px;
    cursor: pointer;
    font-family: inherit;
    font-size: var(--ui-font-xs);
  }

  .sort-mode-toggle:hover {
    background-color: var(--bg-hover);
    color: var(--text-primary);
  }

  .sort-mode-icon {
    font-size: var(--ui-font-xs);
    line-height: 1;
  }

  .sort-mode-text {
    white-space: nowrap;
  }

  .branch-summary-stack {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .branch-list {
    flex: 1 1 auto;
    min-height: 120px;
    overflow-y: auto;
    min-width: 0;
    padding: 4px 0;
  }

  .worktree-summary-wrap {
    border-top: 1px solid var(--border-color);
    background: var(--bg-primary);
    overflow-y: auto;
    flex: 0 0 auto;
    min-width: 0;
  }

  .summary-resize-handle {
    height: 8px;
    flex: 0 0 8px;
    border: none;
    background: transparent;
    cursor: row-resize;
    position: relative;
    width: 100%;
    padding: 0;
    touch-action: none;
  }

  .summary-resize-handle::before {
    content: "";
    position: absolute;
    left: 6px;
    right: 6px;
    top: 3px;
    height: 2px;
    background: var(--border-color);
    border-radius: 2px;
    opacity: 0.5;
  }

  .loading-indicator,
  .error-indicator,
  .empty-indicator {
    padding: 16px;
    text-align: center;
    color: var(--text-muted);
    font-size: var(--ui-font-md);
  }

  .error-indicator {
    color: rgb(255, 160, 160);
  }

  .branch-item {
    display: flex;
    align-items: center;
    padding: 6px 12px;
    cursor: pointer;
    gap: 8px;
    width: 100%;
    background: none;
    border: none;
    color: var(--text-primary);
    font-family: inherit;
    text-align: left;
  }

  .branch-item:hover {
    background-color: var(--bg-hover);
  }

  .branch-item.active {
    background-color: var(--bg-surface);
    color: var(--accent);
  }

  .branch-item.deleting {
    opacity: 0.5;
    cursor: default;
  }

  .branch-item.deleting:hover {
    background: none;
  }

  .branch-icon {
    color: var(--text-muted);
    font-size: var(--ui-font-md);
    font-family: monospace;
    width: 12px;
    flex-shrink: 0;
  }

  /* Safety dot indicator */
  .safety-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .safety-dot.safe {
    background-color: var(--green);
  }

  .safety-dot.warning {
    background-color: var(--yellow);
  }

  .safety-dot.danger {
    background-color: var(--red);
  }

  .safety-dot.disabled {
    background-color: var(--text-muted);
  }

  /* Spinner for deleting branches */
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .safety-spinner {
    width: 8px;
    height: 8px;
    border: 1.5px solid var(--text-muted);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
    flex-shrink: 0;
  }

  .branch-name {
    display: block;
    font-size: var(--ui-font-base);
    white-space: nowrap;
    overflow: hidden;
    flex: 1;
    min-width: 0;
  }

  .branch-name-label {
    display: block;
    min-width: 0;
    white-space: inherit;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .branch-name.auto-scroll {
    text-overflow: clip;
  }

  .branch-name.auto-scroll .branch-name-label {
    display: inline-block;
    min-width: max-content;
    overflow: visible;
    text-overflow: clip;
    padding-inline-end: var(--branch-scroll-gap, 24px);
    will-change: transform;
    animation: branch-name-marquee var(--branch-scroll-duration, 5000ms)
      linear infinite;
  }

  @keyframes branch-name-marquee {
    0%,
    14% {
      transform: translateX(0);
    }

    86%,
    100% {
      transform: translateX(calc(var(--branch-scroll-distance, 0px) * -1));
    }
  }

  .branch-rename-input {
    flex: 1;
    min-width: 0;
    border: 1px solid var(--accent-color);
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: inherit;
    font-family: inherit;
    padding: 0 2px;
    outline: none;
    border-radius: 2px;
  }

  /* Agent indicator: fixed-width slot for all branch rows (gwt-spec issue FR-800) */
  .agent-indicator-slot {
    width: 12px;
    height: 12px;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  /* Layer 1: static dot — tab is open (gwt-spec issue FR-801) */
  .agent-static-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--cyan);
    opacity: 0.45;
  }

  /* Layer 2: pulsing dot — LLM is Running (gwt-spec issue FR-802) */
  .agent-pulse-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--cyan);
    animation: agent-pulse 1.4s ease-in-out infinite;
  }

  @keyframes agent-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.2;
    }
  }

  /* Reduced-motion fallback: show "@" instead of animated dot */
  .agent-fallback {
    display: none;
    font-size: var(--ui-font-md);
    font-family: monospace;
    color: var(--cyan);
  }

  @media (prefers-reduced-motion: reduce) {
    .agent-pulse-dot,
    .agent-static-dot {
      display: none;
    }

    .agent-fallback {
      display: flex;
    }
  }

  .tool-usage {
    font-size: var(--ui-font-xs);
    font-family: monospace;
    color: var(--text-muted);
    border: 1px solid var(--border-color);
    border-radius: 999px;
    padding: 1px 6px;
    flex-shrink: 0;
  }

  .tool-usage.claude {
    color: var(--yellow);
    border-color: rgba(249, 226, 175, 0.35);
  }

  .tool-usage.codex {
    color: var(--cyan);
    border-color: rgba(148, 226, 213, 0.35);
  }

  .tool-usage.gemini {
    color: var(--magenta);
    border-color: rgba(203, 166, 247, 0.35);
  }

  .tool-usage.opencode {
    color: var(--green);
    border-color: rgba(166, 227, 161, 0.35);
  }

  .divergence {
    font-size: var(--ui-font-xs);
    font-family: monospace;
    padding: 1px 4px;
    border-radius: 3px;
    flex-shrink: 0;
  }

  .divergence.ahead {
    color: var(--green);
  }

  .divergence.behind {
    color: var(--yellow);
  }

  .divergence.diverged {
    color: var(--red);
  }

  /* Context menu */
  .context-menu {
    position: fixed;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    padding: 4px 0;
    min-width: 180px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    z-index: 2000;
  }

  .context-menu-item {
    display: block;
    width: 100%;
    background: none;
    border: none;
    color: var(--text-primary);
    padding: 6px 12px;
    font-size: 12px;
    font-family: inherit;
    text-align: left;
    cursor: pointer;
  }

  .context-menu-item:hover {
    background-color: var(--bg-hover);
  }

  .context-menu-item.disabled {
    color: var(--text-muted);
    cursor: default;
  }

  .context-menu-item.disabled:hover {
    background: none;
  }

  /* PR Status badge in branch list */
  .pr-badge {
    font-size: var(--ui-font-xs);
    font-family: monospace;
    padding: 1px 6px;
    border-radius: 999px;
    flex-shrink: 0;
    border: 1px solid var(--border-color);
  }

  .pr-badge.open {
    color: var(--green);
    border-color: rgba(63, 185, 80, 0.35);
  }

  .pr-badge.merged {
    color: var(--magenta, #a371f7);
    border-color: rgba(163, 113, 247, 0.35);
  }

  .pr-badge.closed {
    color: var(--red);
    border-color: rgba(248, 81, 73, 0.35);
  }

  .pr-badge.conflicting {
    color: var(--red);
    border-color: rgba(248, 81, 73, 0.35);
  }

  .pr-badge.blocked {
    color: var(--red);
    border-color: rgba(248, 81, 73, 0.35);
  }

  .pr-badge.checking {
    color: var(--text-muted);
    border-color: rgba(128, 128, 128, 0.35);
  }

  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }

  .pulse {
    animation: pulse 1.5s ease-in-out infinite;
  }
</style>
