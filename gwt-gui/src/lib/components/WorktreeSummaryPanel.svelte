<script lang="ts">
  import { onMount } from "svelte";
  import { invoke as tauriInvoke } from "@tauri-apps/api/core";
  import type {
    BranchInfo,
    BranchLinkedIssueInfo,
    BranchPrReference,
    DockerContext,
    LaunchAgentRequest,
    ToolSessionEntry,
    SessionSummaryResult,
    PrStatusInfo,
    GhCliStatus,
    WorkflowRunInfo,
    SettingsData,
  } from "../types";
  import GitSection from "./GitSection.svelte";
  import MarkdownRenderer from "./MarkdownRenderer.svelte";
  import PrStatusSection from "./PrStatusSection.svelte";
  import { workflowStatusIcon, workflowStatusClass } from "../prStatusHelpers";

  let {
    projectPath,
    selectedBranch = null,
    onLaunchAgent,
    onQuickLaunch,
    onOpenCiLog,
    agentTabBranches = [],
    activeAgentTabBranch = null,
    preferredLanguage = "auto",
    prNumber = null,
    ghCliStatus = null,
  }: {
    projectPath: string;
    selectedBranch?: BranchInfo | null;
    onLaunchAgent?: () => void;
    onQuickLaunch?: (request: LaunchAgentRequest) => Promise<void>;
    onOpenCiLog?: (runId: number) => void;
    agentTabBranches?: string[];
    activeAgentTabBranch?: string | null;
    preferredLanguage?: SettingsData["app_language"];
    prNumber?: number | null;
    ghCliStatus?: GhCliStatus | null;
  } = $props();

  let quickStartEntries: ToolSessionEntry[] = $state([]);
  let quickStartLoading: boolean = $state(false);
  let quickStartError: string | null = $state(null);
  let quickLaunchError: string | null = $state(null);
  let quickLaunching: boolean = $state(false);
  let quickLaunchingKey: string | null = $state(null);

  type SummaryTab =
    | "quick-start"
    | "summary"
    | "git"
    | "issue"
    | "pr"
    | "workflow"
    | "docker";
  let activeTab: SummaryTab = $state("quick-start");

  let linkedIssueLoading: boolean = $state(false);
  let linkedIssueError: string | null = $state(null);
  let linkedIssue: BranchLinkedIssueInfo | null = $state(null);

  let latestBranchPrLoading: boolean = $state(false);
  let latestBranchPrError: string | null = $state(null);
  let latestBranchPr: BranchPrReference | null = $state(null);

  let dockerContextLoading: boolean = $state(false);
  let dockerContextError: string | null = $state(null);
  let dockerContext: DockerContext | null = $state(null);

  let prDetailLoading = $state(false);
  let prDetailError: string | null = $state(null);
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
  const SESSION_SUMMARY_POLL_FOCUSED_INTERVAL_MS = 15000;
  const SESSION_SUMMARY_POLL_NONFOCUSED_INTERVAL_MS = 60000;

  type DockerMode = "HostOS" | "Docker" | "Unknown";
  type DockerModeClass = "hostos" | "docker" | "unknown";
  type DockerSummaryRow = {
    entry: ToolSessionEntry;
    mode: DockerMode;
    modeClass: DockerModeClass;
    composeArgs: string | null;
    service: string | null;
    containerName: string | null;
  };

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

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  type TauriInvoke = <T>(
    command: string,
    args?: Record<string, unknown>,
  ) => Promise<T>;

  function normalizeBranchName(name: string): string {
    return name.startsWith("origin/") ? name.slice("origin/".length) : name;
  }

  function currentBranchName(): string {
    const rawBranch = selectedBranch?.name?.trim() ?? "";
    return normalizeBranchName(rawBranch);
  }

  function normalizeSummaryLanguage(value: string | null | undefined): string {
    const language = (value ?? "").trim().toLowerCase();
    if (language === "ja" || language === "en" || language === "auto") {
      return language;
    }
    return "auto";
  }

  function summaryLanguageLabel(value: string | null): string | null {
    const language = normalizeSummaryLanguage(value);
    if (language === "ja") return "Japanese";
    if (language === "en") return "English";
    return language === "auto" ? "Auto" : null;
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

  function formatSessionSummaryTimestamp(ms: number | null): string | null {
    if (ms === null || !Number.isFinite(ms) || ms <= 0) return null;
    const d = new Date(ms);
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
  }

  function agentIdForToolId(toolId: string): LaunchAgentRequest["agentId"] {
    const key = (toolId ?? "").toLowerCase();
    if (key.includes("claude")) return "claude";
    if (key.includes("codex")) return "codex";
    if (key.includes("gemini")) return "gemini";
    if (key.includes("opencode") || key.includes("open-code")) return "opencode";
    return toolId as LaunchAgentRequest["agentId"];
  }

  function toolClass(entry: ToolSessionEntry): string {
    const id = entry.tool_id?.toLowerCase() ?? "";
    if (id.includes("claude")) return "claude";
    if (id.includes("codex")) return "codex";
    if (id.includes("gemini")) return "gemini";
    if (id.includes("opencode") || id.includes("open-code")) return "opencode";
    return "";
  }

  function displayToolName(entry: ToolSessionEntry): string {
    const id = entry.tool_id?.toLowerCase() ?? "";
    if (id.includes("claude")) return "Claude";
    if (id.includes("codex")) return "Codex";
    if (id.includes("gemini")) return "Gemini";
    if (id.includes("opencode") || id.includes("open-code")) return "OpenCode";
    return entry.tool_label || entry.tool_id;
  }

  function displayToolVersion(entry: ToolSessionEntry): string {
    const v = entry.tool_version?.trim();
    return v && v.length > 0 ? v : "latest";
  }

  function displayModelLabel(entry: ToolSessionEntry): string | null {
    const model = entry.model?.trim();
    if (model) return model;
    const tool = entry.tool_id?.toLowerCase() ?? "";
    if (
      tool.includes("codex") ||
      tool.includes("claude") ||
      tool.includes("gemini") ||
      tool.includes("opencode") ||
      tool.includes("open-code")
    ) {
      return "default";
    }
    return null;
  }

  function runtimeLabel(entry: ToolSessionEntry): string | null {
    if (entry.docker_force_host === true) {
      return "HostOS";
    }

    const hasDockerService = (entry.docker_service ?? "").trim().length > 0;
    if (entry.docker_recreate !== undefined) return "Docker";
    if (entry.docker_build !== undefined) return "Docker";
    if (entry.docker_keep !== undefined) return "Docker";
    if (hasDockerService) return "Docker";
    if (entry.docker_force_host === false) return "Docker";

    return null;
  }

  function runtimeService(entry: ToolSessionEntry): string | null {
    const service = (entry.docker_service ?? "").trim();
    return service.length > 0 ? service : null;
  }

  function normalizeString(value: string | null | undefined): string {
    return (value ?? "").trim();
  }

  function hasDockerInfo(entry: ToolSessionEntry): boolean {
    if (entry.docker_force_host !== undefined && entry.docker_force_host !== null)
      return true;
    if (normalizeString(entry.docker_service).length > 0) return true;
    if (normalizeString(entry.docker_container_name).length > 0) return true;
    if (entry.docker_compose_args && entry.docker_compose_args.length > 0) return true;
    if (entry.docker_recreate !== undefined) return true;
    if (entry.docker_build !== undefined) return true;
    if (entry.docker_keep !== undefined) return true;
    return false;
  }

  function dockerMode(entry: ToolSessionEntry): DockerMode {
    if (entry.docker_force_host === true) return "HostOS";
    if (hasDockerInfo(entry)) return "Docker";
    return "Unknown";
  }

  function dockerModeClass(entry: ToolSessionEntry): DockerModeClass {
    const mode = dockerMode(entry);
    if (mode === "HostOS") return "hostos";
    if (mode === "Docker") return "docker";
    return "unknown";
  }

  function formatComposeArgs(
    args: string[] | null | undefined
  ): string | null {
    if (!args || args.length === 0) return null;
    const normalized = args.map((arg) => normalizeString(arg)).filter((arg) => arg.length > 0);
    return normalized.length > 0 ? normalized.join(" ") : null;
  }

  function formatTimestamp(timestamp: number): string {
    const value = Number.isFinite(timestamp) ? new Date(timestamp).toLocaleString() : "n/a";
    return value;
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

  function quickStartEntryKey(entry: ToolSessionEntry): string {
    const session = entry.session_id?.trim();
    if (session) return session;
    return `${entry.tool_id}-${entry.timestamp}`;
  }

  async function loadQuickStart() {
    quickLaunchError = null;
    quickStartError = null;

    const branch = currentBranchName();
    if (!branch) {
      quickStartEntries = [];
      quickStartLoading = false;
      return;
    }

    const key = `${projectPath}::${branch}`;
    quickStartLoading = true;

    try {
      const invoke = await getInvoke();
      const entries = await invoke<ToolSessionEntry[]>("get_branch_quick_start", {
        projectPath,
        branch,
      });
      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey !== key) return;
      quickStartEntries = entries ?? [];
    } catch (err) {
      quickStartEntries = [];
      quickStartError = `Failed to load Quick Start: ${toErrorMessage(err)}`;
    } finally {
      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey === key) {
        quickStartLoading = false;
      }
    }
  }

  function formatIsoTimestamp(value: string | null | undefined): string | null {
    const raw = (value ?? "").trim();
    if (!raw) return null;
    const parsed = new Date(raw);
    if (Number.isNaN(parsed.getTime())) return raw;
    return parsed.toLocaleString();
  }

  async function loadBranchLinkedIssue() {
    linkedIssueError = null;

    const branch = currentBranchName();
    if (!branch) {
      linkedIssueLoading = false;
      linkedIssue = null;
      return;
    }

    const key = `${projectPath}::${branch}`;
    linkedIssueLoading = true;
    try {
      const invoke = await getInvoke();
      const result = await invoke<BranchLinkedIssueInfo | null>("fetch_branch_linked_issue", {
        projectPath,
        branch,
      });
      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey !== key) return;
      linkedIssue = result;
    } catch (err) {
      linkedIssue = null;
      linkedIssueError = `Failed to load linked issue: ${toErrorMessage(err)}`;
    } finally {
      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey === key) {
        linkedIssueLoading = false;
      }
    }
  }

  async function loadLatestBranchPr() {
    latestBranchPrError = null;

    const branch = currentBranchName();
    if (!branch) {
      latestBranchPrLoading = false;
      latestBranchPr = null;
      return;
    }

    const key = `${projectPath}::${branch}`;
    latestBranchPrLoading = true;
    try {
      const invoke = await getInvoke();
      const result = await invoke<BranchPrReference | null>("fetch_latest_branch_pr", {
        projectPath,
        branch,
      });
      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey !== key) return;
      latestBranchPr = result;
    } catch (err) {
      latestBranchPr = null;
      latestBranchPrError = `Failed to load PR: ${toErrorMessage(err)}`;
    } finally {
      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey === key) {
        latestBranchPrLoading = false;
      }
    }
  }

  async function loadDockerContext() {
    dockerContextError = null;

    const branch = currentBranchName();
    if (!branch) {
      dockerContextLoading = false;
      dockerContext = null;
      return;
    }

    const key = `${projectPath}::${branch}`;
    dockerContextLoading = true;
    try {
      const invoke = await getInvoke();
      const result = await invoke<DockerContext>("detect_docker_context", {
        projectPath,
        branch,
      });
      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey !== key) return;
      dockerContext = result;
    } catch (err) {
      dockerContext = null;
      dockerContextError = `Failed to detect Docker context: ${toErrorMessage(err)}`;
    } finally {
      const currentKey = `${projectPath}::${currentBranchName()}`;
      if (currentKey === key) {
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
    loadBranchLinkedIssue();
    loadLatestBranchPr();
    loadDockerContext();
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
    prDetail = null;
    prDetailBranch = nextBranch;
    prDetailPrNumber = null;
  }

  let resolvedPrNumber = $derived.by(() => latestBranchPr?.number ?? prNumber ?? null);
  let workflowDisplayPrNumber = $derived.by(
    () => latestBranchPr?.number ?? prDetail?.number ?? resolvedPrNumber,
  );

  $effect(() => {
    const nextProjectPath = projectPath ?? "";
    if (nextProjectPath === lastProjectPath) return;
    lastProjectPath = nextProjectPath;
    clearPrDetailState(currentBranchName());
  });

  async function loadPrDetail(branch: string, prNum: number) {
    const requestToken = ++prDetailRequestToken;
    prDetailLoading = true;
    prDetailError = null;
    prDetail = null;
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
        prDetailError = toErrorMessage(err);
      }
    } finally {
      const isCurrent =
        requestToken === prDetailRequestToken && prDetailBranch === branch;
      if (isCurrent) {
        prDetailLoading = false;
      }
    }
  }

  $effect(() => {
    if (activeTab !== "pr" && activeTab !== "workflow") return;

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

  function workflowStatusText(run: WorkflowRunInfo): string {
    if (run.status !== "completed") {
      return run.status === "in_progress" ? "Running" : "Queued";
    }
    switch (run.conclusion) {
      case "success":
        return "Success";
      case "failure":
        return "Failure";
      case "neutral":
        return "Neutral";
      case "skipped":
        return "Skipped";
      default:
        return "Completed";
    }
  }

  function openWorkflowRun(run: WorkflowRunInfo): void {
    if (onOpenCiLog) {
      onOpenCiLog(run.runId);
      return;
    }
    if (typeof window === "undefined" || !window.open) return;

    const prUrl = prDetail?.url ?? "";
    const match = prUrl.match(/^(https:\/\/github\.com\/[^/]+\/[^/]+)\//);
    const workflowBase = match ? match[1] : null;
    if (!workflowBase) return;
    window.open(`${workflowBase}/actions/runs/${run.runId}`, "_blank", "noopener");
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
        <button class="launch-btn" onclick={() => onLaunchAgent?.()}>
          Launch Agent...
        </button>
      </div>

      <div class="summary-tabs">
        <button
          class="summary-tab"
          class:active={activeTab === "quick-start"}
          onclick={() => (activeTab = "quick-start")}
        >
          Quick Start
        </button>
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
          class:active={activeTab === "workflow"}
          onclick={() => {
            activeTab = "workflow";
          }}
        >
          Workflow
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

      {#if activeTab === "quick-start"}
        <div class="quick-start">
          <div class="quick-header">
            <span class="quick-title">Quick Start</span>
            {#if quickStartLoading}
              <span class="quick-subtitle">Loading...</span>
            {:else if quickStartEntries.length > 0}
              <span class="quick-subtitle">
                {quickStartEntries.length} tool{quickStartEntries.length === 1 ? "" : "s"}
              </span>
            {:else}
              <span class="quick-subtitle">No history</span>
            {/if}
          </div>

          {#if quickStartError}
            <div class="quick-error">{quickStartError}</div>
          {/if}

          {#if quickLaunchError}
            <div class="quick-error">{quickLaunchError}</div>
          {/if}

          {#if !quickStartLoading && quickStartEntries.length === 0}
            <div class="quick-empty">
              Launch an agent once on this branch to enable Quick Start.
            </div>
          {:else if quickStartEntries.length > 0}
            <div class="quick-list">
              {#each quickStartEntries as entry (quickStartEntryKey(entry))}
                <div class="quick-row">
                  <div class="quick-info">
                    <div class="quick-tool {toolClass(entry)}">
                      <span class="quick-tool-name">{displayToolName(entry)}</span>
                      <span class="quick-tool-version">
                        @{displayToolVersion(entry)}
                      </span>
                    </div>
                    <div class="quick-meta">
                      {#if runtimeLabel(entry)}
                        <span class="quick-pill">runtime: {runtimeLabel(entry)}</span>
                      {/if}
                      {#if runtimeService(entry)}
                        <span class="quick-pill">service: {runtimeService(entry)}</span>
                      {/if}
                      {#if displayModelLabel(entry) !== null}
                        <span class="quick-pill">model: {displayModelLabel(entry)}</span>
                      {/if}
                      {#if toolClass(entry) === "codex" && entry.reasoning_level}
                        <span class="quick-pill">reasoning: {entry.reasoning_level}</span>
                      {/if}
                      {#if entry.skip_permissions !== undefined && entry.skip_permissions !== null}
                        <span class="quick-pill">
                          skip: {entry.skip_permissions ? "on" : "off"}
                        </span>
                      {/if}
                    </div>
                  </div>
                  <div class="quick-actions">
                    <button
                      class="quick-btn"
                      disabled={quickLaunching}
                      onclick={() => quickLaunch(entry, "continue")}
                    >
                      {quickLaunching && quickLaunchingKey === quickStartEntryKey(entry)
                        ? "Launching..."
                        : "Continue"}
                    </button>
                    <button
                      class="quick-btn ghost"
                      disabled={quickLaunching}
                      onclick={() => quickLaunch(entry, "new")}
                    >
                      New
                    </button>
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {:else if activeTab === "summary"}
        <div class="quick-start ai-summary">
          <div class="quick-header">
            <span class="quick-title">Summary</span>
            {#if summaryRebuildInProgress}
              <span class="quick-subtitle rebuild-progress">
                <span class="summary-spinner" aria-hidden="true"></span>
                Rebuilding summaries ({summaryRebuildCompleted}/{summaryRebuildTotal})
                {#if summaryRebuildBranch}
                  - {summaryRebuildBranch}
                {/if}
              </span>
            {:else if sessionSummaryLoading}
              <span class="quick-subtitle">Loading...</span>
            {:else if sessionSummaryStatus === "ok" && sessionSummaryToolId}
              <span class="quick-subtitle">
                {#if sessionSummarySessionId?.startsWith("pane:")}
                  {sessionSummaryToolId} - Live (pane summary)
                {:else if sessionSummarySessionId}
                  {sessionSummaryToolId} #{sessionSummarySessionId}
                {:else}
                  {sessionSummaryToolId}
                {/if}
                {#if sessionSummaryGenerating}
                  {sessionSummaryMarkdown ? " - Updating..." : " - Generating..."}
                {/if}
              </span>
            {:else if sessionSummaryStatus === "ai-not-configured"}
              <span class="quick-subtitle">AI not configured</span>
            {:else if sessionSummaryStatus === "disabled"}
              <span class="quick-subtitle">Disabled</span>
            {:else if sessionSummaryStatus === "no-session"}
              <span class="quick-subtitle">No session</span>
            {:else if sessionSummaryStatus === "error"}
              <span class="quick-subtitle">Error</span>
            {/if}
          </div>

          {#if sessionSummaryStatus === "ok" &&
            (sessionSummaryToolId || sessionSummarySessionId)}
            {@const inputTime = formatSessionSummaryTimestamp(sessionSummaryInputMtimeMs)}
            {@const updatedTime = formatSessionSummaryTimestamp(sessionSummaryUpdatedMs)}
            {@const languageLabel = summaryLanguageLabel(sessionSummaryLanguage)}
            {#if sessionSummarySourceType || languageLabel || inputTime || updatedTime}
              <div class="session-summary-meta">
                <span class="meta-item">
                  Source: {sessionSummarySourceType === "scrollback" ||
                  sessionSummarySessionId?.startsWith("pane:")
                    ? "Live (scrollback)"
                    : "Session"}
                </span>
                {#if languageLabel}
                  <span class="meta-item">Language: {languageLabel}</span>
                {/if}
                {#if inputTime}
                  <span class="meta-item">Input updated: {inputTime}</span>
                {/if}
                {#if updatedTime}
                  <span class="meta-item">Summary updated: {updatedTime}</span>
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
                #{linkedIssue.number} {linkedIssue.title}
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
        />
      {:else if activeTab === "workflow"}
        <div class="quick-start workflow-panel">
          <div class="quick-header">
            <span class="quick-title">Workflow</span>
            {#if latestBranchPrLoading || (resolvedPrNumber !== null && prDetailLoading)}
              <span class="quick-subtitle">Loading...</span>
            {:else if ghCliStatusMessage}
              <span class="quick-subtitle">GitHub CLI issue</span>
            {:else if latestBranchPrError || prDetailError}
              <span class="quick-subtitle">Error</span>
            {:else if workflowDisplayPrNumber !== null}
              <span class="quick-subtitle">#{workflowDisplayPrNumber}</span>
            {:else}
              <span class="quick-subtitle">No PR</span>
            {/if}
          </div>

          {#if latestBranchPrLoading || (resolvedPrNumber !== null && prDetailLoading)}
            <div class="session-summary-placeholder">Loading...</div>
          {:else if ghCliStatusMessage}
            <div class="quick-error">{ghCliStatusMessage}</div>
          {:else if latestBranchPrError || prDetailError}
            <div class="quick-error">{latestBranchPrError ?? prDetailError}</div>
          {:else if workflowDisplayPrNumber === null && !prDetail}
            <div class="session-summary-placeholder">No PR.</div>
          {:else if !prDetail}
            <div class="session-summary-placeholder">Loading PR details...</div>
          {:else if prDetail.checkSuites.length > 0}
            <div class="workflow-list">
              {#each prDetail.checkSuites as run}
                <button
                  class="workflow-run-item"
                  type="button"
                  onclick={() => openWorkflowRun(run)}
                >
                  <span class="workflow-status {workflowStatusClass(run)}"
                    >{workflowStatusIcon(run)}</span
                  >
                  <span class="workflow-name">{run.workflowName}</span>
                  <span class="workflow-status-text">
                    {workflowStatusText(run)}
                  </span>
                </button>
              {/each}
            </div>
          {:else}
            <div class="workflow-empty">No workflows</div>
          {/if}
        </div>
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
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  .branch-detail h2 {
    font-size: var(--ui-font-lg);
    font-weight: 700;
    color: var(--text-primary);
    font-family: monospace;
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

  .workflow-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .workflow-run-item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
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

  .workflow-status-text {
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

	  .workflow-panel .workflow-run-item {
	    border-radius: 4px;
	  }
	</style>
