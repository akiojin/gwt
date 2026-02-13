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

export async function installTauriMock(page: Page): Promise<void> {
  await page.addInitScript(
    ({ projectPath, lastOpenedAt }: { projectPath: string; lastOpenedAt: string }) => {
      type InvokeArgs = Record<string, unknown>;
      type InvokeEntry = { cmd: string; args: InvokeArgs };

      const invokeLog: InvokeEntry[] = [];
      const callbacks = new Map<number, (...args: unknown[]) => void>();
      let callbackSeq = 1;
      let listenSeq = 1;

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

      function projectInfo(pathLike: unknown) {
        const path = typeof pathLike === "string" && pathLike.length > 0 ? pathLike : projectPath;
        const normalized = path.replace(/\/+$/, "");
        const repoName = normalized.split("/").filter(Boolean).pop() || "gwt";
        return {
          path,
          repo_name: repoName,
          current_branch: "main",
        };
      }

      function invoke(cmd: string, rawArgs?: unknown): unknown {
        const args = normalizeArgs(rawArgs);
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
              projectPath: typeof args.path === "string" ? args.path : projectPath,
            };
          case "open_project":
            return projectInfo(args.path);
          case "close_project":
            return null;
          case "list_worktree_branches":
          case "list_remote_branches":
          case "list_worktrees":
          case "list_terminals":
            return [];
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
          case "get_agent_mode_state_cmd":
            return cloneAgentModeState();
          case "send_agent_mode_message": {
            const input = typeof args.input === "string" ? args.input.trim() : "";
            if (!input) return cloneAgentModeState();
            const now = Date.now();
            const nextMessages = [
              ...agentModeState.messages,
              { role: "user" as const, kind: "message" as const, content: input, timestamp: now },
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
              estimated_tokens: agentModeState.estimated_tokens + Math.max(1, input.length),
              last_error: null,
            };
            return cloneAgentModeState();
          }
        }

        if (cmd === "plugin:event|listen") {
          return listenSeq++;
        }
        if (cmd === "plugin:event|unlisten") {
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

      (window as unknown as { __GWT_TAURI_INVOKE_LOG__?: InvokeEntry[] }).__GWT_TAURI_INVOKE_LOG__ =
        invokeLog;

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
            metadata: { currentWindow: { label: string } };
            invoke: (cmd: string, args?: unknown, options?: unknown) => Promise<unknown>;
            transformCallback: (callback: (...args: unknown[]) => void, once?: boolean) => number;
            unregisterCallback: (id: number) => void;
            convertFileSrc: (filePath: string, protocol?: string) => string;
          };
        }
      ).__TAURI_INTERNALS__ = {
        metadata: {
          currentWindow: {
            label: "main",
          },
        },
        invoke: async (cmd: string, args?: unknown, _options?: unknown) => invoke(cmd, args),
        transformCallback: (callback: (...args: unknown[]) => void, _once = false) => {
          const id = callbackSeq++;
          callbacks.set(id, callback);
          return id;
        },
        unregisterCallback: (id: number) => {
          callbacks.delete(id);
        },
        convertFileSrc: (filePath: string, protocol = "asset") =>
          `${protocol}://${String(filePath).replace(/^\/+/, "")}`,
      };
    },
    {
      projectPath: DEFAULT_PROJECT_PATH,
      lastOpenedAt: DEFAULT_LAST_OPENED_AT,
    }
  );
}
