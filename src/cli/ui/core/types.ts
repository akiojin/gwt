/**
 * Core UI Types - Framework-agnostic type definitions
 *
 * These types are designed to be shared between Ink.js (React) and OpenTUI (SolidJS)
 * implementations. They contain no framework-specific imports.
 *
 * @see specs/SPEC-d27be71b/spec.md - OpenTUI migration spec
 */

// ========================================
// Navigation & Screen State
// ========================================

/**
 * Screen types for UI navigation
 */
export type ScreenType =
  | "branch-list"
  | "log-list"
  | "log-detail"
  | "log-date-picker"
  | "branch-creator"
  | "branch-action-selector"
  | "branch-quick-start"
  | "coding-agent-selector"
  | "model-selector"
  | "session-selector"
  | "execution-mode-selector"
  | "batch-merge-progress"
  | "batch-merge-result"
  | "environment-profile"
  | "help-overlay"; // New: OpenTUI overlay feature

/**
 * Screen visibility state
 */
export type ScreenState = "active" | "hidden";

/**
 * Generic screen definition
 */
export interface Screen<T = unknown> {
  type: ScreenType;
  state: ScreenState;
  data?: T;
}

// ========================================
// Selection & Navigation State
// ========================================

/**
 * Generic selection state for lists
 */
export interface SelectionState {
  selectedIndex: number;
  highlightedIndex: number;
  scrollOffset: number;
}

/**
 * Scroll state for virtualized lists
 */
export interface ScrollState {
  offset: number;
  visibleCount: number;
  totalCount: number;
}

/**
 * Filter state for branch lists
 */
export interface FilterState {
  searchQuery: string;
  showWithWorktree: boolean;
  showWithoutWorktree: boolean;
  showLocal: boolean;
  showRemote: boolean;
  branchTypes: BranchCategory[];
}

// ========================================
// Branch Types
// ========================================

/**
 * Branch category for filtering and display
 */
export type BranchCategory =
  | "feature"
  | "bugfix"
  | "hotfix"
  | "release"
  | "main"
  | "develop"
  | "other";

/**
 * Branch view mode
 */
export type BranchViewMode = "all" | "local" | "remote";

/**
 * Branch data type
 */
export type BranchDataType = "local" | "remote";

/**
 * Sync status with remote
 */
export type SyncStatus =
  | "up-to-date"
  | "ahead"
  | "behind"
  | "diverged"
  | "no-upstream"
  | "remote-only";

/**
 * Worktree status
 */
export type WorktreeStatus = "active" | "inaccessible" | undefined;

// ========================================
// Layout & Terminal
// ========================================

/**
 * Terminal dimensions
 */
export interface TerminalSize {
  width: number;
  height: number;
}

/**
 * Layout configuration
 */
export interface LayoutConfig {
  terminalSize: TerminalSize;
  headerLines: number;
  footerLines: number;
  contentHeight: number;
}

// ========================================
// Statistics
// ========================================

/**
 * Branch list statistics
 */
export interface BranchStatistics {
  localCount: number;
  remoteCount: number;
  worktreeCount: number;
  changesCount: number;
  lastUpdated: Date;
}

// ========================================
// Actions & Events
// ========================================

/**
 * Branch action types
 */
export type BranchAction = "use-existing" | "create-new";

/**
 * Footer action definition
 */
export interface FooterAction {
  key: string;
  description: string;
}

// ========================================
// Notification & Feedback
// ========================================

/**
 * Notification tone for visual feedback
 */
export type NotificationTone = "info" | "success" | "warning" | "error";

/**
 * Notification message
 */
export interface Notification {
  message: string;
  tone: NotificationTone;
  timestamp?: Date;
}

// ========================================
// Loading & Progress
// ========================================

/**
 * Loading state
 */
export interface LoadingState {
  isLoading: boolean;
  message?: string;
  progress?: number; // 0-100
}

/**
 * Async operation state
 */
export type AsyncState<T> =
  | { status: "idle" }
  | { status: "loading"; message?: string }
  | { status: "success"; data: T }
  | { status: "error"; error: Error };

// ========================================
// Merge Operations
// ========================================

/**
 * Merge phase
 */
export type MergePhase = "fetch" | "worktree" | "merge" | "push" | "cleanup";

/**
 * Merge status
 */
export type MergeStatus = "success" | "skipped" | "failed";

/**
 * Push status
 */
export type PushStatus = "success" | "failed" | "not_executed";
