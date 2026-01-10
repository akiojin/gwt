/**
 * Installed version cache for coding agents (FR-017)
 *
 * Caches locally installed agent versions detected at startup
 * to avoid async fetch during wizard navigation.
 */

import type { InstalledVersionInfo } from "./versionFetcher.js";
import { fetchInstalledVersionForAgent } from "./versionFetcher.js";

/**
 * In-memory cache for installed versions
 * Maps agentId -> InstalledVersionInfo | null
 */
const installedVersionCache = new Map<string, InstalledVersionInfo | null>();

/**
 * Type for installed version fetch function (injectable for testing)
 */
export type InstalledVersionFetcher = (
  agentId: string,
) => Promise<InstalledVersionInfo | null>;

/**
 * Get cached installed version for an agent
 * @returns Cached info or null if not installed/unknown
 */
export function getInstalledVersionCache(
  agentId: string,
): InstalledVersionInfo | null {
  const cached = installedVersionCache.get(agentId);
  return cached ?? null;
}

/**
 * Set installed version cache (for testing or direct population)
 */
export function setInstalledVersionCache(
  agentId: string,
  installed: InstalledVersionInfo | null,
): void {
  installedVersionCache.set(agentId, installed);
}

/**
 * Clear installed version cache
 */
export function clearInstalledVersionCache(): void {
  installedVersionCache.clear();
}

/**
 * Prefetch installed versions for multiple agents in parallel
 * This should be called at application startup
 *
 * @param agentIds - List of agent IDs to prefetch
 * @param fetchFn - Optional custom fetch function (for testing)
 */
export async function prefetchInstalledVersions(
  agentIds: string[],
  fetchFn?: InstalledVersionFetcher,
): Promise<void> {
  const fetcher =
    fetchFn ??
    (async (agentId: string) => fetchInstalledVersionForAgent(agentId));

  const results = await Promise.allSettled(
    agentIds.map(async (agentId) => {
      try {
        const installed = await fetcher(agentId);
        setInstalledVersionCache(agentId, installed);
      } catch {
        setInstalledVersionCache(agentId, null);
      }
    }),
  );

  // Swallow any rejections to keep startup non-blocking
  for (const result of results) {
    if (result.status === "rejected") {
      // no-op
    }
  }
}
