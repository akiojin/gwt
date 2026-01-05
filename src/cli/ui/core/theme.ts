/**
 * Theme System - Centralized color and icon definitions
 *
 * This module provides a unified theme system that can be used by both
 * Ink.js (React) and OpenTUI (SolidJS) implementations.
 *
 * @see specs/SPEC-d27be71b/spec.md - OpenTUI migration spec
 */

// ========================================
// Color Definitions
// ========================================

/**
 * Semantic color names used throughout the UI
 */
export const colors = {
  // Primary colors
  primary: "cyan",
  secondary: "gray",

  // Status colors
  success: "green",
  warning: "yellow",
  error: "red",
  info: "blue",

  // UI element colors
  text: "white",
  textDim: "gray",
  textMuted: "gray",

  // Interactive element colors
  selected: "cyan",
  highlighted: "cyan",
  disabled: "gray",

  // Branch type colors
  branchFeature: "green",
  branchBugfix: "yellow",
  branchHotfix: "red",
  branchRelease: "magenta",
  branchMain: "cyan",
  branchDevelop: "blue",
  branchOther: "gray",

  // Sync status colors
  syncUpToDate: "green",
  syncAhead: "cyan",
  syncBehind: "yellow",
  syncDiverged: "red",
  syncNoUpstream: "gray",

  // Worktree status colors
  worktreeActive: "green",
  worktreeInaccessible: "red",

  // Progress colors
  progressFilled: "green",
  progressEmpty: "gray",
  progressPhase: "yellow",
  progressTime: "magenta",
} as const;

export type ColorName = keyof typeof colors;
export type ColorValue = (typeof colors)[ColorName];

// ========================================
// Icon Definitions
// ========================================

/**
 * ASCII icons used throughout the UI
 */
export const icons = {
  // Navigation
  pointer: ">",
  pointerBold: ">",
  arrowUp: "^",
  arrowDown: "v",
  arrowLeft: "<",
  arrowRight: ">",

  // Status
  success: "OK",
  error: "X",
  warning: "!",
  info: "i",
  skipped: "-",

  // Branch indicators
  worktree: "W",
  worktreeActive: "A",
  worktreeInaccessible: "X",
  changes: "C",
  unpushed: "U",
  current: "*",
  pr: "PR",
  merged: "M",

  // Sync status
  syncUpToDate: "=",
  syncAhead: ">",
  syncBehind: "<",
  syncDiverged: "!",

  // Loading
  spinnerFrames: ["|", "/", "-", "\\"],

  // Misc
  bullet: "*",
  divider: "-",
  verticalLine: "|",
  corner: "+",
  branch: "+",
} as const;

export type IconName = keyof typeof icons;

// ========================================
// Branch Type Theme
// ========================================

/**
 * Get color for branch type
 */
export function getBranchTypeColor(
  branchType:
    | "feature"
    | "bugfix"
    | "hotfix"
    | "release"
    | "main"
    | "develop"
    | "other",
): ColorValue {
  const colorMap: Record<string, ColorValue> = {
    feature: colors.branchFeature,
    bugfix: colors.branchBugfix,
    hotfix: colors.branchHotfix,
    release: colors.branchRelease,
    main: colors.branchMain,
    develop: colors.branchDevelop,
    other: colors.branchOther,
  };
  return colorMap[branchType] ?? colors.textDim;
}

// ========================================
// Sync Status Theme
// ========================================

/**
 * Get color and icon for sync status
 */
export function getSyncStatusTheme(
  status:
    | "up-to-date"
    | "ahead"
    | "behind"
    | "diverged"
    | "no-upstream"
    | "remote-only",
): { color: ColorValue; icon: string } {
  const themeMap: Record<string, { color: ColorValue; icon: string }> = {
    "up-to-date": { color: colors.syncUpToDate, icon: icons.syncUpToDate },
    ahead: { color: colors.syncAhead, icon: icons.syncAhead },
    behind: { color: colors.syncBehind, icon: icons.syncBehind },
    diverged: { color: colors.syncDiverged, icon: icons.syncDiverged },
    "no-upstream": { color: colors.syncNoUpstream, icon: "" },
    "remote-only": { color: colors.textDim, icon: "" },
  };
  return themeMap[status] ?? { color: colors.textDim, icon: "" };
}

// ========================================
// Notification Theme
// ========================================

/**
 * Get color for notification tone
 */
export function getNotificationColor(
  tone: "info" | "success" | "warning" | "error",
): ColorValue {
  const colorMap: Record<string, ColorValue> = {
    info: colors.info,
    success: colors.success,
    warning: colors.warning,
    error: colors.error,
  };
  return colorMap[tone] ?? colors.text;
}

// ========================================
// Merge Status Theme
// ========================================

/**
 * Get color and icon for merge status
 */
export function getMergeStatusTheme(status: "success" | "skipped" | "failed"): {
  color: ColorValue;
  icon: string;
} {
  const themeMap: Record<string, { color: ColorValue; icon: string }> = {
    success: { color: colors.success, icon: icons.success },
    skipped: { color: colors.warning, icon: icons.skipped },
    failed: { color: colors.error, icon: icons.error },
  };
  return themeMap[status] ?? { color: colors.textDim, icon: "" };
}

// ========================================
// Stats Item Theme
// ========================================

/**
 * Stats display item configuration
 */
export interface StatsItemConfig {
  label: string;
  icon: string;
  color: ColorValue;
}

/**
 * Default stats items configuration
 */
export const statsItemsConfig: Record<string, StatsItemConfig> = {
  local: { label: "Local", icon: "L", color: colors.primary },
  remote: { label: "Remote", icon: "R", color: colors.info },
  worktree: { label: "Worktrees", icon: icons.worktree, color: colors.success },
  changes: { label: "Changes", icon: icons.changes, color: colors.warning },
};
