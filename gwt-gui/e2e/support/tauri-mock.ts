import type { Page } from "@playwright/test";

const DEFAULT_PROJECT_PATH = "/tmp/gwt-playwright";
const DEFAULT_LAST_OPENED_AT = "2026-02-13T00:00:00.000Z";

type AgentModeState = {
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
        status: "running" | "error";
        errorMessage: string | null;
        scrollback: string;
      };

      const invokeLog: InvokeEntry[] = [];
      const callbacks = new Map<number, (...args: unknown[]) => void>();
      const eventListeners = new Map<number, EventListener>();
      const panes = new Map<string, MockPane>();
      let callbackSeq = 1;
      let listenSeq = 1;
      let paneSeq = 1;
      let nextSpawnShellError = false;
      let lastSpawnedPaneId: string | null = null;

      let agentModeState: AgentModeState = {
        messages: [],
        ai_ready: true,
        ai_error: null,
        last_error: null,
        is_waiting: false,
        session_name: "Agent Mode",
        llm_call_count: 0,
        estimated_tokens: 0,
      };

      function cloneAgentModeState(): AgentModeState {
        return {
          ...agentModeState,
          messages: agentModeState.messages.map((msg) => ({ ...msg })),
        };
      }

      function normalizeArgs(value: unknown): InvokeArgs {
        if (!value || typeof value !== "object") return {};
        return value as InvokeArgs;
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
          agent_name: "terminal",
          branch_name: "",
          status:
            pane.status === "running"
              ? "running"
              : `error: ${pane.errorMessage ?? "PTY stream error: mock failure"}`,
        }));
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
          status: errorMessage ? "error" : "running",
          errorMessage,
          scrollback,
        });

        return paneId;
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

      async function invoke(cmd: string, rawArgs?: unknown): Promise<unknown> {
        const args = normalizeArgs(rawArgs);
        const runtimeCommandResponses = (
          window as unknown as {
            __GWT_MOCK_COMMAND_RESPONSES__?: Record<string, unknown>;
          }
        ).__GWT_MOCK_COMMAND_RESPONSES__;
        if (
          runtimeCommandResponses &&
          Object.prototype.hasOwnProperty.call(runtimeCommandResponses, cmd)
        ) {
          return runtimeCommandResponses[cmd];
        }
        if (Object.prototype.hasOwnProperty.call(commandResponses, cmd)) {
          return commandResponses[cmd];
        }
        invokeLog.push({ cmd, args });

        switch (cmd) {
          case "get_settings":
            return {
              ui_font_size: 13,
              terminal_font_size: 13,
              voice_input: {
                enabled: false,
                hotkey: "Mod+Shift+M",
                language: "auto",
                model: "base",
              },
            };
          case "is_os_env_ready":
            return true;
          case "check_app_update":
            return {
              state: "up_to_date",
              checked_at: "2026-02-13T00:00:00.000Z",
            };
          case "check_and_update_hooks":
            return {
              registered: true,
              updated: false,
              temporary_execution: false,
            };
          case "get_recent_projects":
            return [
              {
                path: projectPath,
                lastOpened: lastOpenedAt,
              },
            ];
          case "probe_path":
            return {
              kind: "gwtProject",
              projectPath:
                typeof args.path === "string" ? args.path : projectPath,
            };
          case "open_project":
            return projectInfo(args.path);
          case "close_project":
            return null;
          case "list_worktree_branches":
          case "list_remote_branches":
          case "list_worktrees":
            return [];
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
          case "sync_window_agent_tabs":
            return null;
          case "register_hooks":
            return null;
          case "spawn_shell":
            return spawnShell(args.workingDir);
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
          case "get_agent_mode_state_cmd":
            return cloneAgentModeState();
          case "send_agent_mode_message": {
            const input =
              typeof args.input === "string" ? args.input.trim() : "";
            if (!input) return cloneAgentModeState();
            const now = Date.now();
            const nextMessages = [
              ...agentModeState.messages,
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
            agentModeState = {
              ...agentModeState,
              messages: nextMessages,
              llm_call_count: agentModeState.llm_call_count + 1,
              estimated_tokens:
                agentModeState.estimated_tokens + Math.max(1, input.length),
              last_error: null,
            };
            return cloneAgentModeState();
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
    },
  );
}
