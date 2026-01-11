import { beforeEach, describe, expect, it } from "bun:test";
import {
  clearInstalledVersionCache,
  getInstalledVersionCache,
  prefetchInstalledVersions,
  setInstalledVersionCache,
} from "../installedVersionCache.js";

describe("installedVersionCache", () => {
  beforeEach(() => {
    clearInstalledVersionCache();
  });

  it("stores and retrieves installed versions", () => {
    setInstalledVersionCache("claude-code", {
      version: "1.0.0",
      path: "/usr/local/bin/claude",
    });

    const cached = getInstalledVersionCache("claude-code");
    expect(cached).toEqual({
      version: "1.0.0",
      path: "/usr/local/bin/claude",
    });
  });

  it("prefetches installed versions with custom fetcher", async () => {
    await prefetchInstalledVersions(["claude-code", "codex-cli"], async (id) =>
      id === "claude-code" ? { version: "2.0.0", path: "/opt/claude" } : null,
    );

    expect(getInstalledVersionCache("claude-code")).toEqual({
      version: "2.0.0",
      path: "/opt/claude",
    });
    expect(getInstalledVersionCache("codex-cli")).toBeNull();
  });

  it("stores null when fetcher throws", async () => {
    await prefetchInstalledVersions(["gemini-cli"], async () => {
      throw new Error("boom");
    });

    expect(getInstalledVersionCache("gemini-cli")).toBeNull();
  });
});
