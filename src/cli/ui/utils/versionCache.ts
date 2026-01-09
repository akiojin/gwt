/**
 * Version cache for coding agents (FR-028 ~ FR-031)
 *
 * Provides caching mechanism for npm package versions to avoid
 * repeated API calls during wizard navigation.
 */

import type { VersionInfo } from "../../../utils/npmRegistry.js";

/**
 * In-memory cache for agent versions
 * Maps agentId -> VersionInfo[]
 */
const versionCache = new Map<string, VersionInfo[]>();

/**
 * Type for version fetch function (injectable for testing)
 */
export type VersionFetcher = (agentId: string) => Promise<VersionInfo[]>;

/**
 * Get cached versions for an agent
 * @returns Cached versions or null if not cached
 */
export function getVersionCache(agentId: string): VersionInfo[] | null {
  const cached = versionCache.get(agentId);
  return cached !== undefined ? cached : null;
}

/**
 * Check if versions are cached for an agent
 */
export function isVersionCachePopulated(agentId: string): boolean {
  return versionCache.has(agentId);
}

/**
 * Clear all cached versions
 */
export function clearVersionCache(): void {
  versionCache.clear();
}

/**
 * Prefetch versions for multiple agents in parallel
 * This should be called at application startup
 *
 * @param agentIds - List of agent IDs to prefetch versions for
 * @param fetchFn - Optional custom fetch function (for testing)
 */
export async function prefetchAgentVersions(
  agentIds: string[],
  fetchFn?: VersionFetcher,
): Promise<void> {
  // Import default fetcher if not provided
  const fetcher =
    fetchFn ??
    (async (agentId: string) => {
      const { fetchVersionOptionsForAgent } =
        await import("./versionFetcher.js");
      return fetchVersionOptionsForAgent(agentId);
    });

  // Fetch all versions in parallel
  const results = await Promise.allSettled(
    agentIds.map(async (agentId) => {
      try {
        const versions = await fetcher(agentId);
        versionCache.set(agentId, versions);
      } catch {
        // On error, don't set cache (getVersionCache will return null)
      }
    }),
  );

  // Log any failures for debugging (silently handled)
  for (let i = 0; i < results.length; i++) {
    const result = results[i];
    if (result && result.status === "rejected") {
      // Silently handle - cache will return null for this agent
    }
  }
}

/**
 * Set versions in cache (for testing or direct population)
 */
export function setVersionCache(
  agentId: string,
  versions: VersionInfo[],
): void {
  versionCache.set(agentId, versions);
}
