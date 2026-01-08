import { describe, expect, it, mock, beforeEach, afterEach } from "bun:test";

// TDD: テストを先に書き、実装は後から行う

describe("npmRegistry", () => {
  describe("fetchPackageVersions", () => {
    let originalFetch: typeof globalThis.fetch;

    beforeEach(() => {
      originalFetch = globalThis.fetch;
    });

    afterEach(() => {
      globalThis.fetch = originalFetch;
    });

    it("should return latest 10 versions including prereleases", async () => {
      // Mock npm registry response
      globalThis.fetch = mock(async () => ({
        ok: true,
        json: async () => ({
          "dist-tags": {
            latest: "1.0.10",
            next: "1.1.0-beta.1",
          },
          time: {
            "1.0.1": "2024-01-01T00:00:00.000Z",
            "1.0.2": "2024-01-02T00:00:00.000Z",
            "1.0.3": "2024-01-03T00:00:00.000Z",
            "1.0.4": "2024-01-04T00:00:00.000Z",
            "1.0.5": "2024-01-05T00:00:00.000Z",
            "1.0.6": "2024-01-06T00:00:00.000Z",
            "1.0.7": "2024-01-07T00:00:00.000Z",
            "1.0.8": "2024-01-08T00:00:00.000Z",
            "1.0.9": "2024-01-09T00:00:00.000Z",
            "1.0.10": "2024-01-10T00:00:00.000Z",
            "1.1.0-alpha.1": "2024-01-11T00:00:00.000Z",
            "1.1.0-beta.1": "2024-01-12T00:00:00.000Z",
          },
          versions: {
            "1.0.1": {},
            "1.0.2": {},
            "1.0.3": {},
            "1.0.4": {},
            "1.0.5": {},
            "1.0.6": {},
            "1.0.7": {},
            "1.0.8": {},
            "1.0.9": {},
            "1.0.10": {},
            "1.1.0-alpha.1": {},
            "1.1.0-beta.1": {},
          },
        }),
      })) as typeof globalThis.fetch;

      const { fetchPackageVersions } = await import("../npmRegistry.js");
      const versions = await fetchPackageVersions("@anthropic-ai/claude-code");

      expect(versions.length).toBe(10);
      // Should be sorted by publish date, newest first
      expect(versions[0].version).toBe("1.1.0-beta.1");
      expect(versions[0].isPrerelease).toBe(true);
      expect(versions[1].version).toBe("1.1.0-alpha.1");
      expect(versions[1].isPrerelease).toBe(true);
      expect(versions[2].version).toBe("1.0.10");
      expect(versions[2].isPrerelease).toBe(false);
    });

    it("should return empty array on timeout", async () => {
      // Mock fetch that respects AbortSignal
      globalThis.fetch = mock(async (_url: string, options?: RequestInit) => {
        const signal = options?.signal as AbortSignal | undefined;
        return new Promise((resolve, reject) => {
          const timer = setTimeout(() => {
            resolve({ ok: true, json: async () => ({}) });
          }, 5000);

          if (signal) {
            signal.addEventListener("abort", () => {
              clearTimeout(timer);
              reject(new DOMException("Aborted", "AbortError"));
            });
          }
        });
      }) as typeof globalThis.fetch;

      const { fetchPackageVersions } = await import("../npmRegistry.js");
      const versions = await fetchPackageVersions(
        "@anthropic-ai/claude-code",
        10,
        100, // 100ms timeout
      );

      expect(versions).toEqual([]);
    });

    it("should return empty array on network error", async () => {
      globalThis.fetch = mock(async () => {
        throw new Error("Network error");
      }) as typeof globalThis.fetch;

      const { fetchPackageVersions } = await import("../npmRegistry.js");
      const versions = await fetchPackageVersions("@anthropic-ai/claude-code");

      expect(versions).toEqual([]);
    });

    it("should return empty array on non-ok response", async () => {
      globalThis.fetch = mock(async () => ({
        ok: false,
        status: 404,
      })) as typeof globalThis.fetch;

      const { fetchPackageVersions } = await import("../npmRegistry.js");
      const versions = await fetchPackageVersions("nonexistent-package");

      expect(versions).toEqual([]);
    });
  });

  describe("parsePackageCommand", () => {
    it("should parse package name without version", async () => {
      const { parsePackageCommand } = await import("../npmRegistry.js");
      const result = parsePackageCommand("@anthropic-ai/claude-code");

      expect(result.packageName).toBe("@anthropic-ai/claude-code");
      expect(result.version).toBeNull();
    });

    it("should parse package name with @latest", async () => {
      const { parsePackageCommand } = await import("../npmRegistry.js");
      const result = parsePackageCommand("@anthropic-ai/claude-code@latest");

      expect(result.packageName).toBe("@anthropic-ai/claude-code");
      expect(result.version).toBe("latest");
    });

    it("should parse package name with specific version", async () => {
      const { parsePackageCommand } = await import("../npmRegistry.js");
      const result = parsePackageCommand("@anthropic-ai/claude-code@1.0.3");

      expect(result.packageName).toBe("@anthropic-ai/claude-code");
      expect(result.version).toBe("1.0.3");
    });

    it("should parse non-scoped package name", async () => {
      const { parsePackageCommand } = await import("../npmRegistry.js");
      const result = parsePackageCommand("opencode-ai@latest");

      expect(result.packageName).toBe("opencode-ai");
      expect(result.version).toBe("latest");
    });
  });

  describe("resolveVersionSuffix", () => {
    it("should return empty string for 'installed'", async () => {
      const { resolveVersionSuffix } = await import("../npmRegistry.js");
      expect(resolveVersionSuffix("installed")).toBe("");
    });

    it("should return empty string for undefined", async () => {
      const { resolveVersionSuffix } = await import("../npmRegistry.js");
      expect(resolveVersionSuffix(undefined)).toBe("");
    });

    it("should return '@latest' for 'latest'", async () => {
      const { resolveVersionSuffix } = await import("../npmRegistry.js");
      expect(resolveVersionSuffix("latest")).toBe("@latest");
    });

    it("should return '@X.Y.Z' for specific version", async () => {
      const { resolveVersionSuffix } = await import("../npmRegistry.js");
      expect(resolveVersionSuffix("1.0.3")).toBe("@1.0.3");
    });

    it("should return '@X.Y.Z-beta.1' for prerelease version", async () => {
      const { resolveVersionSuffix } = await import("../npmRegistry.js");
      expect(resolveVersionSuffix("1.1.0-beta.1")).toBe("@1.1.0-beta.1");
    });
  });
});
