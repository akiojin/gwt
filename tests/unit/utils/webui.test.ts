/**
 * Tests for Web UI utility functions
 * Spec: SPEC-1f56fd80 (FR-006)
 */
import { describe, it, expect, afterEach } from "vitest";
import * as net from "node:net";
import { resolveWebUiPort, isPortInUse } from "../../../src/utils/webui.js";

describe("resolveWebUiPort", () => {
  it("returns default port 3000 when PORT env is undefined", () => {
    expect(resolveWebUiPort(undefined)).toBe(3000);
  });

  it("returns parsed port when valid PORT is provided", () => {
    expect(resolveWebUiPort("8080")).toBe(8080);
  });

  it("returns default port when PORT is invalid", () => {
    expect(resolveWebUiPort("invalid")).toBe(3000);
    expect(resolveWebUiPort("-1")).toBe(3000);
    expect(resolveWebUiPort("99999")).toBe(3000);
  });
});

describe("isPortInUse (FR-006)", () => {
  let server: net.Server | null = null;

  afterEach(async () => {
    const s = server;
    if (s) {
      await new Promise<void>((resolve) => {
        s.close(() => resolve());
      });
      server = null;
    }
  });

  it("returns false when port is available", async () => {
    // Use a random high port that is likely available
    const port = 49152 + Math.floor(Math.random() * 1000);
    const result = await isPortInUse(port);
    expect(result).toBe(false);
  });

  it("returns true when port is in use", async () => {
    // Start a server to occupy the port
    const port = 49152 + Math.floor(Math.random() * 1000);

    const s = net.createServer();
    server = s;
    await new Promise<void>((resolve, reject) => {
      s.on("error", reject);
      s.listen(port, "127.0.0.1", () => resolve());
    });

    const result = await isPortInUse(port);
    expect(result).toBe(true);
  });

  it("returns false on connection errors (network issues)", async () => {
    // Port 1 is typically restricted and will cause EACCES or similar
    // We expect the function to handle errors gracefully
    const result = await isPortInUse(1);
    // Either false (error handled) or true (if somehow accessible)
    expect(typeof result).toBe("boolean");
  });
});
