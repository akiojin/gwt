import { describe, expect, it } from "bun:test";
import { buildBunReexecCommand } from "../../src/utils/bun-runtime.js";

describe("buildBunReexecCommand", () => {
  it("returns null when Bun is already available", () => {
    const result = buildBunReexecCommand({
      hasBunGlobal: true,
      bunExecPath: "bun",
      argv: ["node", "gwt.js"],
      scriptPath: "/path/to/gwt.js",
    });

    expect(result).toBeNull();
  });

  it("returns bun command and forwards arguments", () => {
    const result = buildBunReexecCommand({
      hasBunGlobal: false,
      bunExecPath: null,
      argv: ["node", "/path/to/gwt.js", "--version"],
      scriptPath: "/path/to/gwt.js",
    });

    expect(result).toEqual({
      command: "bun",
      args: ["/path/to/gwt.js", "--version"],
    });
  });

  it("uses custom Bun executable when provided", () => {
    const result = buildBunReexecCommand({
      hasBunGlobal: false,
      bunExecPath: "/custom/bun",
      argv: ["node", "/path/to/gwt.js"],
      scriptPath: "/path/to/gwt.js",
    });

    expect(result).toEqual({
      command: "/custom/bun",
      args: ["/path/to/gwt.js"],
    });
  });
});
