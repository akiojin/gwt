import type { Page } from "@playwright/test";

const DEFAULT_PROJECT_PATH = "/tmp/gwt-playwright";
const DEFAULT_LAST_OPENED_AT = "2026-02-13T00:00:00.000Z";

type ProjectModeState = {
  messages: Array<{
    role: "user" | "assistant" | "system" | "tool";
    kind?: "message" | "thought" | "action" | "observation" | "error";
    content: string;
    timestamp: number;
  }>;
  ai_ready: boolean;
  ai_error: string | null;
  last_error: string | null;
  is_waiting: boolean;
  session_name: string;
  llm_call_count: number;
  estimated_tokens: number;
};

type TauriMockCommandResponse = unknown;

type InstallTauriMockOptions = {
  commandResponses?: Record<string, TauriMockCommandResponse>;
};

type SystemInfo = {
  cpu_usage_percent: number;
  memory_used_bytes: number;
  memory_total_bytes: number;
  gpu: null;
};

export async function installTauriMock(
  page: Page,
  options: InstallTauriMockOptions = {},
): Promise<void> {
  const commandResponses = options.commandResponses ?? {};

  await page.addInitScript(
    ({
      projectPath,
      lastOpenedAt,
      commandResponses,
    }: {
      projectPath: string;
      lastOpenedAt: string;
      commandResponses: Record<string, TauriMockCommandResponse>;
    }) => {
      type InvokeArgs = Record<string, unknown>;
      type InvokeEntry = { cmd: string; args: InvokeArgs };
      type EventListener = { event: string; handlerId: number };
      type MockPane = {
        paneId: string;
        cwd: string;
        agentName: string;
        branchName: string;
        status: "running" | "error";
        errorMessage: string | null;
        scrollback: string;
      };
      type MockLaunchFinishedPayload = {
        jobId: string;
        status: "ok" | "cancelled" | "error";
        paneId: string | null;
        error: string | null;
      };
      type MockLaunchJob = {
        running: boolean;
        finished: MockLaunchFinishedPayload;
      };

      const invokeLog: InvokeEntry[] = [];
      const callbacks = new Map<number, (...args: unknown[]) => void>();
      const eventListeners = new Map<number, EventListener>();
      const panes = new Map<string, MockPane>();
      const launchJobs = new Map<string, MockLaunchJob>();
      let callbackSeq = 1;
      let listenSeq = 1;
      let paneSeq = 1;
      let launchJobSeq = 1;
      let nextSpawnShellError = false;
      let lastSpawnedPaneId: string | null = null;
      let restoreLeaderAcquired = false;
      let mockLocalBranches = Array.isArray(commandResponses.list_worktree_branches)
        ? structuredClone(commandResponses.list_worktree_branches)
        : [];
      let mockRemoteBranches = Array.isArray(commandResponses.list_remote_branches)
        ? structuredClone(commandResponses.list_remote_branches)
        : [];
      let mockWorktrees = Array.isArray(commandResponses.list_worktrees)
        ? structuredClone(commandResponses.list_worktrees)
        : [];

      let projectModeState: ProjectModeState = {
        messages: [],
        ai_ready: true,
        ai_error: null,
        last_error: null,
        is_waiting: false,
        session_name: "Project Mode",
        llm_call_count: 0,
        estimated_tokens: 0,
      };

      function cloneProjectModeState(): ProjectModeState {
        return {
          ...projectModeState,
          messages: projectModeState.messages.map((msg) => ({ ...msg })),
        };
      }

      function normalizeArgs(value: unknown): InvokeArgs {
        if (!value || typeof value !== "object") return {};
        return value as InvokeArgs;
      }

      async function resolveMockResponse(
        response: unknown,
      ): Promise<unknown> {
        if (
          response &&
          typeof response === "object" &&
          "__error" in response
        ) {
          const message = String(
            (response as { __error?: unknown }).__error ?? "Mock command failed",
          );
          throw new Error(message);
        }
        if (
          response &&
          typeof response === "object" &&
          "__delayMs" in response &&
          "value" in response
        ) {
          const delayMs = Number(
            (response as { __delayMs?: unknown }).__delayMs ?? 0,
          );
          if (Number.isFinite(delayMs) && delayMs > 0) {
            await new Promise((resolve) => setTimeout(resolve, delayMs));
          }
          return (response as { value: unknown }).value;
        }
        return response;
      }

      function isEnterOnly(data: unknown): boolean {
        if (!Array.isArray(data)) return false;
        if (data.length === 1) return data[0] === 13 || data[0] === 10;
        if (data.length === 2) return data[0] === 13 && data[1] === 10;
        return false;
      }

      function emitEvent(event: string, payload: unknown): void {
        for (const [listenerId, listener] of [...eventListeners.entries()]) {
          if (listener.event !== event) continue;
          const callback = callbacks.get(listener.handlerId);
          if (!callback) continue;
          callback({
            event,
            id: listenerId,
            payload,
          });
        }
      }

      function listTerminals() {
        return Array.from(panes.values()).map((pane) => ({
          pane_id: pane.paneId,
          agent_name: pane.agentName,
          branch_name: pane.branchName,
          status:
            pane.status === "running"
              ? "running"
              : `error: ${pane.errorMessage ?? "PTY stream error: mock failure"}`,
        }));
      }

      function branchInventoryKey(nameLike: unknown): string {
        const name =
          typeof nameLike === "string" && nameLike.trim()
            ? nameLike.trim()
            : "";
        return name.startsWith("origin/") ? name.slice("origin/".length) : name;
      }

      function listBranchInventory() {
        const entries = new Map<
          string,
          {
            id: string;
            canonical_name: string;
            primary_branch: unknown;
            local_branch: unknown | null;
            remote_branch: unknown | null;
            has_local: boolean;
            has_remote: boolean;
            worktree_path: string | null;
            worktree_count: number;
            resolution_action: "focusExisting" | "createWorktree" | "resolveAmbiguity";
          }
        >();

        for (const branch of mockLocalBranches) {
          const key = branchInventoryKey(branch?.name);
          if (!key) continue;
          entries.set(key, {
            id: key,
            canonical_name: key,
            primary_branch: branch,
            local_branch: branch,
            remote_branch: null,
            has_local: true,
            has_remote: false,
            worktree_path: null,
            worktree_count: 0,
            resolution_action: "createWorktree",
          });
        }

        for (const branch of mockRemoteBranches) {
          const key = branchInventoryKey(branch?.name);
          if (!key) continue;
          const existing = entries.get(key);
          entries.set(key, {
            id: key,
            canonical_name: key,
            primary_branch: existing?.primary_branch ?? branch,
            local_branch: existing?.local_branch ?? null,
            remote_branch: branch,
            has_local: existing?.has_local ?? false,
            has_remote: true,
            worktree_path: existing?.worktree_path ?? null,
            worktree_count: existing?.worktree_count ?? 0,
            resolution_action: existing?.resolution_action ?? "createWorktree",
          });
        }

        for (const worktree of mockWorktrees) {
          const key = branchInventoryKey(worktree?.branch);
          if (!key) continue;
          const existing = entries.get(key);
          const worktreeCount = (existing?.worktree_count ?? 0) + 1;
          entries.set(key, {
            id: key,
            canonical_name: key,
            primary_branch:
              existing?.primary_branch ??
              mockLocalBranches.find((branch) => branchInventoryKey(branch?.name) === key) ??
              mockRemoteBranches.find((branch) => branchInventoryKey(branch?.name) === key) ??
              {
                name: key,
                commit: worktree?.commit ?? "mock-created",
                is_current: false,
                is_agent_running: false,
                agent_status: "unknown",
                ahead: 0,
                behind: 0,
                divergence_status: "UpToDate",
                commit_timestamp: null,
                last_tool_usage: null,
              },
            local_branch:
              existing?.local_branch ??
              mockLocalBranches.find((branch) => branchInventoryKey(branch?.name) === key) ??
              null,
            remote_branch:
              existing?.remote_branch ??
              mockRemoteBranches.find((branch) => branchInventoryKey(branch?.name) === key) ??
              null,
            has_local:
              existing?.has_local ??
              mockLocalBranches.some((branch) => branchInventoryKey(branch?.name) === key),
            has_remote:
              existing?.has_remote ??
              mockRemoteBranches.some((branch) => branchInventoryKey(branch?.name) === key),
            worktree_path:
              worktreeCount === 1 && typeof worktree?.path === "string"
                ? worktree.path
                : null,
            worktree_count: worktreeCount,
            resolution_action:
              worktreeCount > 1 ? "resolveAmbiguity" : "focusExisting",
          });
        }

        return Array.from(entries.values());
      }

      function getBranchInventoryDetail(canonicalNameLike: unknown) {
        const canonicalName =
          typeof canonicalNameLike === "string" ? canonicalNameLike.trim() : "";
        if (!canonicalName) return null;
        return (
          listBranchInventory().find(
            (entry) => entry.canonical_name === canonicalName,
          ) ?? null
        );
      }

      function spawnShell(workingDirLike: unknown): string {
        const paneId = `mock-pane-${paneSeq++}`;
        const cwd =
          typeof workingDirLike === "string" && workingDirLike.trim()
            ? workingDirLike
            : projectPath;
        const errorMessage = nextSpawnShellError
          ? "PTY stream error: mocked read failure"
          : null;
        nextSpawnShellError = false;
        lastSpawnedPaneId = paneId;

        const scrollback = errorMessage
          ? `\r\n[${errorMessage}]\r\n\r\nPress Enter to close this tab.\r\n`
          : "";

        panes.set(paneId, {
          paneId,
          cwd,
          agentName: "terminal",
          branchName: "",
          status: errorMessage ? "error" : "running",
          errorMessage,
          scrollback,
        });

        return paneId;
      }

      function startLaunchJob(requestLike: unknown): string {
        const request =
          requestLike && typeof requestLike === "object"
            ? (requestLike as InvokeArgs)
            : {};
        const branchName =
          typeof request.branch === "string" ? request.branch.trim() : "";
        const agentName =
          typeof request.agentId === "string" && request.agentId.trim()
            ? request.agentId.trim()
            : "codex";

        const paneId = `mock-agent-pane-${paneSeq++}`;
        panes.set(paneId, {
          paneId,
          cwd: projectPath,
          agentName,
          branchName,
          status: "running",
          errorMessage: null,
          scrollback: "",
        });

        const jobId = `mock-launch-job-${launchJobSeq++}`;
        launchJobs.set(jobId, {
          running: true,
          finished: {
            jobId,
            status: "ok",
            paneId,
            error: null,
          },
        });

        setTimeout(() => {
          emitEvent("launch-progress", {
            jobId,
            step: "spawn",
            detail: `Launching ${agentName}`,
          });
        }, 0);

        setTimeout(() => {
          const job = launchJobs.get(jobId);
          if (!job || !job.running) return;
          job.running = false;
          launchJobs.set(jobId, job);
          emitEvent("launch-finished", job.finished);
        }, 10);

        return jobId;
      }

      function projectInfo(pathLike: unknown) {
        const path =
          typeof pathLike === "string" && pathLike.length > 0
            ? pathLike
            : projectPath;
        const normalized = path.replace(/\/+$/, "");
        const repoName = normalized.split("/").filter(Boolean).pop() || "gwt";
        return {
          path,
          repo_name: repoName,
          current_branch: "main",
        };
      }

      function openProjectResult(pathLike: unknown) {
        return {
          info: projectInfo(pathLike),
          action: "opened",
          focusedWindowLabel: null,
        };
      }

      async function invoke(cmd: string, rawArgs?: unknown): Promise<unknown> {
        const args = normalizeArgs(rawArgs);
        invokeLog.push({ cmd, args });
        const runtimeCommandResponses = (
          window as unknown as {
            __GWT_MOCK_COMMAND_RESPONSES__?: Record<string, unknown>;
          }
        ).__GWT_MOCK_COMMAND_RESPONSES__;
        if (runtimeCommandResponses) {
          if (Array.isArray(runtimeCommandResponses.list_worktree_branches)) {
            mockLocalBranches = structuredClone(
              runtimeCommandResponses.list_worktree_branches,
            );
          }
          if (Array.isArray(runtimeCommandResponses.list_remote_branches)) {
            mockRemoteBranches = structuredClone(
              runtimeCommandResponses.list_remote_branches,
            );
          }
          if (Array.isArray(runtimeCommandResponses.list_worktrees)) {
            mockWorktrees = structuredClone(runtimeCommandResponses.list_worktrees);
          }
        }
        if (cmd === "list_worktree_branches") {
          return resolveMockResponse(
            runtimeCommandResponses?.list_worktree_branches ?? commandResponses.list_worktree_branches ?? mockLocalBranches,
          );
        }
        if (cmd === "list_remote_branches") {
          return resolveMockResponse(
            runtimeCommandResponses?.list_remote_branches ?? commandResponses.list_remote_branches ?? mockRemoteBranches,
          );
        }
        if (cmd === "list_worktrees") {
          return resolveMockResponse(
            runtimeCommandResponses?.list_worktrees ?? commandResponses.list_worktrees ?? mockWorktrees,
          );
        }
        if (
          runtimeCommandResponses &&
          Object.prototype.hasOwnProperty.call(runtimeCommandResponses, cmd)
        ) {
          return resolveMockResponse(runtimeCommandResponses[cmd]);
        }
        if (Object.prototype.hasOwnProperty.call(commandResponses, cmd)) {
          return resolveMockResponse(commandResponses[cmd]);
        }

        switch (cmd) {
          case "detect_agents":
            // Keep StatusBar reactive graph stable in web-preview E2E.
            return [];
          case "get_voice_capability":
            return {
              available: false,
              reason: "GPU acceleration is not available",
            };
          case "get_settings":
            return {
              ui_font_size: 13,
              terminal_font_size: 13,
              ui_font_family:
                'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
              terminal_font_family:
                '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace',
              app_language: "auto",
              agent_skill_registration_enabled: true,
              voice_input: {
                enabled: false,
                engine: "qwen3-asr",
                language: "auto",
                quality: "balanced",
                model: "Qwen/Qwen3-ASR-1.7B",
              },
            };
          case "rebuild_all_branch_session_summaries":
            return null;
          case "get_system_info":
            return {
              cpu_usage_percent: 0,
              memory_used_bytes: 0,
              memory_total_bytes: 0,
              gpu: null,
            } satisfies SystemInfo;
          case "fetch_ci_log":
            // App opens CI logs inside a terminal tab.
            return "mock ci log\n";
          case "is_os_env_ready":
            return true;
          case "check_app_update":
            return {
              state: "up_to_date",
              checked_at: "2026-02-13T00:00:00.000Z",
            };
          case "get_recent_projects":
            return [
              {
                path: projectPath,
                lastOpened: lastOpenedAt,
              },
            ];
          case "get_current_window_label":
            return "main";
          case "try_acquire_window_restore_leader": {
            const label =
              typeof args.label === "string" ? args.label.trim() : "";
            if (label !== "main" || restoreLeaderAcquired) return false;
            restoreLeaderAcquired = true;
            return true;
          }
          case "release_window_restore_leader": {
            const label =
              typeof args.label === "string" ? args.label.trim() : "";
            if (label === "main") {
              restoreLeaderAcquired = false;
            }
            return null;
          }
          case "open_gwt_window": {
            const label =
              typeof args.label === "string" ? args.label.trim() : "";
            return label || "main";
          }
          case "probe_path":
            return {
              kind: "gwtProject",
              projectPath:
                typeof args.path === "string" ? args.path : projectPath,
            };
          case "open_project":
            return openProjectResult(args.path);
          case "close_project":
            return null;
          case "list_worktree_branches":
            return mockLocalBranches;
          case "list_branch_inventory":
            return listBranchInventory();
          case "get_branch_inventory_detail":
            return getBranchInventoryDetail(args.canonicalName);
          case "list_remote_branches":
            return mockRemoteBranches;
          case "list_worktrees":
            return mockWorktrees;
          case "list_terminals":
            return listTerminals();
          case "get_current_branch":
            return {
              name: "main",
              commit: "0000000",
              is_current: true,
              is_agent_running: false,
              ahead: 0,
              behind: 0,
              divergence_status: "UpToDate",
              last_tool_usage: null,
            };
          case "materialize_worktree_ref": {
            const branchRef =
              typeof args.branchRef === "string" ? args.branchRef.trim() : "";
            const normalizedBranch = branchRef.replace(/^origin\//, "");
            const existing = mockWorktrees.find(
              (worktree) => worktree?.branch === normalizedBranch,
            );
            if (existing) {
              return { worktree: existing, created: false };
            }

            const created = {
              path: `${projectPath}/.gwt/worktrees/${normalizedBranch.replace(/[^a-zA-Z0-9_-]+/g, "-")}`,
              branch: normalizedBranch,
              commit: "mock-created",
              status: "active",
              is_main: false,
              has_changes: false,
              has_unpushed: false,
              is_current: false,
              is_protected: false,
              is_agent_running: false,
              agent_status: "unknown",
              ahead: 0,
              behind: 0,
              is_gone: false,
              last_tool_usage: null,
              safety_level: "safe",
            };

            mockWorktrees = [...mockWorktrees, created];
            if (
              !mockLocalBranches.some(
                (branch) => branch?.name?.trim?.() === normalizedBranch,
              )
            ) {
              mockLocalBranches = [
                ...mockLocalBranches,
                {
                  name: normalizedBranch,
                  commit: "mock-created",
                  is_current: false,
                  is_agent_running: false,
                  ahead: 0,
                  behind: 0,
                  divergence_status: "UpToDate",
                  last_tool_usage: null,
                },
              ];
            }
            if (runtimeCommandResponses) {
              runtimeCommandResponses.list_worktrees = structuredClone(mockWorktrees);
              runtimeCommandResponses.list_worktree_branches = structuredClone(
                mockLocalBranches,
              );
            }

            return { worktree: created, created: true };
          }
          case "sync_window_agent_tabs":
            return null;
          case "start_launch_job":
            return startLaunchJob(args.request);
          case "poll_launch_job": {
            const jobId = typeof args.jobId === "string" ? args.jobId : "";
            const job = launchJobs.get(jobId);
            if (!job) {
              return { running: false, finished: null };
            }
            return {
              running: job.running,
              finished: job.running ? null : job.finished,
            };
          }
          case "cancel_launch_job": {
            const jobId = typeof args.jobId === "string" ? args.jobId : "";
            const job = launchJobs.get(jobId);
            if (!job) return null;
            if (job.running) {
              job.running = false;
              job.finished = {
                jobId,
                status: "cancelled",
                paneId: null,
                error: null,
              };
              launchJobs.set(jobId, job);
              emitEvent("launch-finished", job.finished);
            }
            return null;
          }
          case "spawn_shell":
            return spawnShell(args.workingDir);
          case "terminal_ready": {
            const paneId = typeof args.paneId === "string" ? args.paneId : "";
            const scrollback = panes.get(paneId)?.scrollback ?? "";
            return Array.from(new TextEncoder().encode(scrollback));
          }
          case "capture_scrollback_tail": {
            const paneId = typeof args.paneId === "string" ? args.paneId : "";
            return panes.get(paneId)?.scrollback ?? "";
          }
          case "resize_terminal":
            return null;
          case "close_terminal": {
            const paneId = typeof args.paneId === "string" ? args.paneId : "";
            if (!paneId) return null;
            const existed = panes.delete(paneId);
            if (existed) {
              emitEvent("terminal-closed", { pane_id: paneId });
            }
            return null;
          }
          case "write_terminal": {
            const paneId = typeof args.paneId === "string" ? args.paneId : "";
            const pane = panes.get(paneId);
            if (!pane) return null;

            if (isEnterOnly(args.data) && pane.status !== "running") {
              panes.delete(paneId);
              emitEvent("terminal-closed", { pane_id: paneId });
              return null;
            }

            return null;
          }
          case "get_project_mode_state_cmd":
            return cloneProjectModeState();
          case "send_project_mode_message_cmd": {
            const input =
              typeof args.input === "string" ? args.input.trim() : "";
            if (!input) return cloneProjectModeState();
            const now = Date.now();
            const nextMessages = [
              ...projectModeState.messages,
              {
                role: "user" as const,
                kind: "message" as const,
                content: input,
                timestamp: now,
              },
              {
                role: "assistant" as const,
                kind: "message" as const,
                content: `Echo: ${input}`,
                timestamp: now + 1,
              },
            ];
            projectModeState = {
              ...projectModeState,
              messages: nextMessages,
              llm_call_count: projectModeState.llm_call_count + 1,
              estimated_tokens:
                projectModeState.estimated_tokens + Math.max(1, input.length),
              last_error: null,
            };
            return cloneProjectModeState();
          }
        }

        if (cmd === "plugin:event|listen") {
          const id = listenSeq++;
          const eventName = typeof args.event === "string" ? args.event : "";
          const handlerId =
            typeof args.handler === "number" ? args.handler : null;
          if (eventName && handlerId !== null) {
            eventListeners.set(id, { event: eventName, handlerId });
          }
          return id;
        }
        if (cmd === "plugin:event|unlisten") {
          const listenerId =
            typeof args.eventId === "number"
              ? args.eventId
              : typeof args.id === "number"
                ? args.id
                : null;
          if (listenerId !== null) {
            eventListeners.delete(listenerId);
          }
          return null;
        }
        if (cmd === "plugin:app|version") {
          return "7.1.1";
        }
        if (cmd.startsWith("plugin:window|")) {
          if (cmd === "plugin:window|get_all_windows") return ["main"];
          if (cmd === "plugin:window|is_focused") return true;
          return null;
        }
        if (cmd.startsWith("plugin:dialog|")) {
          if (cmd === "plugin:dialog|confirm") return false;
          return null;
        }

        return null;
      }

      (
        window as unknown as { __GWT_TAURI_INVOKE_LOG__?: InvokeEntry[] }
      ).__GWT_TAURI_INVOKE_LOG__ = invokeLog;
      (
        window as unknown as {
          __GWT_MOCK_SET_NEXT_SPAWN_ERROR__?: (enabled: boolean) => void;
        }
      ).__GWT_MOCK_SET_NEXT_SPAWN_ERROR__ = (enabled: boolean) => {
        nextSpawnShellError = !!enabled;
      };
      (
        window as unknown as {
          __GWT_MOCK_EMIT_EVENT__?: (event: string, payload: unknown) => void;
        }
      ).__GWT_MOCK_EMIT_EVENT__ = (event: string, payload: unknown) => {
        emitEvent(event, payload);
      };
      (
        window as unknown as {
          __GWT_MOCK_LAST_SPAWNED_PANE_ID__?: () => string | null;
        }
      ).__GWT_MOCK_LAST_SPAWNED_PANE_ID__ = () => lastSpawnedPaneId;

      (
        window as unknown as {
          __TAURI_EVENT_PLUGIN_INTERNALS__?: { unregisterListener: () => void };
        }
      ).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: () => {},
      };

      (
        window as unknown as {
          __TAURI_INTERNALS__?: {
            metadata: {
              currentWindow: { label: string };
              currentWebview: { label: string };
            };
            invoke: (
              cmd: string,
              args?: unknown,
              options?: unknown,
            ) => Promise<unknown>;
            transformCallback: (
              callback: (...args: unknown[]) => void,
              once?: boolean,
            ) => number;
            unregisterCallback: (id: number) => void;
            convertFileSrc: (filePath: string, protocol?: string) => string;
          };
        }
      ).__TAURI_INTERNALS__ = {
        metadata: {
          currentWindow: {
            label: "main",
          },
          currentWebview: {
            label: "main",
          },
        },
        invoke: async (cmd: string, args?: unknown, _options?: unknown) =>
          invoke(cmd, args),
        transformCallback: (
          callback: (...args: unknown[]) => void,
          _once = false,
        ) => {
          const id = callbackSeq++;
          callbacks.set(id, callback);
          return id;
        },
        unregisterCallback: (id: number) => {
          callbacks.delete(id);
          for (const [listenerId, listener] of eventListeners.entries()) {
            if (listener.handlerId === id) {
              eventListeners.delete(listenerId);
            }
          }
        },
        convertFileSrc: (filePath: string, protocol = "asset") =>
          `${protocol}://${String(filePath).replace(/^\/+/, "")}`,
      };
    },
    {
      projectPath: DEFAULT_PROJECT_PATH,
      lastOpenedAt: DEFAULT_LAST_OPENED_AT,
      commandResponses,
    },
  );
}
