/**
 * npm Registry API utilities for fetching package versions
 */

export interface VersionInfo {
  version: string;
  isPrerelease: boolean;
  publishedAt?: string;
}

export type VersionSelection = "installed" | "latest" | string;

interface NpmRegistryResponse {
  "dist-tags"?: Record<string, string>;
  time?: Record<string, string>;
  versions?: Record<string, unknown>;
}

const DEFAULT_TIMEOUT_MS = 3000;
const DEFAULT_LIMIT = 10;

/**
 * Check if a version string is a prerelease version
 */
function isPrerelease(version: string): boolean {
  // Prerelease versions contain - followed by alpha, beta, rc, canary, next, etc.
  return /-(alpha|beta|rc|canary|next|dev|pre)\b/i.test(version);
}

/**
 * Fetch package versions from npm registry
 * Returns the latest N versions sorted by publish date (newest first)
 */
export async function fetchPackageVersions(
  packageName: string,
  limit: number = DEFAULT_LIMIT,
  timeoutMs: number = DEFAULT_TIMEOUT_MS,
): Promise<VersionInfo[]> {
  try {
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), timeoutMs);

    const response = await fetch(
      `https://registry.npmjs.org/${encodeURIComponent(packageName)}`,
      {
        signal: controller.signal,
        headers: {
          Accept: "application/json",
        },
      },
    );

    clearTimeout(timeoutId);

    if (!response.ok) {
      return [];
    }

    const data = (await response.json()) as NpmRegistryResponse;

    if (!data.versions || !data.time) {
      return [];
    }

    // Get all versions with their publish times
    const versionsWithTime: Array<{
      version: string;
      publishedAt: string;
    }> = [];

    for (const version of Object.keys(data.versions)) {
      const publishedAt = data.time[version];
      if (publishedAt) {
        versionsWithTime.push({ version, publishedAt });
      }
    }

    // Sort by publish date (newest first)
    versionsWithTime.sort((a, b) => {
      const dateA = new Date(a.publishedAt).getTime();
      const dateB = new Date(b.publishedAt).getTime();
      return dateB - dateA;
    });

    // Take the top N versions
    const topVersions = versionsWithTime.slice(0, limit);

    // Convert to VersionInfo
    return topVersions.map((v) => ({
      version: v.version,
      isPrerelease: isPrerelease(v.version),
      publishedAt: v.publishedAt,
    }));
  } catch {
    // Return empty array on any error (network, timeout, parse error, etc.)
    return [];
  }
}

/**
 * Parse a package command to extract package name and version
 * Examples:
 *   "@anthropic-ai/claude-code" -> { packageName: "@anthropic-ai/claude-code", version: null }
 *   "@anthropic-ai/claude-code@latest" -> { packageName: "@anthropic-ai/claude-code", version: "latest" }
 *   "@anthropic-ai/claude-code@1.0.3" -> { packageName: "@anthropic-ai/claude-code", version: "1.0.3" }
 *   "opencode-ai@latest" -> { packageName: "opencode-ai", version: "latest" }
 */
export function parsePackageCommand(command: string): {
  packageName: string;
  version: string | null;
} {
  // Handle scoped packages (@scope/name@version)
  if (command.startsWith("@")) {
    // Find the second @ which separates version
    const firstSlash = command.indexOf("/");
    if (firstSlash === -1) {
      // Invalid scoped package format
      return { packageName: command, version: null };
    }
    const afterScope = command.substring(firstSlash + 1);
    const atIndex = afterScope.indexOf("@");
    if (atIndex === -1) {
      return { packageName: command, version: null };
    }
    const packageName = command.substring(0, firstSlash + 1 + atIndex);
    const version = afterScope.substring(atIndex + 1);
    return { packageName, version };
  }

  // Handle non-scoped packages (name@version)
  const atIndex = command.indexOf("@");
  if (atIndex === -1) {
    return { packageName: command, version: null };
  }
  const packageName = command.substring(0, atIndex);
  const version = command.substring(atIndex + 1);
  return { packageName, version };
}

/**
 * Resolve version selection to a bunx-compatible suffix
 * Examples:
 *   "installed" -> "" (use bunx default/cached version)
 *   "latest" -> "@latest"
 *   "1.0.3" -> "@1.0.3"
 *   undefined -> "" (use bunx default/cached version)
 */
export function resolveVersionSuffix(version?: VersionSelection): string {
  if (!version || version === "installed") {
    return "";
  }
  return `@${version}`;
}
