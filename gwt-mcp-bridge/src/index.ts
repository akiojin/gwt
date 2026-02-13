import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import WebSocket from "ws";
import { z } from "zod";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface McpState {
  port: number;
}

interface JsonRpcRequest {
  jsonrpc: "2.0";
  method: string;
  params: Record<string, unknown>;
  id: number;
}

interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: number;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

// ---------------------------------------------------------------------------
// WebSocket client
// ---------------------------------------------------------------------------

let ws: WebSocket | null = null;
let rpcId = 0;
let connected = false;
const pending = new Map<
  number,
  { resolve: (v: unknown) => void; reject: (e: Error) => void }
>();

function readMcpState(): McpState {
  const stateFile = join(homedir(), ".gwt", "mcp-state.json");
  const raw = readFileSync(stateFile, "utf-8");
  return JSON.parse(raw) as McpState;
}

function connectWebSocket(port: number): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(`ws://localhost:${port}`);
    let settled = false;

    const fail = (reason: Error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      reject(reason);
    };

    const timeout = setTimeout(() => {
      socket.terminate();
      fail(new Error("WebSocket connection timeout"));
    }, 5000);

    socket.on("open", () => {
      settled = true;
      clearTimeout(timeout);
      connected = true;
      resolve(socket);
    });

    socket.on("error", (err) => {
      fail(err instanceof Error ? err : new Error(String(err)));
    });

    socket.on("message", (data) => {
      if (settled && socket.readyState !== WebSocket.OPEN) return;
      try {
        const msg = JSON.parse(data.toString()) as JsonRpcResponse;
        const entry = pending.get(msg.id);
        if (entry) {
          pending.delete(msg.id);
          if (msg.error) {
            entry.reject(new Error(msg.error.message));
          } else {
            entry.resolve(msg.result);
          }
        }
      } catch {
        // ignore malformed messages
      }
    });

    socket.on("close", () => {
      if (!settled) {
        fail(new Error("WebSocket closed during handshake"));
        return;
      }

      cleanupAgentConfigs();
      if (connected) {
        process.exit(0);
      }
    });
  });
}

function sendRpc(method: string, params: Record<string, unknown>): Promise<unknown> {
  return new Promise((resolve, reject) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      reject(new Error("WebSocket not connected"));
      return;
    }

    const id = ++rpcId;
    const req: JsonRpcRequest = { jsonrpc: "2.0", method, params, id };

    const timeout = setTimeout(() => {
      pending.delete(id);
      reject(new Error("RPC timeout"));
    }, 30000);

    pending.set(id, {
      resolve: (v) => {
        clearTimeout(timeout);
        resolve(v);
      },
      reject: (e) => {
        clearTimeout(timeout);
        reject(e);
      },
    });

    ws.send(JSON.stringify(req), (err) => {
      if (!err) {
        return;
      }
      pending.delete(id);
      clearTimeout(timeout);
      const message = err instanceof Error ? err.message : String(err);
      reject(new Error(`Failed to send RPC request: ${message}`));
    });
  });
}

// ---------------------------------------------------------------------------
// Agent config cleanup (T23: self-cleanup on WS disconnect)
// ---------------------------------------------------------------------------

function cleanupAgentConfigs(): void {
  tryRemoveJsonKey(join(homedir(), ".claude.json"), ["mcpServers", "gwt-agent-bridge"]);
  tryRemoveTomlSection(join(homedir(), ".codex", "config.toml"), "gwt-agent-bridge");
  tryRemoveJsonKey(join(homedir(), ".gemini", "settings.json"), ["mcpServers", "gwt-agent-bridge"]);
}

function tryRemoveJsonKey(filePath: string, keyPath: string[]): void {
  try {
    if (!existsSync(filePath)) return;
    const raw = readFileSync(filePath, "utf-8");
    const obj = JSON.parse(raw);

    let current = obj;
    for (let i = 0; i < keyPath.length - 1; i++) {
      if (current == null || typeof current !== "object") return;
      current = current[keyPath[i]];
    }

    if (current == null || typeof current !== "object") return;
    const lastKey = keyPath[keyPath.length - 1];
    if (!(lastKey in current)) return;

    delete current[lastKey];
    writeFileSync(filePath, JSON.stringify(obj, null, 2) + "\n", "utf-8");
  } catch {
    // best-effort: ignore errors
  }
}

function tryRemoveTomlSection(filePath: string, serverName: string): void {
  try {
    if (!existsSync(filePath)) return;
    const raw = readFileSync(filePath, "utf-8");
    const lines = raw.split("\n");
    const sectionHeader = `[mcp_servers.${serverName}]`;

    let inSection = false;
    const filtered: string[] = [];

    for (const line of lines) {
      if (line.trim() === sectionHeader) {
        inSection = true;
        continue;
      }

      if (inSection) {
        if (line.trim().startsWith("[")) {
          inSection = false;
          filtered.push(line);
        }
        continue;
      }

      filtered.push(line);
    }

    writeFileSync(filePath, filtered.join("\n"), "utf-8");
  } catch {
    // best-effort: ignore errors
  }
}

// ---------------------------------------------------------------------------
// MCP Server & Tool registration
// ---------------------------------------------------------------------------

const server = new McpServer({
  name: "gwt-agent-bridge",
  version: "1.0.0",
});

async function callTool(method: string, params: Record<string, unknown>) {
  try {
    const result = await sendRpc(method, params);
    return {
      content: [{ type: "text" as const, text: JSON.stringify(result) }],
    };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return {
      content: [{ type: "text" as const, text: JSON.stringify({ error: message }) }],
      isError: true,
    };
  }
}

// gwt_list_tabs: no parameters
server.tool(
  "gwt_list_tabs",
  "List all active agent tabs in gwt",
  async () => callTool("gwt_list_tabs", {}),
);

// gwt_get_tab_info: { tab_id: string }
server.tool(
  "gwt_get_tab_info",
  "Get detailed information about a specific agent tab",
  { tab_id: z.string().describe("The ID of the tab to query") },
  async ({ tab_id }) => callTool("gwt_get_tab_info", { tab_id }),
);

// gwt_send_message: { target_tab_id: string, message: string }
server.tool(
  "gwt_send_message",
  "Send a structured message to a specific agent tab",
  {
    target_tab_id: z.string().describe("The ID of the target tab"),
    message: z.string().describe("The message text to send"),
  },
  async ({ target_tab_id, message }) =>
    callTool("gwt_send_message", { target_tab_id, message }),
);

// gwt_broadcast_message: { message: string }
server.tool(
  "gwt_broadcast_message",
  "Broadcast a message to all other agent tabs",
  { message: z.string().describe("The message text to broadcast") },
  async ({ message }) => callTool("gwt_broadcast_message", { message }),
);

// gwt_launch_agent: { agent_id: string, branch: string }
server.tool(
  "gwt_launch_agent",
  "Launch a new agent tab on a specified branch",
  {
    agent_id: z.string().describe("The agent type to launch (e.g. claude, codex, gemini)"),
    branch: z.string().describe("The git branch to work on"),
  },
  async ({ agent_id, branch }) =>
    callTool("gwt_launch_agent", { agent_id, branch }),
);

// gwt_stop_tab: { tab_id: string }
server.tool(
  "gwt_stop_tab",
  "Stop a specific agent tab",
  { tab_id: z.string().describe("The ID of the tab to stop") },
  async ({ tab_id }) => callTool("gwt_stop_tab", { tab_id }),
);

// gwt_get_worktree_diff: { tab_id: string }
server.tool(
  "gwt_get_worktree_diff",
  "Get the git diff of a tab's worktree",
  { tab_id: z.string().describe("The ID of the tab whose worktree diff to get") },
  async ({ tab_id }) => callTool("gwt_get_worktree_diff", { tab_id }),
);

// gwt_get_changed_files: { tab_id: string }
server.tool(
  "gwt_get_changed_files",
  "Get the list of changed files in a tab's worktree",
  { tab_id: z.string().describe("The ID of the tab whose changed files to list") },
  async ({ tab_id }) => callTool("gwt_get_changed_files", { tab_id }),
);

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main(): Promise<void> {
  let state: McpState;
  try {
    state = readMcpState();
  } catch {
    // gwt is not running or state file missing - exit silently
    process.exit(0);
  }

  try {
    ws = await connectWebSocket(state.port);
  } catch {
    cleanupAgentConfigs();
    process.exit(0);
  }

  const transport = new StdioServerTransport();
  await server.connect(transport);
}

main().catch(() => {
  cleanupAgentConfigs();
  process.exit(1);
});
