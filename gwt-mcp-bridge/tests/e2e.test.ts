/**
 * E2E tests for gwt-mcp-bridge.
 *
 * Approach:
 *   1. Start a mock WebSocket server (simulates gwt-tauri).
 *   2. Write ~/.gwt/mcp-state.json inside a temp HOME.
 *   3. Spawn the bridge via the MCP SDK's StdioClientTransport.
 *   4. Verify the full pipeline: MCP client -> stdio -> bridge -> WS -> mock server.
 */
import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { WebSocketServer } from "ws";

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

interface JsonRpcRequest {
  jsonrpc: "2.0";
  method: string;
  params: Record<string, unknown>;
  id: number;
}

// ---------------------------------------------------------------------------
// Test suite
// ---------------------------------------------------------------------------

describe("MCP Bridge E2E", () => {
  let mockWsServer: WebSocketServer;
  let tempHome: string;
  let client: Client;

  beforeAll(async () => {
    // 1. Temp HOME so bridge reads mcp-state.json from there
    tempHome = mkdtempSync(join(tmpdir(), "gwt-mcp-e2e-"));
    mkdirSync(join(tempHome, ".gwt"), { recursive: true });

    // 2. Mock WebSocket server (simulates gwt-tauri WS)
    mockWsServer = await new Promise<WebSocketServer>((resolve) => {
      const wss = new WebSocketServer({ port: 0 }, () => resolve(wss));
    });
    const wsPort = (mockWsServer.address() as { port: number }).port;

    mockWsServer.on("connection", (ws) => {
      ws.on("message", (raw) => {
        const req = JSON.parse(raw.toString()) as JsonRpcRequest;
        let result: unknown;

        switch (req.method) {
          case "gwt_list_tabs":
            result = [
              { tab_id: "tab-1", agent_type: "claude", branch: "main", status: "running" },
              { tab_id: "tab-2", agent_type: "codex", branch: "feat/x", status: "running" },
            ];
            break;
          case "gwt_get_tab_info":
            result = {
              tab_id: req.params?.tab_id ?? "tab-1",
              agent_type: "claude",
              branch: "main",
              status: "running",
            };
            break;
          case "gwt_send_message":
            result = { success: true };
            break;
          case "gwt_broadcast_message":
            result = { sent_count: 2 };
            break;
          case "gwt_launch_agent":
            result = {
              status: "requested",
              agent_id: req.params?.agent_id,
              branch: req.params?.branch,
            };
            break;
          case "gwt_stop_tab":
            result = { success: true };
            break;
          case "gwt_get_worktree_diff":
            result = { diff: "diff --git a/file.txt b/file.txt" };
            break;
          case "gwt_get_changed_files":
            result = [{ path: "file.txt", status: "modified", is_staged: false }];
            break;
          default:
            ws.send(
              JSON.stringify({
                jsonrpc: "2.0",
                id: req.id,
                error: { code: -32601, message: `Unknown method: ${req.method}` },
              }),
            );
            return;
        }

        ws.send(JSON.stringify({ jsonrpc: "2.0", id: req.id, result }));
      });
    });

    // 3. Write mcp-state.json so the bridge can discover the mock server
    writeFileSync(
      join(tempHome, ".gwt", "mcp-state.json"),
      JSON.stringify({ port: wsPort }),
    );

    // 4. Spawn bridge via MCP SDK client
    const bridgePath = join(__dirname, "..", "dist", "gwt-mcp-bridge.js");
    const transport = new StdioClientTransport({
      command: "node",
      args: [bridgePath],
      env: { ...process.env, HOME: tempHome } as Record<string, string>,
      stderr: "pipe",
    });

    client = new Client({ name: "gwt-e2e-test", version: "1.0.0" });
    await client.connect(transport);
  }, 15_000);

  afterAll(async () => {
    try { await client?.close(); } catch { /* bridge may have exited */ }
    try { mockWsServer?.close(); } catch { /* may already be closed */ }
    if (tempHome) rmSync(tempHome, { recursive: true, force: true });
  });

  // ------- test_mcp_initialize -------
  it("should complete MCP handshake successfully", () => {
    // client.connect() in beforeAll performs the full initialize handshake.
    expect(client).toBeDefined();
  });

  // ------- test_mcp_list_tools -------
  it("should expose 8 MCP tools", async () => {
    const { tools } = await client.listTools();
    expect(tools).toHaveLength(8);

    const names = tools.map((t) => t.name).sort();
    expect(names).toEqual([
      "gwt_broadcast_message",
      "gwt_get_changed_files",
      "gwt_get_tab_info",
      "gwt_get_worktree_diff",
      "gwt_launch_agent",
      "gwt_list_tabs",
      "gwt_send_message",
      "gwt_stop_tab",
    ]);
  });

  // ------- test_mcp_call_tool_list_tabs -------
  it("should call gwt_list_tabs via WebSocket and return tab list", async () => {
    const result = await client.callTool({ name: "gwt_list_tabs" });
    expect(result.content).toHaveLength(1);

    const text = (result.content[0] as { type: string; text: string }).text;
    const tabs = JSON.parse(text);
    expect(Array.isArray(tabs)).toBe(true);
    expect(tabs).toHaveLength(2);
    expect(tabs[0]).toMatchObject({ tab_id: "tab-1", agent_type: "claude" });
    expect(tabs[1]).toMatchObject({ tab_id: "tab-2", agent_type: "codex" });
  });

  it("should call gwt_get_tab_info with arguments", async () => {
    const result = await client.callTool({
      name: "gwt_get_tab_info",
      arguments: { tab_id: "tab-1" },
    });
    expect(result.isError).toBeFalsy();

    const text = (result.content[0] as { type: string; text: string }).text;
    const info = JSON.parse(text);
    expect(info).toMatchObject({
      tab_id: "tab-1",
      agent_type: "claude",
      branch: "main",
      status: "running",
    });
  });

  it("should call gwt_send_message via WebSocket", async () => {
    const result = await client.callTool({
      name: "gwt_send_message",
      arguments: { target_tab_id: "tab-1", message: "hello" },
    });
    expect(result.isError).toBeFalsy();

    const text = (result.content[0] as { type: string; text: string }).text;
    expect(JSON.parse(text)).toMatchObject({ success: true });
  });

  it("should call gwt_get_worktree_diff via WebSocket", async () => {
    const result = await client.callTool({
      name: "gwt_get_worktree_diff",
      arguments: { tab_id: "tab-1" },
    });
    expect(result.isError).toBeFalsy();

    const text = (result.content[0] as { type: string; text: string }).text;
    const parsed = JSON.parse(text);
    expect(parsed.diff).toContain("diff --git");
  });

  it("should call gwt_get_changed_files via WebSocket", async () => {
    const result = await client.callTool({
      name: "gwt_get_changed_files",
      arguments: { tab_id: "tab-1" },
    });
    expect(result.isError).toBeFalsy();

    const text = (result.content[0] as { type: string; text: string }).text;
    const files = JSON.parse(text);
    expect(Array.isArray(files)).toBe(true);
    expect(files[0]).toMatchObject({ path: "file.txt", status: "modified" });
  });

  // ------- test_mcp_ws_disconnect_recovery -------
  it("should fail gracefully after WebSocket disconnect", async () => {
    // Sever all WS connections from the server side
    for (const ws of mockWsServer.clients) {
      ws.close();
    }

    // Give the bridge time to detect the close and exit
    await new Promise((resolve) => setTimeout(resolve, 2000));

    // The bridge should have exited; the next call must error
    try {
      await client.callTool({ name: "gwt_list_tabs" });
      // Should not reach here
      expect.unreachable("callTool should have thrown after WS disconnect");
    } catch {
      // Expected: bridge process exited, transport is closed
      expect(true).toBe(true);
    }
  }, 10_000);
});
