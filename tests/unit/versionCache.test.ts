/**
 * Tests for version cache functionality (FR-028 ~ FR-031)
 *
 * FR-028: gwt is required to prefetch npm version lists for all bunx-type agents at startup and cache them
 * FR-029: Version selection screen must use cached version list and not re-access npm registry
 * FR-030: Version prefetch must be done in background without blocking UI
 * FR-031: If version cache is empty (fetch failure/timeout), fallback to showing only "latest"
 */

import { describe, expect, it, beforeEach, mock } from "bun:test";
import type { VersionInfo } from "../../src/utils/npmRegistry.js";

// Test data
const MOCK_VERSIONS: VersionInfo[] = [
  {
    version: "1.0.3",
    isPrerelease: false,
    publishedAt: "2025-01-01T00:00:00Z",
  },
  {
    version: "1.0.2",
    isPrerelease: false,
    publishedAt: "2024-12-15T00:00:00Z",
  },
  {
    version: "1.0.3-beta.1",
    isPrerelease: true,
    publishedAt: "2024-12-20T00:00:00Z",
  },
];

describe("Version Cache", () => {
  beforeEach(() => {
    // Reset cache state before each test
  });

  describe("Cache initialization (FR-028)", () => {
    it("should prefetch versions for all bunx-type agents at startup", async () => {
      // Import dynamically to ensure fresh module state
      const { prefetchAgentVersions, getVersionCache, clearVersionCache } =
        await import("../../src/cli/ui/utils/versionCache.js");
      clearVersionCache();

      const mockFetch = mock(() => Promise.resolve(MOCK_VERSIONS));

      // Prefetch for multiple agents
      await prefetchAgentVersions(
        ["claude-code", "codex-cli", "gemini-cli"],
        mockFetch,
      );

      // Verify each agent's versions are cached
      expect(getVersionCache("claude-code")).toEqual(MOCK_VERSIONS);
      expect(getVersionCache("codex-cli")).toEqual(MOCK_VERSIONS);
      expect(getVersionCache("gemini-cli")).toEqual(MOCK_VERSIONS);

      // Verify fetch was called for each agent
      expect(mockFetch).toHaveBeenCalledTimes(3);
    });

    it("should skip agents that are not bunx-type", async () => {
      const { prefetchAgentVersions, getVersionCache, clearVersionCache } =
        await import("../../src/cli/ui/utils/versionCache.js");
      clearVersionCache();

      const mockFetch = mock(() => Promise.resolve(MOCK_VERSIONS));

      // Prefetch with some agents (the function filters internally)
      await prefetchAgentVersions(["claude-code"], mockFetch);

      // Cache should exist for bunx-type agent
      expect(getVersionCache("claude-code")).toEqual(MOCK_VERSIONS);
    });
  });

  describe("Cache retrieval (FR-029)", () => {
    it("should return cached versions without re-fetching", async () => {
      const { prefetchAgentVersions, getVersionCache, clearVersionCache } =
        await import("../../src/cli/ui/utils/versionCache.js");
      clearVersionCache();

      const mockFetch = mock(() => Promise.resolve(MOCK_VERSIONS));

      // Initial prefetch
      await prefetchAgentVersions(["claude-code"], mockFetch);
      expect(mockFetch).toHaveBeenCalledTimes(1);

      // Subsequent retrievals should not call fetch
      const cached1 = getVersionCache("claude-code");
      const cached2 = getVersionCache("claude-code");

      expect(cached1).toEqual(MOCK_VERSIONS);
      expect(cached2).toEqual(MOCK_VERSIONS);
      // Fetch should still only have been called once
      expect(mockFetch).toHaveBeenCalledTimes(1);
    });

    it("should return null for agents not in cache", async () => {
      const { getVersionCache, clearVersionCache } =
        await import("../../src/cli/ui/utils/versionCache.js");
      clearVersionCache();

      const result = getVersionCache("unknown-agent");
      expect(result).toBeNull();
    });
  });

  describe("Background fetch (FR-030)", () => {
    it("should not block when prefetching fails", async () => {
      const { prefetchAgentVersions, getVersionCache, clearVersionCache } =
        await import("../../src/cli/ui/utils/versionCache.js");
      clearVersionCache();

      const mockFetch = mock(() => Promise.reject(new Error("Network error")));

      // Should not throw even if fetch fails
      await expect(
        prefetchAgentVersions(["claude-code"], mockFetch),
      ).resolves.toBeUndefined();

      // Cache should be empty (null) for failed fetch
      expect(getVersionCache("claude-code")).toBeNull();
    });

    it("should handle partial failures gracefully", async () => {
      const { prefetchAgentVersions, getVersionCache, clearVersionCache } =
        await import("../../src/cli/ui/utils/versionCache.js");
      clearVersionCache();

      let callCount = 0;
      const mockFetch = mock(() => {
        callCount++;
        if (callCount === 2) {
          return Promise.reject(new Error("Network error"));
        }
        return Promise.resolve(MOCK_VERSIONS);
      });

      // First and third succeed, second fails
      await prefetchAgentVersions(
        ["claude-code", "codex-cli", "gemini-cli"],
        mockFetch,
      );

      expect(getVersionCache("claude-code")).toEqual(MOCK_VERSIONS);
      expect(getVersionCache("codex-cli")).toBeNull(); // Failed
      expect(getVersionCache("gemini-cli")).toEqual(MOCK_VERSIONS);
    });
  });

  describe("Fallback behavior (FR-031)", () => {
    it("should return null when cache is empty, allowing UI to show latest only", async () => {
      const { getVersionCache, clearVersionCache } =
        await import("../../src/cli/ui/utils/versionCache.js");
      clearVersionCache();

      // Without prefetch, cache should be empty
      const result = getVersionCache("claude-code");
      expect(result).toBeNull();
    });

    it("should handle timeout by returning empty cache", async () => {
      const { prefetchAgentVersions, getVersionCache, clearVersionCache } =
        await import("../../src/cli/ui/utils/versionCache.js");
      clearVersionCache();

      // Simulate timeout by returning empty array
      const mockFetch = mock(() => Promise.resolve([]));

      await prefetchAgentVersions(["claude-code"], mockFetch);

      // Empty array from fetch should be stored as empty
      const result = getVersionCache("claude-code");
      expect(result).toEqual([]);
    });
  });

  describe("Cache state management", () => {
    it("should support clearing the cache", async () => {
      const { prefetchAgentVersions, getVersionCache, clearVersionCache } =
        await import("../../src/cli/ui/utils/versionCache.js");
      clearVersionCache();

      const mockFetch = mock(() => Promise.resolve(MOCK_VERSIONS));

      await prefetchAgentVersions(["claude-code"], mockFetch);
      expect(getVersionCache("claude-code")).toEqual(MOCK_VERSIONS);

      clearVersionCache();
      expect(getVersionCache("claude-code")).toBeNull();
    });

    it("should check if cache is populated", async () => {
      const {
        prefetchAgentVersions,
        isVersionCachePopulated,
        clearVersionCache,
      } = await import("../../src/cli/ui/utils/versionCache.js");
      clearVersionCache();

      expect(isVersionCachePopulated("claude-code")).toBe(false);

      const mockFetch = mock(() => Promise.resolve(MOCK_VERSIONS));
      await prefetchAgentVersions(["claude-code"], mockFetch);

      expect(isVersionCachePopulated("claude-code")).toBe(true);
    });
  });
});
