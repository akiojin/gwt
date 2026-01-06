/**
 * Keybindings - Centralized keyboard shortcut definitions
 *
 * This module provides a unified keybinding system for the CLI UI.
 *
 * @see specs/SPEC-d27be71b/spec.md - OpenTUI migration spec
 */

// ========================================
// Key Definitions
// ========================================

/**
 * Special key identifiers
 */
export const Keys = {
  // Navigation
  UP: "up",
  DOWN: "down",
  LEFT: "left",
  RIGHT: "right",
  PAGE_UP: "pageUp",
  PAGE_DOWN: "pageDown",
  HOME: "home",
  END: "end",

  // Actions
  ENTER: "return",
  ESCAPE: "escape",
  TAB: "tab",
  SPACE: "space",
  BACKSPACE: "backspace",
  DELETE: "delete",

  // Letters (lowercase)
  A: "a",
  B: "b",
  C: "c",
  D: "d",
  E: "e",
  F: "f",
  G: "g",
  H: "h",
  I: "i",
  J: "j",
  K: "k",
  L: "l",
  M: "m",
  N: "n",
  O: "o",
  P: "p",
  Q: "q",
  R: "r",
  S: "s",
  T: "t",
  U: "u",
  V: "v",
  W: "w",
  X: "x",
  Y: "y",
  Z: "z",
} as const;

export type KeyName = (typeof Keys)[keyof typeof Keys];

// ========================================
// Modifier Keys
// ========================================

/**
 * Modifier key state
 */
export interface Modifiers {
  ctrl?: boolean;
  shift?: boolean;
  alt?: boolean;
  meta?: boolean;
}

/**
 * Full key combination
 */
export interface KeyCombination {
  key: KeyName | string;
  modifiers?: Modifiers;
}

// ========================================
// Action Definitions
// ========================================

/**
 * UI action types
 */
export type UIAction =
  // Navigation
  | "navigate-up"
  | "navigate-down"
  | "navigate-left"
  | "navigate-right"
  | "page-up"
  | "page-down"
  | "go-to-top"
  | "go-to-bottom"
  // Selection
  | "select"
  | "confirm"
  | "cancel"
  | "back"
  // Branch list specific
  | "toggle-filter"
  | "toggle-view-mode"
  | "search"
  | "clear-search"
  | "refresh"
  | "copy-branch-name"
  // Quick actions
  | "quick-create"
  | "quick-delete"
  | "quick-merge"
  // Help
  | "show-help"
  | "hide-help"
  // Application
  | "quit";

// ========================================
// Keybinding Configuration
// ========================================

/**
 * Single keybinding definition
 */
export interface KeybindingDef {
  action: UIAction;
  key: KeyCombination;
  description: string;
  contexts?: string[]; // Which screens this binding applies to
}

/**
 * Default keybindings for the application
 */
export const defaultKeybindings: KeybindingDef[] = [
  // Navigation
  {
    action: "navigate-up",
    key: { key: Keys.UP },
    description: "Move up",
  },
  {
    action: "navigate-down",
    key: { key: Keys.DOWN },
    description: "Move down",
  },
  {
    action: "page-up",
    key: { key: Keys.PAGE_UP },
    description: "Page up",
  },
  {
    action: "page-down",
    key: { key: Keys.PAGE_DOWN },
    description: "Page down",
  },
  {
    action: "go-to-top",
    key: { key: Keys.G, modifiers: { shift: true } },
    description: "Go to top",
  },
  {
    action: "go-to-bottom",
    key: { key: Keys.G },
    description: "Go to bottom (gg)",
  },

  // Selection
  {
    action: "select",
    key: { key: Keys.ENTER },
    description: "Select",
  },
  {
    action: "confirm",
    key: { key: Keys.Y },
    description: "Confirm",
  },
  {
    action: "cancel",
    key: { key: Keys.ESCAPE },
    description: "Cancel",
  },
  {
    action: "back",
    key: { key: Keys.ESCAPE },
    description: "Go back",
  },

  // Branch list specific
  {
    action: "toggle-filter",
    key: { key: Keys.F },
    description: "Toggle filter",
    contexts: ["branch-list"],
  },
  {
    action: "toggle-view-mode",
    key: { key: Keys.V },
    description: "Toggle view mode",
    contexts: ["branch-list"],
  },
  {
    action: "search",
    key: { key: Keys.SPACE },
    description: "Search",
    contexts: ["branch-list"],
  },
  {
    action: "clear-search",
    key: { key: Keys.ESCAPE },
    description: "Clear search",
    contexts: ["branch-list"],
  },
  {
    action: "refresh",
    key: { key: Keys.R },
    description: "Refresh",
    contexts: ["branch-list"],
  },
  {
    action: "copy-branch-name",
    key: { key: Keys.C },
    description: "Copy branch name",
    contexts: ["branch-list"],
  },

  // Quick actions
  {
    action: "quick-create",
    key: { key: Keys.N },
    description: "New branch",
    contexts: ["branch-list"],
  },
  {
    action: "quick-delete",
    key: { key: Keys.D },
    description: "Delete",
    contexts: ["branch-list"],
  },

  // Help
  {
    action: "show-help",
    key: { key: Keys.H },
    description: "Show help",
  },
  {
    action: "show-help",
    key: { key: "?" },
    description: "Show help",
  },
  {
    action: "hide-help",
    key: { key: Keys.ESCAPE },
    description: "Hide help",
    contexts: ["help-overlay"],
  },

  // Application
  {
    action: "quit",
    key: { key: Keys.Q },
    description: "Quit",
  },
  {
    action: "quit",
    key: { key: Keys.C, modifiers: { ctrl: true } },
    description: "Quit (Ctrl+C)",
  },
];

// ========================================
// Keybinding Utilities
// ========================================

/**
 * Get keybindings for a specific context/screen
 */
export function getKeybindingsForContext(context: string): KeybindingDef[] {
  return defaultKeybindings.filter(
    (kb) => !kb.contexts || kb.contexts.includes(context),
  );
}

/**
 * Get keybindings for a specific action
 */
export function getKeybindingsForAction(action: UIAction): KeybindingDef[] {
  return defaultKeybindings.filter((kb) => kb.action === action);
}

/**
 * Format key combination for display
 */
export function formatKeyCombination(combo: KeyCombination): string {
  const parts: string[] = [];

  if (combo.modifiers?.ctrl) parts.push("Ctrl");
  if (combo.modifiers?.alt) parts.push("Alt");
  if (combo.modifiers?.shift) parts.push("Shift");
  if (combo.modifiers?.meta) parts.push("Cmd");

  // Format special keys
  const keyDisplay = formatKeyName(combo.key);
  parts.push(keyDisplay);

  return parts.join("+");
}

/**
 * Format key name for display
 */
function formatKeyName(key: string): string {
  const displayMap: Record<string, string> = {
    [Keys.UP]: "Up",
    [Keys.DOWN]: "Down",
    [Keys.LEFT]: "Left",
    [Keys.RIGHT]: "Right",
    [Keys.ENTER]: "Enter",
    [Keys.ESCAPE]: "Esc",
    [Keys.TAB]: "Tab",
    [Keys.SPACE]: "Space",
    [Keys.BACKSPACE]: "Backspace",
    [Keys.DELETE]: "Del",
    [Keys.PAGE_UP]: "PgUp",
    [Keys.PAGE_DOWN]: "PgDn",
  };

  return displayMap[key] ?? key.toUpperCase();
}

// ========================================
// Footer Actions Builder
// ========================================

/**
 * Build footer actions from keybindings for a context
 */
export function buildFooterActions(
  context: string,
  excludeActions: UIAction[] = [],
): Array<{ key: string; description: string }> {
  const bindings = getKeybindingsForContext(context);
  const seenActions = new Set<UIAction>();

  return bindings
    .filter((kb) => {
      if (excludeActions.includes(kb.action)) return false;
      if (seenActions.has(kb.action)) return false;
      seenActions.add(kb.action);
      return true;
    })
    .map((kb) => ({
      key: formatKeyCombination(kb.key),
      description: kb.description,
    }));
}
