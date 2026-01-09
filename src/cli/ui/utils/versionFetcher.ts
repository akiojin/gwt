/**
 * Version fetcher for coding agents
 *
 * Provides functions to fetch version information from npm registry
 * and local installations.
 */

import type { VersionInfo } from "../../../utils/npmRegistry.js";
import {
  fetchPackageVersions,
  parsePackageCommand,
} from "../../../utils/npmRegistry.js";
import { BUILTIN_CODING_AGENTS } from "../../../config/builtin-coding-agents.js";
import { findCommand } from "../../../utils/command.js";
import type { SelectInputItem } from "../components/solid/SelectInput.js";

/**
 * Get package name for an agent ID (only for bunx-type agents)
 */
export function getPackageNameForAgent(agentId: string): string | null {
  const agent = BUILTIN_CODING_AGENTS.find((a) => a.id === agentId);
  if (!agent || agent.type !== "bunx") {
    return null;
  }
  const { packageName } = parsePackageCommand(agent.command);
  return packageName;
}

/**
 * Fetch version options for an agent from npm registry
 */
export async function fetchVersionOptionsForAgent(
  agentId: string,
): Promise<VersionInfo[]> {
  const packageName = getPackageNameForAgent(agentId);
  if (!packageName) {
    return [];
  }
  return fetchPackageVersions(packageName);
}

/**
 * Agent ID to command name mapping
 */
const AGENT_COMMAND_MAP: Record<string, string> = {
  "claude-code": "claude",
  "codex-cli": "codex",
  "gemini-cli": "gemini",
  opencode: "opencode",
};

/**
 * Installed version information
 */
export interface InstalledVersionInfo {
  version: string;
  path: string;
}

/**
 * Fetch installed version information for an agent
 * Returns null if not installed locally
 */
export async function fetchInstalledVersionForAgent(
  agentId: string,
): Promise<InstalledVersionInfo | null> {
  const commandName = AGENT_COMMAND_MAP[agentId];
  if (!commandName) {
    return null;
  }

  const result = await findCommand(commandName);
  if (result.source !== "installed" || !result.path) {
    return null;
  }

  // Version format: v1.0.3 -> 1.0.3
  const version = result.version?.replace(/^v/, "") ?? "unknown";

  return {
    version,
    path: result.path,
  };
}

/**
 * Convert VersionInfo to SelectInputItem for UI
 */
export function versionInfoToSelectItem(v: VersionInfo): SelectInputItem {
  const item: SelectInputItem = {
    label: v.isPrerelease ? `${v.version} (pre)` : v.version,
    value: v.version,
  };
  if (v.publishedAt) {
    item.description = new Date(v.publishedAt).toLocaleDateString();
  }
  return item;
}

/**
 * Create installed option for SelectInput
 */
export function createInstalledOption(
  installed: InstalledVersionInfo,
): SelectInputItem {
  return {
    label: `installed@${installed.version}`,
    value: "installed",
    description: installed.path,
  };
}

/**
 * Get all bunx-type agent IDs
 */
export function getBunxAgentIds(): string[] {
  return BUILTIN_CODING_AGENTS.filter((a) => a.type === "bunx").map(
    (a) => a.id,
  );
}
