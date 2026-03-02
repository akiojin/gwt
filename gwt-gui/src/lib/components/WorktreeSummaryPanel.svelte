<script lang="ts">
  import { onMount } from "svelte";
  import { invoke as tauriInvoke } from "$lib/tauriInvoke";
  import type {
    BranchInfo,
    BranchLinkedIssueInfo,
    BranchPrReference,
    DockerContext,
    LaunchAgentRequest,
    ToolSessionEntry,
    SessionSummaryResult,
    PrStatusInfo,
    PrStatusLite,
    GhCliStatus,
    WorkflowRunInfo,
    SettingsData,
  } from "../types";
  import GitSection from "./GitSection.svelte";
  import MarkdownRenderer from "./MarkdownRenderer.svelte";
  import PrStatusSection from "./PrStatusSection.svelte";
  import MergeConfirmModal from "./MergeConfirmModal.svelte";
  import { openExternalUrl } from "../openExternalUrl";
  import { toastBus } from "../toastBus";
  import {
    toErrorMessage,
    normalizeBranchName,
    formatSessionSummaryTimestamp,
    normalizeSummaryLanguage,
    summaryLanguageLabel,
    agentIdForToolId,
    toolClass,
    displayToolName,
    displayToolVersion,
    normalizeString,
    hasDockerInfo,
    dockerMode,
    dockerModeClass,
    formatComposeArgs,
    formatTimestamp,
    quickStartEntryKey,
    normalizeLinkedIssue,
    formatIsoTimestamp,
    sessionSummaryHeaderSubtitle,
    hasSessionSummaryIdentity,
    hasSessionSummaryMeta,
    sessionSummarySourceLabel,
    linkedIssueTitle,
    type DockerMode,
    type DockerModeClass,
  } from "./worktreeSummaryHelpers";

  let {
    projectPath,
    selectedBranch = null,
    onLaunchAgent,
    onQuickLaunch,
    onNewTerminal,
    onOpenDocsEditor,
    onOpenCiLog,
    agentTabBranches = [],
    activeAgentTabBranch = null,
    preferredLanguage = "auto",
    prNumber = null,
    selectedPrStatus = null,
    ghCliStatus = null,
  }: {
    projectPath: string;
    selectedBranch?: BranchInfo | null;
    onLaunchAgent?: () => void;
    onQuickLaunch?: (request: LaunchAgentRequest) => Promise<void>;
    onNewTerminal?: () => void;
    onOpenDocsEditor?: (worktreePath: string) => Promise<void> | void;
    onOpenCiLog?: (runId: number) => void;
    agentTabBranches?: string[];
    activeAgentTabBranch?: string | null;
    preferredLanguage?: SettingsData["app_language"];
    prNumber?: number | null;
    selectedPrStatus?: PrStatusLite | null;
    ghCliStatus?: GhCliStatus | null;
  } = $props();

  let quickStartEntries: ToolSessionEntry[] = $state([]);
  let quickStartLoading: boolean = $state(false);
  let quickStartError: string | null = $state(null);
  let quickLaunchError: string | null = $state(null);
  let quickLaunching: boolean = $state(false);
  let quickLaunchingKey: string | null = $state(null);
  let docsActionBusy: boolean = $state(false);
  let docsActionError: string | null = $state(null);

  type SummaryTab =
    | "summary"
    | "git"
    | "issue"
    | "pr"
    | "docker";
  let activeTab: SummaryTab = $state("summary");

  let linkedIssueLoading: boolean = $state(false);
  let linkedIssueError: string | null = $state(null);
  let linkedIssue: BranchLinkedIssueInfo | null = $state(null);

  let latestBranchPrLoading: boolean = $state(false);
  let latestBranchPrError: string | null = $state(null);
  let latestBranchPr: BranchPrReference | null = $state(null);
  let latestBranchPrBranch: string | null = $state(null);

  let dockerContextLoading: boolean = $state(false);
  let dockerContextError: string | null = $state(null);
  let dockerContext: DockerContext | null = $state(null);

  let prDetailLoading = $state(false);
  let prDetailError: string | null = $state(null);
  let updateBranchError: string | null = $state(null);
  let prDetail: PrStatusInfo | null = $state(null);
  let prDetailBranch: string | null = $state(null);
  let prDetailPrNumber: number | null = $state(null);
  let prDetailRequestToken = 0;
  let lastProjectPath: string | null = $state(null);

  let sessionSummaryLoading: boolean = $state(false);
  let sessionSummaryGenerating: boolean = $state(false);
  let sessionSummaryStatus: SessionSummaryResult["status"] | "" = $state("");
  let sessionSummaryMarkdown: string | null = $state(null);
  let sessionSummaryWarning: string | null = $state(null);
  let sessionSummaryError: string | null = $state(null);
  let sessionSummaryToolId: string | null = $state(null);
  let sessionSummarySessionId: string | null = $state(null);
  let sessionSummaryLanguage: string | null = $state(null);
  let sessionSummarySourceType: SessionSummaryResult["sourceType"] | null = $state(
    null,
  );
  let sessionSummaryInputMtimeMs: number | null = $state(null);
  let sessionSummaryUpdatedMs: number | null = $state(null);
  let summaryRebuildInProgress = $state(false);
  let summaryRebuildTotal = $state(0);
  let summaryRebuildCompleted = $state(0);
  let summaryRebuildBranch: string | null = $state(null);
  let summaryRebuildError: string | null = $state(null);

  let sessionSummaryHeaderText = $derived.by(() =>
    sessionSummaryHeaderSubtitle({
      summaryRebuildInProgress,
      summaryRebuildCompleted,
      summaryRebuildTotal,
      summaryRebuildBranch,
      sessionSummaryLoading,
      sessionSummaryStatus,
      sessionSummaryToolId,
      sessionSummarySessionId,
      sessionSummaryGenerating,
      sessionSummaryMarkdown,
    }),
  );
  let sessionSummaryHasIdentity = $derived.by(() =>
    hasSessionSummaryIdentity(
      sessionSummaryStatus,
      sessionSummaryToolId,
      sessionSummarySessionId,
    ),
  );
  let sessionSummaryInputTime = $derived.by(() =>
    formatSessionSummaryTimestamp(sessionSummaryInputMtimeMs),
  );
  let sessionSummaryUpdatedTime = $derived.by(() =>
    formatSessionSummaryTimestamp(sessionSummaryUpdatedMs),
  );
  let sessionSummaryLanguageLabel = $derived.by(() =>
    summaryLanguageLabel(sessionSummaryLanguage),
  );
  let sessionSummaryHasMeta = $derived.by(() =>
    hasSessionSummaryMeta(
      sessionSummarySourceType,
      sessionSummaryLanguageLabel,
      sessionSummaryInputTime,
      sessionSummaryUpdatedTime,
    ),
  );

  const SESSION_SUMMARY_POLL_FOCUSED_INTERVAL_MS = 15000;
  const SESSION_SUMMARY_POLL_NONFOCUSED_INTERVAL_MS = 60000;
  const QUICK_START_CACHE_TTL_MS = 30_000;
  const LINKED_ISSUE_CACHE_TTL_MS = 120_000;
  const LATEST_BRANCH_PR_CACHE_TTL_MS = 30_000;
  const DOCKER_CONTEXT_CACHE_TTL_MS = 60_000;
  const UPDATE_BRANCH_POLL_ATTEMPTS = 8;
  const UPDATE_BRANCH_POLL_INTERVAL_MS = 1_500;
  const MERGE_POLL_ATTEMPTS = 10;
  const MERGE_POLL_INTERVAL_MS = 2_000;

  type CacheEntry<T> = {
    value: T;
    fetchedAtMs: number;
  };

  type DockerSummaryRow = {
    entry: ToolSessionEntry;
    mode: DockerMode;
    modeClass: DockerModeClass;
    composeArgs: string | null;
    service: string | null;
    containerName: string | null;
  };

  const quickStartCache = new Map<string, CacheEntry<ToolSessionEntry[]>>();
  const linkedIssueCache = new Map<string, CacheEntry<BranchLinkedIssueInfo | null>>();
  const latestBranchPrCache = new Map<string, CacheEntry<BranchPrReference | null>>();
  const dockerContextCache = new Map<string, CacheEntry<DockerContext | null>>();

  let ghCliStatusMessage = $derived.by(() => {
    if (!ghCliStatus) return null;
    if (!ghCliStatus.available) {
      return "GitHub CLI (gh) is not available.";
    }
    if (!ghCliStatus.authenticated) {
      return "GitHub CLI (gh) is not authenticated. Run: gh auth login";
    }
    return null;
  });

  type SessionSummaryUpdatedPayload = {
    projectPath: string;
    branch: string;
    result: SessionSummaryResult;
  };
  type SessionSummaryRebuildProgressPayload = {
    projectPath: string;
    language: string;
    total: number;
    completed: number;
    branch?: string | null;
    status: string;
    error?: string | null;
  };

  type InstructionDocsCheckResult = {
    worktreePath: string;
    checkedFiles: string[];
    updatedFiles: string[];
  };
  type TauriInvoke = <T>(
    command: string,
    args?: Record<string, unknown>,
  ) => Promise<T>;

  function currentBranchName(): string {
    const rawBranch = selectedBranch?.name?.trim() ?? "";
    return normalizeBranchName(rawBranch);
  }

  function currentBranchKey(): string {
    const branch = currentBranchName();
    if (!branch) return "";
    return `${projectPath}::${branch}`;
  }

  function isCacheFresh<T>(entry: CacheEntry<T> | undefined, ttlMs: number): entry is CacheEntry<T> {
    if (!entry) return false;
    return Date.now() - entry.fetchedAtMs < ttlMs;
  }

  async function waitForNextFrame(): Promise<void> {
    if (typeof window === "undefined" || typeof window.requestAnimationFrame !== "function") {
      await new Promise<void>((resolve) => {
        setTimeout(resolve, 0);
      });
      return;
    }
    await new Promise<void>((resolve) => {
      window.requestAnimationFrame(() => resolve());
    });
  }

  async function waitMs(ms: number): Promise<void> {
    await new Promise<void>((resolve) => {
      setTimeout(resolve, ms);
    });
  }

  function hasAgentTabForBranch(branch: string): boolean {
    const target = normalizeBranchName(branch);
    return (agentTabBranches ?? [])
      .map((b) => normalizeBranchName(b))
      .includes(target);
  }

  function isAgentTabFocusedForBranch(branch: string): boolean {
    const target = normalizeBranchName(branch);
    const active = (activeAgentTabBranch ?? "").trim();
    if (!active) return false;
    return normalizeBranchName(active) === target;
  }

  let dockerSummaryRows: DockerSummaryRow[] = $derived.by(() => {
    return quickStartEntries
      .filter(hasDockerInfo)
      .map((entry) => ({
        entry,
        mode: dockerMode(entry),
        modeClass: dockerModeClass(entry),
        composeArgs: formatComposeArgs(entry.docker_compose_args),
        service: (normalizeString(entry.docker_service) || null),
        containerName: (normalizeString(entry.docker_container_name) || null),
      }))
      .sort((left, right) => right.entry.timestamp - left.entry.timestamp);
  });

  let latestQuickStartEntry: ToolSessionEntry | null = $derived.by(() => {
    if (quickStartEntries.length === 0) return null;
    return quickStartEntries.reduce((latest, entry) => {
      return entry.timestamp > latest.timestamp ? entry : latest;
    });
  });

  let quickHeaderButtonsDisabled = $derived.by(
    () =>
      quickStartLoading ||
      quickLaunching ||
      !onQuickLaunch ||
      latestQuickStartEntry === null,
  );

  type LoadOptions = {
    force?: boolean;
    defer?: boolean;
  };

  async function loadQuickStart(options: LoadOptions = {}) {
    const force = options.force === true;
    const defer = options.defer === true;
    quickLaunchError = null;
    quickStartError = null;

    const branch = currentBranchName();
    if (!branch) {
      quickStartEntries = [];
      quickStartLoading = false;
      return;
    }

    const key = `${projectPath}::${branch}`;
    const cached = quickStartCache.get(key);
    if (!force && isCacheFresh(cached, QUICK_START_CACHE_TTL_MS)) {
      quickStartEntries = cached.value;
      quickStartLoading = false;
      return;
    }

    quickStartLoading = true;

    try {
      if (defer) {
        await waitForNextFrame();
      }
      if (currentBranchKey() !== key) return;

      const invoke = await getInvoke();
      const entries = await invoke<ToolSessionEntry[]>("get_branch_quick_start", {
        projectPath,
        branch,
      });
      if (currentBranchKey() !== key) return;
      const nextEntries = entries ?? [];
      quickStartCache.set(key, { value: nextEntries, fetchedAtMs: Date.now() });
      quickStartEntries = nextEntries;
    } catch (err) {
      if (currentBranchKey() !== key) return;
      quickStartEntries = [];
      quickStartError = `Failed to load Quick Start: ${toErrorMessage(err)}`;
    } finally {
      if (currentBranchKey() === key) {
        quickStartLoading = false;
      }
    }
  }

  async function loadBranchLinkedIssue(options: LoadOptions = {}) {
    const force = options.force === true;
    const defer = options.defer === true;
    linkedIssueError = null;

    const branch = currentBranchName();
    if (!branch) {
      linkedIssueLoading = false;
      linkedIssue = null;
      return;
    }

    const key = `${projectPath}::${branch}`;
    const cached = linkedIssueCache.get(key);
    if (!force && isCacheFresh(cached, LINKED_ISSUE_CACHE_TTL_MS)) {
      linkedIssue = cached.value;
      linkedIssueLoading = false;
      return;
    }

    linkedIssue = null;
    linkedIssueLoading = true;
    try {
      if (defer) {
        await waitForNextFrame();
      }
      if (currentBranchKey() !== key) return;

      const invoke = await getInvoke();
      const rawResult = await invoke<unknown>("fetch_branch_linked_issue", {
        projectPath,
        branch,
      });
      const result = normalizeLinkedIssue(rawResult);
      if (currentBranchKey() !== key) return;
      linkedIssueCache.set(key, { value: result, fetchedAtMs: Date.now() });
      linkedIssue = result;
    } catch (err) {
      if (currentBranchKey() !== key) return;
      linkedIssue = null;
      linkedIssueError = `Failed to load linked issue: ${toErrorMessage(err)}`;
    } finally {
      if (currentBranchKey() === key) {
        linkedIssueLoading = false;
      }
    }
  }

  async function loadLatestBranchPr(options: LoadOptions = {}) {
    const force = options.force === true;
    const defer = options.defer === true;
    latestBranchPrError = null;

    const branch = currentBranchName();
    if (!branch) {
      latestBranchPrLoading = false;
      latestBranchPr = null;
      latestBranchPrBranch = null;
      return;
    }

    const key = `${projectPath}::${branch}`;
    const cached = latestBranchPrCache.get(key);
    if (!force && isCacheFresh(cached, LATEST_BRANCH_PR_CACHE_TTL_MS)) {
      latestBranchPr = cached.value;
      latestBranchPrBranch = branch;
      latestBranchPrLoading = false;
      return;
    }

    latestBranchPr = null;
    latestBranchPrBranch = branch;
    latestBranchPrLoading = true;
    try {
      if (defer) {
        await waitForNextFrame();
      }
      if (currentBranchKey() !== key) return;

      const invoke = await getInvoke();
      const result = await invoke<BranchPrReference | null>("fetch_latest_branch_pr", {
        projectPath,
        branch,
      });
      if (currentBranchKey() !== key) return;
      latestBranchPrCache.set(key, { value: result, fetchedAtMs: Date.now() });
      latestBranchPr = result;
      latestBranchPrBranch = branch;
    } catch (err) {
      if (currentBranchKey() !== key) return;
      latestBranchPr = null;
      latestBranchPrBranch = branch;
      latestBranchPrError = `Failed to load PR: ${toErrorMessage(err)}`;
    } finally {
      if (currentBranchKey() === key) {
        latestBranchPrLoading = false;
      }
    }
  }

  async function loadDockerContext(options: LoadOptions = {}) {
    const force = options.force === true;
    const defer = options.defer === true;
    dockerContextError = null;

    const branch = currentBranchName();
    if (!branch) {
      dockerContextLoading = false;
      dockerContext = null;
      return;
    }

    const key = `${projectPath}::${branch}`;
    const cached = dockerContextCache.get(key);
    if (!force && isCacheFresh(cached, DOCKER_CONTEXT_CACHE_TTL_MS)) {
      dockerContext = cached.value;
      dockerContextLoading = false;
      return;
    }

    dockerContext = null;
    dockerContextLoading = true;
    try {
      if (defer) {
        await waitForNextFrame();
      }
      if (currentBranchKey() !== key) return;

      const invoke = await getInvoke();
      const result = await invoke<DockerContext>("detect_docker_context", {
        projectPath,
        branch,
      });
      if (currentBranchKey() !== key) return;
      dockerContextCache.set(key, { value: result, fetchedAtMs: Date.now() });
      dockerContext = result;
    } catch (err) {
      if (currentBranchKey() !== key) return;
      dockerContext = null;
      dockerContextError = `Failed to detect Docker context: ${toErrorMessage(err)}`;
    } finally {
      if (currentBranchKey() === key) {
        dockerContextLoading = false;
      }
    }
  }

  async function loadSessionSummary(
    options: { silent?: boolean; cachedOnly?: boolean } = {},
  ) {
    const silent = options.silent === true;
    const cachedOnly = options.cachedOnly === true;
    const normalizedLanguage = normalizeSummaryLanguage(preferredLanguage);
    sessionSummaryError = null;
    sessionSummaryWarning = null;

    const branch = currentBranchName();
    if (!branch) {
      sessionSummaryLoading = false;
      sessionSummaryGenerating = false;
      sessionSummaryStatus = "";
      sessionSummaryMarkdown = null;
      sessionSummaryToolId = null;
      sessionSummarySessionId = null;
      sessionSummaryLanguage = null;
      sessionSummarySourceType = null;
      sessionSummaryInputMtimeMs = null;
      sessionSummaryUpdatedMs = null;
      return;
    }

    const key = `${projectPath}::${branch}`;
    if (!silent) {
      sessionSummaryLoading = true;
      sessionSummaryGenerating = false;
      sessionSummaryStatus = "";
      sessionSummaryMarkdown = null;
      sessionSummaryToolId = null;
      sessionSummarySessionId = null;
      sessionSummaryLanguage = null;
      sessionSummarySourceType = null;
      sessionSummaryInputMtimeMs = null;
      sessionSummaryUpdatedMs = null;
    }

    try {
      const invoke = await getInvoke();
      const result = await invoke<SessionSummaryResult>("get_branch_session_summary", {
        projectPath,
        branch,
        cachedOnly,
        preferredLanguage: normalizedLanguage,
      });

      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey !== key) return;

      sessionSummaryStatus = result.status;
      sessionSummaryGenerating = !!result.generating;
      const nextMarkdown = result.markdown ?? null;
      if (nextMarkdown !== null) {
        sessionSummaryMarkdown = nextMarkdown;
      } else if (!silent || result.status !== "ok") {
        sessionSummaryMarkdown = null;
      }
      sessionSummaryWarning = result.warning ?? null;
      sessionSummaryError = result.error ?? null;
      sessionSummaryToolId = result.toolId ?? null;
      sessionSummarySessionId = result.sessionId ?? null;
      sessionSummaryLanguage = result.language ?? normalizedLanguage;
      sessionSummarySourceType = result.sourceType ?? null;
      sessionSummaryInputMtimeMs = result.inputMtimeMs ?? null;
      sessionSummaryUpdatedMs = result.summaryUpdatedMs ?? null;
    } catch (err) {
      sessionSummaryStatus = "error";
      sessionSummaryGenerating = false;
      if (!silent) {
        sessionSummaryMarkdown = null;
      }
      sessionSummaryToolId = null;
      sessionSummarySessionId = null;
      sessionSummaryLanguage = null;
      sessionSummarySourceType = null;
      sessionSummaryInputMtimeMs = null;
      sessionSummaryUpdatedMs = null;
      sessionSummaryError = `Failed to generate session summary: ${toErrorMessage(err)}`;
    } finally {
      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey === key && !silent) {
        sessionSummaryLoading = false;
      }
    }
  }

  $effect(() => {
    void selectedBranch;
    void projectPath;
    loadQuickStart();
  });

  $effect(() => {
    void selectedBranch;
    void projectPath;
    void activeTab;

    if (!currentBranchName()) {
      linkedIssueLoading = false;
      linkedIssueError = null;
      linkedIssue = null;
      latestBranchPrLoading = false;
      latestBranchPrError = null;
      latestBranchPr = null;
      latestBranchPrBranch = null;
      dockerContextLoading = false;
      dockerContextError = null;
      dockerContext = null;
      return;
    }

    if (activeTab === "issue") {
      loadBranchLinkedIssue({ defer: true });
      return;
    }
    if (activeTab === "pr") {
      loadLatestBranchPr({ defer: true });
      return;
    }
    if (activeTab === "docker") {
      loadDockerContext({ defer: true });
    }
  });

  $effect(() => {
    void selectedBranch;
    void projectPath;
    void agentTabBranches;
    void activeAgentTabBranch;
    void preferredLanguage;

    const branch = currentBranchName();
    if (!branch) {
      loadSessionSummary();
      return;
    }

    const tabExists = hasAgentTabForBranch(branch);
    const focused = tabExists && isAgentTabFocusedForBranch(branch);
    const pollIntervalMs = tabExists
      ? focused
        ? SESSION_SUMMARY_POLL_FOCUSED_INTERVAL_MS
        : SESSION_SUMMARY_POLL_NONFOCUSED_INTERVAL_MS
      : null;

    loadSessionSummary({ cachedOnly: !tabExists });

    if (pollIntervalMs === null) {
      return;
    }

    const timer = window.setInterval(() => {
      if (
        sessionSummaryStatus === "disabled" ||
        sessionSummaryStatus === "ai-not-configured"
      ) {
        return;
      }
      loadSessionSummary({ silent: true, cachedOnly: false });
    }, pollIntervalMs);

    return () => {
      window.clearInterval(timer);
    };
  });

  onMount(() => {
    let unlistenSummaryUpdated: null | (() => void) = null;
    let unlistenRebuildProgress: null | (() => void) = null;
    let cancelled = false;
    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const unlistenSummaryFn = await listen<SessionSummaryUpdatedPayload>(
          "session-summary-updated",
          (event) => {
            const payload = event.payload;
            if (!payload) return;
            if (payload.projectPath !== projectPath) return;

            const currentBranch = currentBranchName();
            if (!currentBranch || payload.branch !== currentBranch) return;

            const result = payload.result;
            const incomingSessionId = result.sessionId ?? null;
            if (!incomingSessionId) return;

            const currentSessionId = sessionSummarySessionId ?? null;
            if (currentSessionId && incomingSessionId !== currentSessionId) return;

            sessionSummaryStatus = result.status;
            sessionSummaryGenerating = !!result.generating;
            sessionSummaryMarkdown = result.markdown ?? null;
            sessionSummaryWarning = result.warning ?? null;
            sessionSummaryError = result.error ?? null;
            sessionSummaryToolId = result.toolId ?? null;
            sessionSummarySessionId = result.sessionId ?? null;
            sessionSummaryLanguage =
              result.language ?? normalizeSummaryLanguage(preferredLanguage);
            sessionSummarySourceType = result.sourceType ?? null;
            sessionSummaryInputMtimeMs = result.inputMtimeMs ?? null;
            sessionSummaryUpdatedMs = result.summaryUpdatedMs ?? null;
          }
        );
        const unlistenRebuildFn = await listen<SessionSummaryRebuildProgressPayload>(
          "session-summary-rebuild-progress",
          (event) => {
            const payload = event.payload;
            if (!payload) return;
            if (payload.projectPath !== projectPath) return;

            summaryRebuildTotal = payload.total ?? 0;
            summaryRebuildCompleted = payload.completed ?? 0;
            summaryRebuildBranch = payload.branch ?? null;
            if (payload.status === "started") {
              summaryRebuildError = null;
            } else if (payload.error) {
              summaryRebuildError = payload.error;
            }
            summaryRebuildInProgress = payload.status !== "completed";

            if (payload.status === "completed") {
              const branch = currentBranchName();
              if (!branch) return;
              const tabExists = hasAgentTabForBranch(branch);
              loadSessionSummary({
                silent: true,
                cachedOnly: !tabExists,
              });
            }
          }
        );
        if (cancelled) {
          unlistenSummaryFn();
          unlistenRebuildFn();
          return;
        }
        unlistenSummaryUpdated = unlistenSummaryFn;
        unlistenRebuildProgress = unlistenRebuildFn;
      } catch (err) {
        // Ignore when Tauri event bridge is unavailable (e.g., tests/web preview).
      }
    })();

    return () => {
      cancelled = true;
      if (unlistenSummaryUpdated) unlistenSummaryUpdated();
      if (unlistenRebuildProgress) unlistenRebuildProgress();
    };
  });

  function clearPrDetailState(nextBranch: string | null = null) {
    prDetailRequestToken++;
    prDetailLoading = false;
    prDetailError = null;
    updateBranchError = null;
    prDetail = null;
    prDetailBranch = nextBranch;
    prDetailPrNumber = null;
  }

  let resolvedPrNumber = $derived.by(() => {
    if (prNumber !== null && prNumber !== undefined) {
      return prNumber;
    }
    if (!latestBranchPr) return null;
    const branch = currentBranchName();
    if (branch && latestBranchPrBranch !== branch) return null;
    return latestBranchPr.number;
  });
  let prRetrying = $derived.by(() => {
    const status = selectedPrStatus;
    const prNum = resolvedPrNumber;
    if (!status || prNum === null) return false;
    if (status.number !== prNum) return false;
    return status.retrying === true;
  });

  $effect(() => {
    const nextProjectPath = projectPath ?? "";
    if (nextProjectPath === lastProjectPath) return;
    lastProjectPath = nextProjectPath;
    quickStartCache.clear();
    linkedIssueCache.clear();
    latestBranchPrCache.clear();
    latestBranchPr = null;
    latestBranchPrBranch = null;
    dockerContextCache.clear();
    clearPrDetailState(currentBranchName());
  });

  type LoadPrDetailOptions = {
    clearBeforeLoad?: boolean;
    showLoading?: boolean;
    errorPrefix?: string;
  };

  async function loadPrDetail(
    branch: string,
    prNum: number,
    options: LoadPrDetailOptions = {},
  ) {
    const clearBeforeLoad = options.clearBeforeLoad !== false;
    const showLoading = options.showLoading !== false;
    const errorPrefix = options.errorPrefix ?? null;
    const requestToken = ++prDetailRequestToken;
    if (showLoading) {
      prDetailLoading = true;
    }
    if (clearBeforeLoad) {
      prDetailError = null;
      prDetail = null;
    }
    prDetailPrNumber = prNum;
    try {
      const invoke = await getInvoke();
      const result = await invoke<PrStatusInfo>("fetch_pr_detail", {
        projectPath,
        prNumber: prNum,
      });
      const isCurrent =
        requestToken === prDetailRequestToken && prDetailBranch === branch;
      if (isCurrent) {
        prDetail = result;
      }
    } catch (err) {
      const isCurrent =
        requestToken === prDetailRequestToken && prDetailBranch === branch;
      if (isCurrent) {
        const message = toErrorMessage(err);
        prDetailError = errorPrefix ? `${errorPrefix}: ${message}` : message;
      }
    } finally {
      const isCurrent =
        requestToken === prDetailRequestToken && prDetailBranch === branch;
      if (isCurrent && showLoading) {
        prDetailLoading = false;
      }
    }
  }

  function isCurrentPrTarget(branch: string, prNum: number): boolean {
    if (currentBranchName() !== branch) return false;
    const currentPr = resolvedPrNumber;
    if (currentPr !== null && currentPr !== prNum) return false;
    return true;
  }

  function isViewingCurrentPr(branch: string, prNum: number): boolean {
    if (activeTab !== "pr") return false;
    return isCurrentPrTarget(branch, prNum);
  }

  async function pollPrDetailAfterBranchUpdate(
    branch: string,
    prNum: number,
  ): Promise<void> {
    for (let attempt = 0; attempt < UPDATE_BRANCH_POLL_ATTEMPTS; attempt++) {
      if (!isViewingCurrentPr(branch, prNum)) return;

      await loadPrDetail(branch, prNum, {
        clearBeforeLoad: false,
        showLoading: false,
        errorPrefix: "Failed to refresh PR detail",
      });

      if (!isViewingCurrentPr(branch, prNum)) return;
      if (prDetailError) return;
      if (prDetail?.mergeStateStatus !== "BEHIND") return;

      if (attempt < UPDATE_BRANCH_POLL_ATTEMPTS - 1) {
        await waitMs(UPDATE_BRANCH_POLL_INTERVAL_MS);
      }
    }
  }

  $effect(() => {
    if (activeTab !== "pr") return;

    const branch = currentBranchName();
    const prNum = resolvedPrNumber;
    if (!branch || !prNum) {
      const nextBranch = branch || null;
      if (
        prDetailBranch !== nextBranch ||
        prDetail !== null ||
        prDetailError !== null ||
        prDetailLoading ||
        prDetailPrNumber !== null
      ) {
        clearPrDetailState(nextBranch);
      }
      return;
    }

    if (branch !== prDetailBranch || prNum !== prDetailPrNumber) {
      prDetailBranch = branch;
      loadPrDetail(branch, prNum);
    }
  });

  async function quickLaunch(entry: ToolSessionEntry, action: "continue" | "new") {
    if (!selectedBranch) return;
    if (!onQuickLaunch) return;
    if (quickLaunching) return;

    quickLaunchError = null;
    quickLaunching = true;
    quickLaunchingKey = quickStartEntryKey(entry);
    try {
      const agentId = agentIdForToolId(entry.tool_id);
      const mode = action === "continue" ? "continue" : "normal";
      const resumeSessionId =
        action === "continue"
          ? entry.session_id?.trim() || undefined
          : undefined;

      const request: LaunchAgentRequest = {
        agentId,
        branch: selectedBranch.name,
        mode,
        resumeSessionId,
        model: entry.model?.trim() || undefined,
        agentVersion: displayToolVersion(entry),
        skipPermissions: entry.skip_permissions ?? undefined,
        reasoningLevel: entry.reasoning_level?.trim() || undefined,
        dockerService: entry.docker_service?.trim() || undefined,
        dockerForceHost: entry.docker_force_host ?? undefined,
        dockerRecreate: entry.docker_recreate ?? undefined,
        dockerBuild: entry.docker_build ?? undefined,
        dockerKeep: entry.docker_keep ?? undefined,
      };

      await onQuickLaunch(request);
    } catch (err) {
      quickLaunchError = `Failed to launch: ${toErrorMessage(err)}`;
    } finally {
      quickLaunching = false;
      quickLaunchingKey = null;
    }
  }

  async function handleCheckFixDocsAndEdit() {
    if (docsActionBusy) return;
    const branch = currentBranchName();
    if (!branch) {
      docsActionError = "Select a branch before checking docs.";
      return;
    }

    docsActionBusy = true;
    docsActionError = null;
    try {
      const invoke = await getInvoke();
      const result = await invoke<InstructionDocsCheckResult>(
        "check_and_fix_agent_instruction_docs",
        {
          projectPath,
          branch,
        },
      );

      const updatedCount = result.updatedFiles?.length ?? 0;
      const suffix = updatedCount === 0 ? "No changes needed." : `Updated ${updatedCount} file(s).`;
      toastBus.emit({ message: `Docs check complete. ${suffix}` });

      if (onOpenDocsEditor) {
        await onOpenDocsEditor(result.worktreePath);
      }
    } catch (err) {
      docsActionError = `Failed to check/fix docs: ${toErrorMessage(err)}`;
    } finally {
      docsActionBusy = false;
    }
  }

  let updatingBranch = $state(false);
  let merging = $state(false);
  let showMergeConfirm = $state(false);
  let mergeConfirmContextKey: string | null = $state(null);

  function currentPrContextKey(): string | null {
    const branch = currentBranchName();
    const prNum = resolvedPrNumber;
    if (!branch || !prNum) return null;
    return `${branch}#${prNum}`;
  }

  $effect(() => {
    if (!showMergeConfirm) return;
    const nextContextKey = currentPrContextKey();
    if (activeTab !== "pr" || !nextContextKey || nextContextKey !== mergeConfirmContextKey) {
      closeMergeConfirm();
    }
  });

  function handleOpenCiLog(run: WorkflowRunInfo): void {
    if (onOpenCiLog) {
      onOpenCiLog(run.runId);
      return;
    }

    const prUrl = prDetail?.url ?? "";
    const match = prUrl.match(/^(https:\/\/github\.com\/[^/]+\/[^/]+)\//);
    const workflowBase = match ? match[1] : null;
    if (!workflowBase) return;
    void openExternalUrl(`${workflowBase}/actions/runs/${run.runId}`);
  }

  async function handleUpdateBranch(): Promise<void> {
    if (!prDetail || updatingBranch) return;
    const branch = currentBranchName();
    const prNum = prDetail.number;
    updatingBranch = true;
    updateBranchError = null;
    try {
      const invoke = await getInvoke();
      await invoke<string>("update_pr_branch", {
        projectPath,
        prNumber: prNum,
      });

      if (branch) {
        prDetailBranch = branch;
        await pollPrDetailAfterBranchUpdate(branch, prNum);
      }
    } catch (err) {
      updateBranchError = `Failed to update branch: ${toErrorMessage(err)}`;
      console.error("Failed to update branch:", err);
    } finally {
      updatingBranch = false;
    }
  }

  function handleMerge(): void {
    mergeConfirmContextKey = currentPrContextKey();
    showMergeConfirm = true;
  }

  function closeMergeConfirm(): void {
    showMergeConfirm = false;
    mergeConfirmContextKey = null;
  }

  async function pollPrDetailAfterMerge(
    branch: string,
    prNum: number,
  ): Promise<void> {
    for (let attempt = 0; attempt < MERGE_POLL_ATTEMPTS; attempt++) {
      if (!isCurrentPrTarget(branch, prNum)) return;

      await loadPrDetail(branch, prNum, {
        clearBeforeLoad: false,
        showLoading: false,
        errorPrefix: "Failed to refresh PR detail",
      });

      if (!isCurrentPrTarget(branch, prNum)) return;
      if (prDetailError) return;
      if (prDetail?.state === "MERGED") return;

      if (attempt < MERGE_POLL_ATTEMPTS - 1) {
        await waitMs(MERGE_POLL_INTERVAL_MS);
      }
    }
  }

  async function confirmMerge(): Promise<void> {
    if (!prDetail || merging) return;
    const branch = currentBranchName();
    const prNum = prDetail.number;
    showMergeConfirm = false;
    merging = true;
    try {
      const invoke = await getInvoke();
      await invoke<string>("merge_pull_request", {
        projectPath,
        prNumber: prNum,
      });

      toastBus.emit({ message: `PR #${prNum} merged successfully` });

      if (branch) {
        await pollPrDetailAfterMerge(branch, prNum);
      }
    } catch (err) {
      toastBus.emit({
        message: `Failed to merge PR: ${toErrorMessage(err)}`,
        durationMs: 8000,
      });
      console.error("Failed to merge PR:", err);
    } finally {
      merging = false;
    }
  }

  async function getInvoke(): Promise<TauriInvoke> {
    const globalInvoke = (globalThis as { __TAURI_INTERNALS__?: { invoke?: TauriInvoke } })
      .__TAURI_INTERNALS__?.invoke;
    const invokeFn = globalInvoke ?? tauriInvoke;
    if (!invokeFn) {
      throw new Error("Tauri invoke API is unavailable");
    }
    return invokeFn;
  }
</script>

<div class="worktree-summary-panel">
  {#if selectedBranch}
    <div class="branch-detail">
      <div class="branch-header">
        <h2>{selectedBranch.name}</h2>
        <div class="branch-header-actions">
          <button
            class="header-quick-btn"
            disabled={quickHeaderButtonsDisabled}
            onclick={() => latestQuickStartEntry && quickLaunch(latestQuickStartEntry, "continue")}
          >
            {quickLaunching &&
            latestQuickStartEntry &&
            quickLaunchingKey === quickStartEntryKey(latestQuickStartEntry)
              ? "Launching..."
              : "Continue"}
          </button>
          <button
            class="header-quick-btn ghost"
            disabled={quickHeaderButtonsDisabled}
            onclick={() => latestQuickStartEntry && quickLaunch(latestQuickStartEntry, "new")}
          >
            New
          </button>
          <button
            class="header-quick-btn ghost"
            disabled={docsActionBusy}
            onclick={handleCheckFixDocsAndEdit}
          >
            {docsActionBusy ? "Checking..." : "Check/Fix Docs + Edit"}
          </button>
          <button
            class="new-terminal-btn"
            title="New Terminal"
            onclick={() => onNewTerminal?.()}
          >
            &gt;_
          </button>
          <button class="launch-btn" onclick={() => onLaunchAgent?.()}>
            Launch Agent...
          </button>
        </div>
      </div>

      {#if quickStartError}
        <div class="quick-error">{quickStartError}</div>
      {/if}
      {#if quickLaunchError}
        <div class="quick-error">{quickLaunchError}</div>
      {/if}
      {#if docsActionError}
        <div class="quick-error">{docsActionError}</div>
      {/if}

      <div class="summary-tabs">
        <button
          class="summary-tab"
          class:active={activeTab === "summary"}
          onclick={() => (activeTab = "summary")}
        >
          Summary
        </button>
        <button
          class="summary-tab"
          class:active={activeTab === "git"}
          onclick={() => {
            activeTab = "git";
          }}
        >
          Git
        </button>
        <button
          class="summary-tab"
          class:active={activeTab === "issue"}
          onclick={() => {
            activeTab = "issue";
          }}
        >
          Issue
        </button>
        <button
          class="summary-tab"
          class:active={activeTab === "pr"}
          onclick={() => {
            activeTab = "pr";
          }}
        >
          PR
        </button>
        <button
          class="summary-tab"
          class:active={activeTab === "docker"}
          onclick={() => {
            activeTab = "docker";
          }}
        >
          Docker
        </button>
      </div>

      {#if activeTab === "summary"}
        <div class="quick-start ai-summary">
          <div class="quick-header">
            <span class="quick-title">Summary</span>
            {#if sessionSummaryHeaderText}
              <span class="quick-subtitle" class:rebuild-progress={summaryRebuildInProgress}>
                {#if summaryRebuildInProgress}
                  <span class="summary-spinner" aria-hidden="true"></span>
                {/if}
                {sessionSummaryHeaderText}
              </span>
            {/if}
          </div>

          {#if sessionSummaryHasIdentity}
            {#if sessionSummaryHasMeta}
              <div class="session-summary-meta">
                <span class="meta-item">
                  Source: {sessionSummarySourceLabel(
                    sessionSummarySourceType,
                    sessionSummarySessionId,
                  )}
                </span>
                <span class="meta-item">Language: {sessionSummaryLanguageLabel}</span>
                {#if sessionSummaryInputTime}
                  <span class="meta-item">Input updated: {sessionSummaryInputTime}</span>
                {/if}
                {#if sessionSummaryUpdatedTime}
                  <span class="meta-item">Summary updated: {sessionSummaryUpdatedTime}</span>
                {/if}
              </div>
            {/if}
          {/if}

          {#if sessionSummaryWarning}
            <div class="session-summary-warning">
              {sessionSummaryWarning}
            </div>
          {/if}
          {#if summaryRebuildError && !summaryRebuildInProgress}
            <div class="session-summary-warning">
              Rebuild warning: {summaryRebuildError}
            </div>
          {/if}

          {#if sessionSummaryLoading}
            <div class="session-summary-placeholder">Loading...</div>
          {:else if sessionSummaryStatus === "ok" && sessionSummaryGenerating && !sessionSummaryMarkdown}
            <div class="session-summary-placeholder">Generating...</div>
          {:else if sessionSummaryStatus === "ai-not-configured"}
            <div class="session-summary-placeholder">
              Configure AI in Settings to enable session summary.
            </div>
          {:else if sessionSummaryStatus === "disabled"}
            <div class="session-summary-placeholder">Session summary disabled.</div>
          {:else if sessionSummaryStatus === "no-session"}
            <div class="session-summary-placeholder">No session.</div>
          {:else if sessionSummaryStatus === "error"}
            <div class="quick-error">
              {sessionSummaryError ?? "Failed to generate session summary."}
            </div>
          {:else if sessionSummaryStatus === "ok" && sessionSummaryMarkdown}
            <MarkdownRenderer
              className="session-summary-markdown"
              text={sessionSummaryMarkdown}
            />
          {:else}
            <div class="session-summary-placeholder">No summary.</div>
          {/if}
        </div>
      {:else if activeTab === "git"}
        <div class="detail-grid">
          <div class="detail-item">
            <span class="detail-label">Commit</span>
            <span class="detail-value mono">{selectedBranch.commit}</span>
          </div>
          <div class="detail-item">
            <span class="detail-label">Status</span>
            <span class="detail-value">
              {selectedBranch.divergence_status}
              {#if selectedBranch.ahead > 0}
                (+{selectedBranch.ahead})
              {/if}
              {#if selectedBranch.behind > 0}
                (-{selectedBranch.behind})
              {/if}
            </span>
          </div>
          <div class="detail-item">
            <span class="detail-label">Current</span>
            <span class="detail-value">{selectedBranch.is_current ? "Yes" : "No"}</span>
          </div>
        </div>
        <GitSection
          projectPath={projectPath}
          branch={selectedBranch.name}
          collapsible={false}
          defaultCollapsed={false}
        />
      {:else if activeTab === "issue"}
        <div class="quick-start issue-panel">
          <div class="quick-header">
            <span class="quick-title">Issue</span>
            {#if linkedIssueLoading}
              <span class="quick-subtitle">Loading...</span>
            {:else if linkedIssueError}
              <span class="quick-subtitle">Error</span>
            {:else if linkedIssue}
              <span class="quick-subtitle">#{linkedIssue.number}</span>
            {:else}
              <span class="quick-subtitle">No linked issue</span>
            {/if}
          </div>

          {#if linkedIssueLoading}
            <div class="session-summary-placeholder">Loading...</div>
          {:else if linkedIssueError}
            <div class="quick-error">{linkedIssueError}</div>
          {:else if !linkedIssue}
            <div class="session-summary-placeholder">
              No issue linked to this branch.
            </div>
          {:else}
            {@const issueUpdated = formatIsoTimestamp(linkedIssue.updatedAt)}
            <div class="linked-issue-card">
              <a
                class="linked-issue-title"
                href={linkedIssue.url}
                target="_blank"
                rel="noopener noreferrer"
              >
                {linkedIssueTitle(linkedIssue)}
              </a>
              <div class="quick-meta">
                {#if issueUpdated}
                  <span class="quick-pill">updated: {issueUpdated}</span>
                {/if}
                {#if linkedIssue.labels.length === 0}
                  <span class="quick-pill">labels: none</span>
                {:else}
                  {#each linkedIssue.labels as label}
                    <span class="quick-pill">label: {label}</span>
                  {/each}
                {/if}
              </div>
            </div>
          {/if}
        </div>
      {:else if activeTab === "pr"}
        <PrStatusSection
          prDetail={prDetail}
          loading={latestBranchPrLoading || (resolvedPrNumber !== null && prDetailLoading)}
          error={ghCliStatusMessage ?? latestBranchPrError ?? prDetailError}
          updateError={updateBranchError}
          onOpenCiLog={handleOpenCiLog}
          onUpdateBranch={handleUpdateBranch}
          updatingBranch={updatingBranch}
          onMerge={handleMerge}
          {merging}
          retrying={prRetrying}
        />
        <MergeConfirmModal
          open={showMergeConfirm}
          {prDetail}
          {merging}
          onClose={closeMergeConfirm}
          onConfirm={confirmMerge}
        />
      {:else if activeTab === "docker"}
        <div class="quick-start docker-summary">
          <div class="quick-header">
            <span class="quick-title">Docker</span>
            {#if dockerContextLoading}
              <span class="quick-subtitle">Detecting...</span>
            {:else if dockerContext}
              <span class="quick-subtitle">Current: {dockerContext.file_type}</span>
            {:else if dockerContextError}
              <span class="quick-subtitle">Current: error</span>
            {:else}
              <span class="quick-subtitle">Current: n/a</span>
            {/if}
          </div>

          {#if dockerContextLoading}
            <div class="session-summary-placeholder">Detecting Docker context...</div>
          {:else if dockerContextError}
            <div class="quick-error">{dockerContextError}</div>
          {:else if dockerContext}
            <div class="docker-current">
              <div class="quick-meta">
                <span class="quick-pill">type: {dockerContext.file_type}</span>
                <span class="quick-pill">
                  docker: {dockerContext.docker_available ? "available" : "unavailable"}
                </span>
                <span class="quick-pill">
                  compose: {dockerContext.compose_available ? "available" : "unavailable"}
                </span>
                <span class="quick-pill">
                  daemon: {dockerContext.daemon_running ? "running" : "stopped"}
                </span>
                <span class="quick-pill">
                  force-host: {dockerContext.force_host ? "on" : "off"}
                </span>
                {#if dockerContext.worktree_path}
                  <span class="quick-pill">worktree: {dockerContext.worktree_path}</span>
                {/if}
                {#if dockerContext.compose_services.length > 0}
                  <span class="quick-pill">
                    services: {dockerContext.compose_services.join(", ")}
                  </span>
                {/if}
              </div>
            </div>
          {:else}
            <div class="session-summary-placeholder">No Docker context.</div>
          {/if}

          <div class="quick-header">
            <span class="quick-title">Quick Start history</span>
            {#if quickStartLoading}
              <span class="quick-subtitle">Loading...</span>
            {:else if dockerSummaryRows.length > 0}
              <span class="quick-subtitle">
                {dockerSummaryRows.length} record{dockerSummaryRows.length === 1 ? "" : "s"}
              </span>
            {:else}
              <span class="quick-subtitle">No Docker records</span>
            {/if}
          </div>

          {#if quickStartLoading}
            <div class="session-summary-placeholder">Loading...</div>
          {:else if dockerSummaryRows.length === 0}
            <div class="session-summary-placeholder">
              No Docker usage found in quick start history.
            </div>
          {:else}
            <div class="docker-summary-list">
              {#each dockerSummaryRows as row (quickStartEntryKey(row.entry))}
                <div class="docker-summary-item">
                  <div class="docker-summary-head">
                    <div class="docker-summary-identity">
                      <div class="quick-tool {toolClass(row.entry)}">
                        <span class="quick-tool-name">{displayToolName(row.entry)}</span>
                        <span class="quick-tool-version">@{displayToolVersion(row.entry)}</span>
                      </div>
                      {#if row.entry.session_id}
                        <div class="docker-summary-session">Session {row.entry.session_id}</div>
                      {/if}
                    </div>
                    <span class="docker-summary-time">{formatTimestamp(row.entry.timestamp)}</span>
                  </div>
                  <div class="quick-meta">
                    <span class={`quick-pill ${row.modeClass}`}>runtime: {row.mode}</span>
                    {#if row.service}
                      <span class="quick-pill">service: {row.service}</span>
                    {/if}
                    {#if row.entry.docker_force_host !== undefined && row.entry.docker_force_host !== null}
                      <span class="quick-pill">
                        force-host: {row.entry.docker_force_host ? "on" : "off"}
                      </span>
                    {/if}
                    {#if row.entry.docker_recreate !== undefined}
                      <span class="quick-pill">recreate: {row.entry.docker_recreate ? "on" : "off"}</span>
                    {/if}
                    {#if row.entry.docker_build !== undefined}
                      <span class="quick-pill">build: {row.entry.docker_build ? "on" : "off"}</span>
                    {/if}
                    {#if row.entry.docker_keep !== undefined}
                      <span class="quick-pill">keep: {row.entry.docker_keep ? "on" : "off"}</span>
                    {/if}
                    {#if row.containerName}
                      <span class="quick-pill">container: {row.containerName}</span>
                    {/if}
                    {#if row.composeArgs}
                      <span class="quick-pill">compose args: {row.composeArgs}</span>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {:else}
    <div class="placeholder">
      <h2>Worktree Summary</h2>
      <p>Select a branch to view details.</p>
    </div>
  {/if}
</div>

<style>
  .worktree-summary-panel {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    color: var(--text-muted);
    text-align: center;
    min-height: 120px;
  }

  .placeholder h2 {
    font-size: var(--ui-font-lg);
    font-weight: 600;
    color: var(--text-secondary);
  }

  .placeholder p {
    font-size: var(--ui-font-sm);
  }

  .branch-detail {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .branch-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }

  .branch-header-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    flex-wrap: wrap;
    margin-left: auto;
    min-width: 0;
    flex-shrink: 1;
  }

  .branch-detail h2 {
    margin: 0;
    font-size: var(--ui-font-lg);
    font-weight: 700;
    color: var(--text-primary);
    font-family: monospace;
    flex: 1 1 auto;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .launch-btn {
    background: var(--accent);
    color: var(--bg-primary);
    border: none;
    border-radius: 8px;
    padding: 6px 10px;
    font-size: var(--ui-font-sm);
    font-weight: 600;
    cursor: pointer;
    font-family: inherit;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .launch-btn:hover {
    background: var(--accent-hover);
  }

  .new-terminal-btn {
    background: var(--bg-surface);
    color: var(--text-primary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    padding: 6px 10px;
    font-size: var(--ui-font-sm);
    font-weight: 700;
    font-family: monospace;
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .new-terminal-btn:hover {
    border-color: var(--accent);
  }

  .header-quick-btn {
    padding: 6px 10px;
    border-radius: 8px;
    border: 1px solid var(--border-color);
    background: var(--bg-surface);
    color: var(--text-primary);
    font-size: var(--ui-font-sm);
    font-weight: 700;
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s, background-color 0.15s;
    white-space: nowrap;
  }

  .header-quick-btn:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .header-quick-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .header-quick-btn.ghost {
    background: transparent;
    color: var(--text-secondary);
  }

  .detail-grid {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .detail-item {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }

  .detail-label {
    font-size: var(--ui-font-xs);
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    min-width: 56px;
    flex-shrink: 0;
  }

  .detail-value {
    font-size: var(--ui-font-sm);
    color: var(--text-primary);
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .detail-value.mono {
    font-family: monospace;
  }

  .quick-start {
    border: 1px solid var(--border-color);
    border-radius: 12px;
    background: var(--bg-secondary);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .quick-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  .quick-title {
    font-size: var(--ui-font-sm);
    font-weight: 700;
    letter-spacing: 0.5px;
    text-transform: uppercase;
    color: var(--text-secondary);
  }

  .quick-subtitle {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    font-family: monospace;
    text-align: right;
  }

  .quick-subtitle.rebuild-progress {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }

  .summary-spinner {
    width: 12px;
    height: 12px;
    border: 2px solid rgba(255, 255, 255, 0.25);
    border-top-color: rgba(255, 255, 255, 0.75);
    border-radius: 999px;
    animation: summary-spin 0.8s linear infinite;
  }

  @keyframes summary-spin {
    from {
      transform: rotate(0deg);
    }
    to {
      transform: rotate(360deg);
    }
  }

  .session-summary-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    font-family: monospace;
    line-height: 1.4;
  }

  .session-summary-meta .meta-item {
    white-space: nowrap;
  }

  .quick-error {
    padding: 10px 12px;
    border: 1px solid rgba(255, 0, 0, 0.35);
    background: rgba(255, 0, 0, 0.08);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: var(--ui-font-sm);
    line-height: 1.4;
  }

  .session-summary-warning {
    padding: 10px 12px;
    border: 1px solid rgba(249, 226, 175, 0.35);
    background: rgba(249, 226, 175, 0.08);
    border-radius: 8px;
    color: var(--text-primary);
    font-size: var(--ui-font-sm);
    line-height: 1.4;
  }

  .session-summary-placeholder {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    line-height: 1.4;
  }

  .quick-empty {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    line-height: 1.4;
  }

  .quick-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .linked-issue-card {
    border: 1px solid var(--border-color);
    background: var(--bg-primary);
    border-radius: 10px;
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .linked-issue-title {
    color: var(--accent);
    text-decoration: none;
    font-weight: 700;
    font-size: var(--ui-font-sm);
    overflow-wrap: anywhere;
  }

  .linked-issue-title:hover {
    text-decoration: underline;
  }

  .quick-row {
    display: flex;
    flex-direction: column;
    gap: 10px;
    border: 1px solid var(--border-color);
    background: var(--bg-primary);
    border-radius: 10px;
    padding: 10px 12px;
  }

  .quick-info {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .quick-tool {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-family: monospace;
    min-width: 0;
  }

  .quick-tool-name {
    font-size: var(--ui-font-sm);
    font-weight: 700;
  }

  .quick-tool-version {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
  }

  .quick-tool.claude .quick-tool-name {
    color: var(--yellow);
  }

  .quick-tool.codex .quick-tool-name {
    color: var(--cyan);
  }

  .quick-tool.gemini .quick-tool-name {
    color: var(--magenta);
  }

  .quick-tool.opencode .quick-tool-name {
    color: var(--green);
  }

  .quick-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    color: var(--text-muted);
    font-size: var(--ui-font-xs);
  }

  .quick-pill {
    border: 1px solid var(--border-color);
    background: var(--bg-secondary);
    border-radius: 999px;
    padding: 2px 8px;
    font-family: monospace;
  }

  .quick-pill.hostos {
    border-color: var(--cyan);
    color: var(--cyan);
  }

  .quick-pill.docker {
    border-color: var(--green);
    color: var(--green);
  }

  .quick-pill.unknown {
    border-color: var(--text-muted);
    color: var(--text-muted);
  }

  .quick-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    justify-content: flex-end;
    flex-wrap: wrap;
  }

  .quick-btn {
    padding: 7px 10px;
    border-radius: 8px;
    border: 1px solid var(--border-color);
    background: var(--bg-surface);
    color: var(--text-primary);
    font-size: var(--ui-font-sm);
    font-weight: 700;
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.15s, background-color 0.15s;
  }

  .quick-btn:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .quick-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .quick-btn.ghost {
    background: transparent;
    color: var(--text-secondary);
  }

  .docker-summary-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .docker-current {
    border: 1px solid var(--border-color);
    background: var(--bg-primary);
    border-radius: 10px;
    padding: 10px 12px;
  }

  .docker-summary-item {
    border: 1px solid var(--border-color);
    background: var(--bg-primary);
    border-radius: 10px;
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .docker-summary-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .docker-summary-identity {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .docker-summary-session {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    font-family: monospace;
  }

  .docker-summary-time {
    font-size: var(--ui-font-xs);
    color: var(--text-muted);
    font-family: monospace;
    text-align: right;
    white-space: nowrap;
  }

	  .session-summary-markdown {
	    border: 1px solid var(--border-color);
	    border-radius: 10px;
	    background: var(--bg-primary);
	    padding: 10px 12px;
	    overflow: hidden;
	    margin: 0;
	  }

  .summary-tabs {
    display: flex;
    gap: 0;
    border-bottom: 1px solid var(--border-color);
    margin-bottom: 10px;
  }

  .summary-tab {
    padding: 6px 16px;
    border: none;
    background: none;
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
    font-weight: 600;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    font-family: inherit;
  }

  .summary-tab.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }

  .summary-tab:hover:not(.active) {
    color: var(--text-secondary);
  }

	</style>
