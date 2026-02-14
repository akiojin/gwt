<script lang="ts">
  import type {
    BranchInfo,
    WorktreeInfo,
    CleanupProgress,
    LaunchAgentRequest,
    PrStatusInfo,
    WorkflowRunInfo,
    GhCliStatus,
  } from "../types";
  import { workflowStatusIcon, workflowStatusClass } from "../prStatusHelpers";
  import AgentSidebar from "./AgentSidebar.svelte";
  import WorktreeSummaryPanel from "./WorktreeSummaryPanel.svelte";

  type FilterType = "Local" | "Remote" | "All";
  type SidebarMode = "branch" | "agent";
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
  type TauriInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

  let {
    projectPath,
    onBranchSelect,
    onBranchActivate,
    onCleanupRequest,
    onLaunchAgent,
    onQuickLaunch,
    onResize,
    onOpenCiLog,
    widthPx = 260,
    minWidthPx = 220,
    maxWidthPx = 520,
    refreshKey = 0,
    mode = "branch",
    onModeChange,
    selectedBranch = null,
    currentBranch = "",
    agentTabBranches = [],
    prStatuses = {},
    ghCliStatus = null,
  }: {
    projectPath: string;
    onBranchSelect: (branch: BranchInfo) => void;
    onBranchActivate?: (branch: BranchInfo) => void;
    onCleanupRequest?: (preSelectedBranch?: string) => void;
    onLaunchAgent?: () => void;
    onQuickLaunch?: (request: LaunchAgentRequest) => Promise<void>;
    onResize?: (nextWidthPx: number) => void;
    onOpenCiLog?: (runId: number) => void;
    widthPx?: number;
    minWidthPx?: number;
    maxWidthPx?: number;
    refreshKey?: number;
    mode?: SidebarMode;
    onModeChange?: (next: SidebarMode) => void;
    selectedBranch?: BranchInfo | null;
    currentBranch?: string;
    agentTabBranches?: string[];
    prStatuses?: Record<string, PrStatusInfo | null>;
    ghCliStatus?: GhCliStatus | null;
  } = $props();

  const SIDEBAR_SUMMARY_HEIGHT_STORAGE_KEY = "gwt.sidebar.worktreeSummaryHeight";
  const DEFAULT_WORKTREE_SUMMARY_HEIGHT_PX = 360;
  const MIN_WORKTREE_SUMMARY_HEIGHT_PX = 160;
  const MIN_BRANCH_LIST_HEIGHT_PX = 120;
  const SUMMARY_RESIZE_HANDLE_HEIGHT_PX = 8;
  const FILTER_BACKGROUND_REFRESH_TTL_MS = 10_000;
  const SEARCH_FILTER_DEBOUNCE_MS = 120;

  // PR Status tree expand state
  let expandedBranches: Set<string> = $state(new Set());

  // PR Polling — inline to avoid .svelte.ts import issues in tests
  const PR_POLL_INTERVAL_MS = 30_000;
  let pollingStatuses: Record<string, PrStatusInfo | null> = $state({});
  let pollingGhCliStatus: GhCliStatus | null = $state(null);
  let prPollingBootstrappedPath: string | null = null;
  let prPollingInFlight = false;

  $effect(() => {
    const path = projectPath;
    if (!path) {
      prPollingBootstrappedPath = null;
      pollingStatuses = {};
      pollingGhCliStatus = null;
      return;
    }

    let destroyed = false;
    let timer: ReturnType<typeof setInterval> | null = null;

    async function refresh(markBootstrap = false) {
      if (destroyed) return;
      if (prPollingInFlight) return;
      prPollingInFlight = true;
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
          return;
        }
        if (markBootstrap) {
          prPollingBootstrappedPath = path;
        }
        const { invoke } = await import("@tauri-apps/api/core");
        const result = await invoke<{
          statuses: Record<string, PrStatusInfo | null>;
          ghStatus: GhCliStatus;
        }>("fetch_pr_status", { projectPath: path, branches: queryBranches });
        if (!destroyed) {
          const statuses = result.statuses ?? {};
          const mappedStatuses: Record<string, PrStatusInfo | null> = {};
          for (const branch of branches) {
            const key = branchKeyByName.get(branch.name) ?? branch.name;
            mappedStatuses[branch.name] = statuses[key] ?? null;
          }
          pollingStatuses = mappedStatuses;
          pollingGhCliStatus = result.ghStatus ?? null;
        }
      } catch {
        // Polling failure is silent — keep stale data
      } finally {
        prPollingInFlight = false;
      }
    }

    function start() {
      if (destroyed) return;
      stop();
      if (prPollingBootstrappedPath !== path) {
        void refresh(true);
      }
      timer = setInterval(() => {
        if (isTextEntryFocused()) return;
        void refresh();
      }, PR_POLL_INTERVAL_MS);
    }

    function stop() {
      if (timer !== null) {
        clearInterval(timer);
        timer = null;
      }
    }

    function onVisibility() {
      if (document.hidden) {
        stop();
      } else {
        start();
        if (!isTextEntryFocused()) {
          void refresh(false);
        }
      }
    }

    document.addEventListener("visibilitychange", onVisibility);
    start();

    return () => {
      destroyed = true;
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  });

  // Effective values: prefer polling data, fall back to props
  let activePrStatuses = $derived.by(() => {
    if (pollingStatuses && Object.keys(pollingStatuses).length > 0) {
      return pollingStatuses;
    }
    return prStatuses;
  });
  let activeGhCliStatus = $derived(pollingGhCliStatus ?? ghCliStatus);

  // Derived prNumber for WorktreeSummaryPanel
  let selectedPrNumber = $derived.by(() => {
    if (!selectedBranch) return null;
    const status = activePrStatuses[selectedBranch.name];
    return status?.number ?? null;
  });

  function toggleBranch(branchName: string) {
    const next = new Set(expandedBranches);
    if (next.has(branchName)) {
      next.delete(branchName);
    } else {
      next.add(branchName);
    }
    expandedBranches = next;
  }

  function openCiLog(run: WorkflowRunInfo) {
    onOpenCiLog?.(run.runId);
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
  let lastFetchKey = "";
  let lastForceKey = "";
  let lastProjectPath = "";
  let fetchToken = 0;
  let localRefreshKey = $state(0);
  let filterCache: Map<FilterType, FilterCacheEntry> = $state(new Map());
  const inflightFetches = new Map<string, Promise<FetchSnapshotResult>>();

  // Worktree safety info
  let worktreeMap: Map<string, WorktreeInfo> = $state(new Map());

  function normalizeTabBranch(name: string): string {
    const trimmed = name.trim();
    return trimmed.startsWith("origin/") ? trimmed.slice("origin/".length) : trimmed;
  }

  function stripRemotePrefix(name: string): string {
    const trimmed = name.trim();
    const slash = trimmed.indexOf("/");
    if (slash <= 0) return trimmed;
    return trimmed.slice(slash + 1);
  }

  function normalizeBranchForPrLookup(branchName: string): string {
    const trimmed = branchName.trim();
    return remoteBranchNames.has(trimmed) ? stripRemotePrefix(trimmed) : trimmed;
  }

  function loadSummaryHeight(): number {
    if (typeof window === "undefined") return DEFAULT_WORKTREE_SUMMARY_HEIGHT_PX;
    try {
      const raw = window.localStorage.getItem(SIDEBAR_SUMMARY_HEIGHT_STORAGE_KEY);
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
        String(Math.max(MIN_WORKTREE_SUMMARY_HEIGHT_PX, Math.round(heightPx)))
      );
    } catch {
      // Ignore localStorage failures.
    }
  }

  function clampSummaryHeight(nextHeightPx: number): number {
    const stackHeight = branchSummaryStackEl?.clientHeight ?? 0;
    const minSummaryHeight = Math.max(
      MIN_WORKTREE_SUMMARY_HEIGHT_PX,
      Math.round(nextHeightPx)
    );

    if (!Number.isFinite(stackHeight) || stackHeight <= 0) {
      return minSummaryHeight;
    }

    const availableSummaryHeight = Math.max(
      0,
      stackHeight - MIN_BRANCH_LIST_HEIGHT_PX - SUMMARY_RESIZE_HANDLE_HEIGHT_PX
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

  let agentTabBranchSet = $derived(
    new Set(
      agentTabBranches
        .map((b) => normalizeTabBranch(b))
        .filter((b) => b && b !== "Worktree" && b !== "Agent")
    )
  );

  function hasActiveAgentTab(branch: BranchInfo): boolean {
    // Local view only includes branches that have an active local worktree.
    if (activeFilter === "Local") return agentTabBranchSet.has(branch.name);

    // Only mark actual worktrees as active to avoid noise in Remote-only lists.
    if (activeFilter === "Remote") return false;
    if (!worktreeMap.get(branch.name)) return false;
    return agentTabBranchSet.has(branch.name);
  }

  // Branches currently being deleted
  let deletingBranches: Set<string> = $state(new Set());
  let branchSummaryStackEl: HTMLElement | null = $state(null);
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

  // Confirmation dialog state
  let confirmDelete: { branch: string; safetyLevel: string } | null =
    $state(null);
  let confirmDeleteError: string | null = $state(null);
  let resizing = false;
  let resizePointerId: number | null = null;
  let resizeStartX = 0;
  let resizeStartWidth = 0;
  let previousBodyCursor = "";
  let previousBodyUserSelect = "";

  const filters: FilterType[] = ["Local", "Remote", "All"];

  let filteredBranches = $derived(
    searchQuery
      ? branches.filter((b) =>
          b.name.toLowerCase().includes(searchQuery.toLowerCase())
        )
      : branches
  );
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
    const projectChanged = lastProjectPath !== "" && projectPath !== lastProjectPath;
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
    confirmDelete = null;
    confirmDeleteError = null;
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
        const { listen } = await import("@tauri-apps/api/event");
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
          }
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
        const { listen } = await import("@tauri-apps/api/event");
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

  // Close dialogs on Escape
  $effect(() => {
    if (!confirmDelete && !confirmDeleteError) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        confirmDelete = null;
        confirmDeleteError = null;
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
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
        forceRefresh || cached.dirty || ttlElapsed >= FILTER_BACKGROUND_REFRESH_TTL_MS;
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

  async function refreshFilterSnapshot(
    filter: FilterType,
    path: string,
    cacheKey: string,
    token: number,
    background: boolean,
    applyToActiveView: boolean
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

    if (background && hadFallbackCache) {
      if (applyToActiveView && filter === activeFilter) {
        loading = false;
      }
      return;
    }

    if (!applyToActiveView || filter !== activeFilter) {
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
    cacheKey: string
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
    cacheKey: string
  ): Promise<FetchSnapshotResult> {
    try {
      const invoke = await getInvoke();

      if (filter === "Local") {
        const next = await invoke<BranchInfo[]>("list_worktree_branches", { projectPath: path });
        const worktrees = await invoke<WorktreeInfo[]>("list_worktrees", { projectPath: path }).catch(
          () => [] as WorktreeInfo[]
        );
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
        const next = await invoke<BranchInfo[]>("list_remote_branches", { projectPath: path });
        return {
          ok: true,
          snapshot: {
            branches: next,
            remoteBranchNames: new Set(next.map((branch) => branch.name.trim())),
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

      const worktrees = await invoke<WorktreeInfo[]>("list_worktrees", { projectPath: path }).catch(
        () => [] as WorktreeInfo[]
      );

      return {
        ok: true,
        snapshot: {
          branches: merged,
          remoteBranchNames: new Set(remote.map((branch) => branch.name.trim())),
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
    if (filter === "Remote") {
      return `${path}::${refreshKey}`;
    }
    return `${path}::${refreshKey}::${localRefreshKey}`;
  }

  function buildWorktreeMap(worktrees: WorktreeInfo[]): Map<string, WorktreeInfo> {
    const map = new Map<string, WorktreeInfo>();
    for (const wt of worktrees) {
      if (wt.branch) map.set(wt.branch, wt);
    }
    return map;
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
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      return String((err as { message?: unknown }).message);
    }
    return String(err);
  }

  async function getInvoke(): Promise<TauriInvoke> {
    const tauriCore = (await import("@tauri-apps/api/core")) as
      | { invoke?: TauriInvoke; default?: { invoke?: TauriInvoke } }
      | undefined;
    const invokeFn = tauriCore?.invoke ?? tauriCore?.default?.invoke;
    if (!invokeFn) {
      throw new Error("Tauri invoke API is unavailable");
    }
    return invokeFn;
  }

  function getSafetyLevel(branch: BranchInfo): string {
    const wt = worktreeMap.get(branch.name);
    if (!wt) return "";
    return wt.safety_level || "";
  }

  function getSafetyTitle(branch: BranchInfo): string {
    const level = getSafetyLevel(branch);
    switch (level) {
      case "safe":
        return "Safe to delete";
      case "warning":
        return "Has uncommitted changes or unpushed commits";
      case "danger":
        return "Has uncommitted changes and unpushed commits";
      case "disabled":
        return "Protected or current branch";
      default:
        return "";
    }
  }

  function isBranchProtected(branch: BranchInfo): boolean {
    const wt = worktreeMap.get(branch.name);
    return wt ? wt.is_protected || wt.is_current : false;
  }

  function divergenceIndicator(branch: BranchInfo): string {
    switch (branch.divergence_status) {
      case "Ahead":
        return `+${branch.ahead}`;
      case "Behind":
        return `-${branch.behind}`;
      case "Diverged":
        return `+${branch.ahead} -${branch.behind}`;
      default:
        return "";
    }
  }

  function divergenceClass(status: string): string {
    switch (status) {
      case "Ahead":
        return "ahead";
      case "Behind":
        return "behind";
      case "Diverged":
        return "diverged";
      default:
        return "";
    }
  }

  function toolUsageClass(usage: string | null | undefined): string {
    const key = (usage ?? "").toLowerCase();
    if (key.startsWith("claude@")) return "claude";
    if (key.startsWith("codex@")) return "codex";
    if (key.startsWith("gemini@")) return "gemini";
    if (key.startsWith("opencode@") || key.startsWith("open-code@"))
      return "opencode";
    return "";
  }

  // --- Context menu ---

  function clampSidebarWidth(width: number): number {
    const min = Number.isFinite(minWidthPx) ? minWidthPx : 220;
    const maxCandidate = Number.isFinite(maxWidthPx) ? maxWidthPx : 520;
    const max = Math.max(min, maxCandidate);
    if (!Number.isFinite(width)) return min;
    return Math.max(min, Math.min(max, Math.round(width)));
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
    if (summaryResizePointerId !== null && event.pointerId !== summaryResizePointerId) return;
    const delta = event.clientY - summaryResizeStartY;
    setSummaryHeight(summaryResizeStartHeight - delta);
  }

  function handleSummaryResizePointerUp(event: PointerEvent) {
    if (summaryResizePointerId !== null && event.pointerId !== summaryResizePointerId) return;
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
    const branch = contextMenu.branch;
    contextMenu = null;
    const level = getSafetyLevel(branch);
    confirmDelete = { branch: branch.name, safetyLevel: level };
    confirmDeleteError = null;
  }

  function handleCleanupWorktrees() {
    if (!contextMenu) return;
    const branchName = contextMenu.branch.name;
    contextMenu = null;
    onCleanupRequest?.(branchName);
  }

  // --- Single delete ---

  async function handleConfirmDelete() {
    if (!confirmDelete) return;
    const { branch, safetyLevel } = confirmDelete;
    const force = safetyLevel !== "safe";
    confirmDelete = null;
    confirmDeleteError = null;

    deletingBranches = new Set([...deletingBranches, branch]);

    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("cleanup_single_worktree", {
        projectPath,
        branch,
        force,
      });
      const next = new Set(deletingBranches);
      next.delete(branch);
      deletingBranches = next;
      localRefreshKey++;
    } catch (err) {
      const next = new Set(deletingBranches);
      next.delete(branch);
      deletingBranches = next;
      confirmDeleteError =
        typeof err === "string"
          ? err
          : err && typeof err === "object" && "message" in err
            ? String((err as { message?: unknown }).message)
            : String(err);
    }
  }
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
    <button
      class="mode-btn"
      class:active={mode === "agent"}
      aria-pressed={mode === "agent"}
      title="Agent Mode"
      onclick={() => handleModeChange("agent")}
    >
      <span class="mode-icon">A</span>
      <span class="mode-label">Agent</span>
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
    </div>
    <div class="branch-summary-stack" bind:this={branchSummaryStackEl}>
      <div class="branch-list">
        {#if loading}
          <div class="loading-indicator">Loading...</div>
        {:else if errorMessage}
          <div class="error-indicator">{errorMessage}</div>
        {:else if filteredBranches.length === 0}
          <div class="empty-indicator">No branches found.</div>
        {:else}
          {#each filteredBranches as branch}
            <div class="branch-tree-item">
              {#if activeFilter !== "Remote" && activePrStatuses[branch.name]}
                <button
                  class="tree-toggle"
                  class:expanded={expandedBranches.has(branch.name)}
                  onclick={(e) => { e.stopPropagation(); toggleBranch(branch.name); }}
                  title={expandedBranches.has(branch.name) ? "Collapse" : "Expand"}
                >
                  {expandedBranches.has(branch.name) ? "\u25BC" : "\u25B6"}
                </button>
              {:else}
                <span class="tree-toggle-placeholder"></span>
              {/if}
              <button
                class="branch-item"
                class:active={branch.is_current}
                class:agent-active={hasActiveAgentTab(branch)}
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
                <span class="branch-icon">{branch.is_current ? "*" : " "}</span>
                {#if deletingBranches.has(branch.name)}
                  <span class="safety-spinner"></span>
                {:else if getSafetyLevel(branch)}
                  <span
                    class="safety-dot {getSafetyLevel(branch)}"
                    title={getSafetyTitle(branch)}
                  ></span>
                {/if}
                {#if hasActiveAgentTab(branch)}
                  <span
                    class="agent-tab-icon"
                    title="Agent tab is open for this branch"
                    role="img"
                    aria-label="Agent tab is open for this branch"
                  >
                    <span class="agent-tab-bars" aria-hidden="true">
                      <span class="agent-tab-bar b1"></span>
                      <span class="agent-tab-bar b2"></span>
                      <span class="agent-tab-bar b3"></span>
                    </span>
                    <span class="agent-tab-fallback" aria-hidden="true">@</span>
                  </span>
                {/if}
                <span class="branch-name">{branch.name}</span>
                {#if activeGhCliStatus && !activeGhCliStatus.authenticated}
                  <span class="pr-badge disconnected">GitHub not connected</span>
                {:else if activePrStatuses[branch.name]}
                  {@const pr = activePrStatuses[branch.name]!}
                  <span class="pr-badge {pr.state.toLowerCase()}">
                    #{pr.number} {pr.state === "OPEN" ? "Open" : pr.state === "MERGED" ? "Merged" : "Closed"}
                  </span>
                {:else if activeGhCliStatus?.authenticated}
                  <span class="pr-badge no-pr">No PR</span>
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
              </button>
            </div>
            {#if expandedBranches.has(branch.name) && activePrStatuses[branch.name]}
              <div class="workflow-runs">
                {#each activePrStatuses[branch.name]!.checkSuites as run}
                  <button class="workflow-run-item" onclick={() => openCiLog(run)}>
                    <span class="workflow-status {workflowStatusClass(run)}">{workflowStatusIcon(run)}</span>
                    <span class="workflow-name">{run.workflowName}</span>
                  </button>
                {:else}
                  <div class="workflow-empty">No workflows</div>
                {/each}
              </div>
            {/if}
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
      <div
        class="worktree-summary-wrap"
        style="height: {summaryHeightPx}px;"
      >
        <WorktreeSummaryPanel
          {projectPath}
          {selectedBranch}
          prNumber={selectedPrNumber}
          onLaunchAgent={onLaunchAgent}
          onQuickLaunch={onQuickLaunch}
        />
      </div>
    </div>
  {:else}
    <AgentSidebar
      {projectPath}
      selectedBranch={selectedBranch}
      currentBranch={currentBranch}
    />
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

<!-- Single delete confirmation dialog -->
{#if mode === "branch"}
  {#if confirmDelete}
    {@const wt = worktreeMap.get(confirmDelete.branch)}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="overlay" onclick={() => (confirmDelete = null)}>
      <div class="confirm-dialog" onclick={(e) => e.stopPropagation()}>
        <h3>Delete Worktree</h3>
        <p class="confirm-text">
          {#if confirmDelete.safetyLevel === "danger"}
            Branch <strong>{confirmDelete.branch}</strong> has uncommitted changes
            and unpushed commits. This cannot be undone.
          {:else if confirmDelete.safetyLevel === "warning" && wt?.has_changes}
            Branch <strong>{confirmDelete.branch}</strong> has uncommitted changes.
          {:else if confirmDelete.safetyLevel === "warning" && wt?.has_unpushed}
            Branch <strong>{confirmDelete.branch}</strong> has unpushed commits.
          {:else}
            Delete worktree and local branch <strong
              >{confirmDelete.branch}</strong
            >?
          {/if}
        </p>
        <div class="confirm-actions">
          <button class="confirm-cancel" onclick={() => (confirmDelete = null)}>
            Cancel
          </button>
          <button class="confirm-delete" onclick={handleConfirmDelete}>
            Delete
          </button>
        </div>
      </div>
    </div>
  {/if}
{/if}

<!-- Delete error dialog -->
{#if mode === "branch"}
  {#if confirmDeleteError}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="overlay" onclick={() => (confirmDeleteError = null)}>
      <div class="confirm-dialog" onclick={(e) => e.stopPropagation()}>
        <h3>Delete Failed</h3>
        <p class="confirm-error">{confirmDeleteError}</p>
        <div class="confirm-actions">
          <button
            class="confirm-cancel"
            onclick={() => (confirmDeleteError = null)}
          >
            Close
          </button>
        </div>
      </div>
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
  }

  .search-input {
    width: 100%;
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

  .branch-item.agent-active:not(.active) {
    background-color: rgba(148, 226, 213, 0.08);
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
    font-size: var(--ui-font-base);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
  }

  .agent-tab-icon {
    color: var(--cyan);
    width: 12px;
    text-align: center;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 12px;
    line-height: 1;
  }

  .agent-tab-bars {
    display: inline-flex;
    align-items: flex-end;
    justify-content: center;
    gap: 1px;
    height: 10px;
  }

  .agent-tab-bar {
    width: 2px;
    height: 4px;
    border-radius: 1px;
    background: var(--cyan);
    opacity: 0.85;
    transform-origin: bottom;
    animation: agentTabBars 0.9s ease-in-out infinite;
  }

  .agent-tab-bar.b1 {
    animation-delay: 0ms;
  }

  .agent-tab-bar.b2 {
    animation-delay: 150ms;
  }

  .agent-tab-bar.b3 {
    animation-delay: 300ms;
  }

  /* Graphical activity indicator for branches with open agent tabs */
  @keyframes agentTabBars {
    0%,
    100% {
      transform: scaleY(0.35);
      opacity: 0.55;
    }
    50% {
      transform: scaleY(1);
      opacity: 1;
    }
  }

  .agent-tab-fallback {
    display: none;
    font-size: var(--ui-font-md);
    font-family: monospace;
  }

  @media (prefers-reduced-motion: reduce) {
    .agent-tab-bars {
      display: none;
    }

    .agent-tab-bar {
      animation: none;
    }

    .agent-tab-fallback {
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

  /* Overlays & dialogs */
  .overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
  }

  .confirm-dialog {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 10px;
    padding: 20px 24px;
    max-width: 400px;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.4);
  }

  .confirm-dialog h3 {
    font-size: 14px;
    font-weight: 700;
    color: var(--text-primary);
    margin-bottom: 8px;
  }

  .confirm-text {
    color: var(--text-secondary);
    font-size: 12px;
    line-height: 1.5;
    margin-bottom: 16px;
  }

  .confirm-error {
    color: rgb(255, 160, 160);
    font-size: 12px;
    line-height: 1.5;
    margin-bottom: 16px;
    white-space: pre-wrap;
  }

  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .confirm-cancel {
    padding: 5px 14px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 5px;
    color: var(--text-primary);
    cursor: pointer;
    font-family: inherit;
    font-size: 12px;
  }

  .confirm-cancel:hover {
    background: var(--bg-hover);
  }

  .confirm-delete {
    padding: 5px 14px;
    background: var(--red);
    border: 1px solid transparent;
    border-radius: 5px;
    color: var(--bg-primary);
    cursor: pointer;
    font-family: inherit;
    font-size: 12px;
    font-weight: 600;
  }

  .confirm-delete:hover {
    opacity: 0.9;
  }

  /* PR Status tree */
  .branch-tree-item {
    display: flex;
    align-items: stretch;
  }

  .tree-toggle {
    width: 20px;
    flex-shrink: 0;
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 10px;
    padding: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .tree-toggle:hover {
    color: var(--text-primary);
  }

  .tree-toggle-placeholder {
    width: 20px;
    flex-shrink: 0;
  }

  .workflow-runs {
    padding-left: 24px;
    display: flex;
    flex-direction: column;
  }

  .workflow-run-item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    background: none;
    border: none;
    color: var(--text-secondary);
    font-size: var(--ui-font-xs);
    cursor: pointer;
    text-align: left;
    font-family: inherit;
  }

  .workflow-run-item:hover {
    background: var(--bg-hover);
  }

  .workflow-status {
    font-size: 11px;
    width: 14px;
    text-align: center;
  }

  .workflow-status.pass {
    color: var(--green);
  }

  .workflow-status.fail {
    color: var(--red);
  }

  .workflow-status.running {
    color: var(--yellow);
  }

  .workflow-status.pending {
    color: var(--text-muted);
  }

  .workflow-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .workflow-empty {
    padding: 3px 8px;
    color: var(--text-muted);
    font-size: var(--ui-font-xs);
    font-style: italic;
  }

  .pr-badge {
    font-size: var(--ui-font-xs);
    padding: 1px 6px;
    border-radius: 999px;
    white-space: nowrap;
    flex-shrink: 0;
    font-weight: 600;
  }

  .pr-badge.open {
    background: rgba(63, 185, 80, 0.15);
    color: var(--green);
  }

  .pr-badge.merged {
    background: rgba(163, 113, 247, 0.15);
    color: var(--magenta);
  }

  .pr-badge.closed {
    background: rgba(248, 81, 73, 0.15);
    color: var(--red);
  }

  .pr-badge.no-pr {
    color: var(--text-muted);
    background: none;
  }

  .pr-badge.disconnected {
    color: var(--text-muted);
    background: none;
    font-style: italic;
  }
</style>
