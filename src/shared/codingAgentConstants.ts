/**
 * Shared constants for Coding Agent integrations.
 *
 * These values are consumed by both the CLI (Node) runtime and the Web UI so
 * that command previews, permission flags, and default arguments stay in sync.
 */

export const CLAUDE_PERMISSION_SKIP_ARGS = [
  "--dangerously-skip-permissions",
] as const;

export const CODEX_DEFAULT_ARGS = [
  "--enable",
  "web_search_request",
  "--model=gpt-5.2-codex",
  "--sandbox",
  "workspace-write",
  "-c",
  "model_reasoning_effort=high",
  "-c",
  "model_reasoning_summaries=detailed",
  "-c",
  "sandbox_workspace_write.network_access=true",
  "-c",
  "shell_environment_policy.inherit=all",
  "-c",
  "shell_environment_policy.ignore_default_excludes=true",
  "-c",
  "shell_environment_policy.experimental_use_profile=true",
] as const;

export const CODEX_SKILLS_FLAG_DEPRECATED_FROM = "0.80.0";

type ParsedVersion = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string | null;
};

const MODEL_FLAG_PREFIX = "--model=";

function normalizeVersion(value?: string | null): string | null {
  if (!value) return null;
  const trimmed = value.trim();
  if (!trimmed) return null;
  return trimmed.replace(/^v/i, "");
}

function parseVersion(value?: string | null): ParsedVersion | null {
  const normalized = normalizeVersion(value);
  if (!normalized) return null;
  const match = normalized.match(
    /^(\d+)\.(\d+)(?:\.(\d+))?(?:-([0-9A-Za-z.-]+))?$/,
  );
  if (!match) return null;
  const major = Number(match[1]);
  const minor = Number(match[2]);
  const patch = Number(match[3] ?? "0");
  if (![major, minor, patch].every(Number.isFinite)) return null;
  return {
    major,
    minor,
    patch,
    prerelease: match[4] ?? null,
  };
}

function compareVersions(a: ParsedVersion, b: ParsedVersion): number {
  if (a.major !== b.major) return a.major - b.major;
  if (a.minor !== b.minor) return a.minor - b.minor;
  if (a.patch !== b.patch) return a.patch - b.patch;
  if (a.prerelease && !b.prerelease) return -1;
  if (!a.prerelease && b.prerelease) return 1;
  if (a.prerelease && b.prerelease) {
    return a.prerelease.localeCompare(b.prerelease);
  }
  return 0;
}

export function shouldEnableCodexSkillsFlag(version?: string | null): boolean {
  const parsed = parseVersion(version);
  if (!parsed) return false;
  const threshold = parseVersion(CODEX_SKILLS_FLAG_DEPRECATED_FROM);
  if (!threshold) return false;
  return compareVersions(parsed, threshold) < 0;
}

export function withCodexSkillsFlag(
  args: readonly string[],
  enable: boolean,
): string[] {
  if (!enable) return Array.from(args);
  const alreadyEnabled = args.some(
    (arg, index) => arg === "--enable" && args[index + 1] === "skills",
  );
  if (alreadyEnabled) return Array.from(args);
  const next = Array.from(args);
  const modelIndex = next.findIndex((arg) => arg.startsWith(MODEL_FLAG_PREFIX));
  const insertIndex = modelIndex === -1 ? next.length : modelIndex;
  next.splice(insertIndex, 0, "--enable", "skills");
  return next;
}
