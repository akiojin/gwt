/**
 * App Store - Application-wide state management for SolidJS
 *
 * This store manages navigation, screen state, and application-level concerns.
 * Uses SolidJS's fine-grained reactivity with signals and stores.
 *
 * @see specs/SPEC-d27be71b/spec.md - OpenTUI migration spec
 */

import { createSignal, createRoot } from "solid-js";
import { createStore } from "solid-js/store";
import type { ScreenType, Screen } from "../core/types.js";

// ========================================
// Types
// ========================================

export interface AppState {
  /** Current active screen */
  currentScreen: ScreenType;
  /** Screen stack for navigation history */
  screenStack: Screen[];
  /** Is help overlay visible */
  helpVisible: boolean;
  /** Application version */
  version: string | null;
  /** Current working directory */
  workingDirectory: string;
  /** Active profile name */
  activeProfile: string | null;
}

// ========================================
// Initial State
// ========================================

const initialState: AppState = {
  currentScreen: "branch-list",
  screenStack: [],
  helpVisible: false,
  version: null,
  workingDirectory: process.cwd(),
  activeProfile: null,
};

// ========================================
// Store Creation
// ========================================

/**
 * Create the app store with actions
 *
 * Usage in SolidJS components:
 * ```tsx
 * import { appStore, appActions } from './stores/appStore';
 *
 * function MyComponent() {
 *   return (
 *     <div>
 *       <p>Current screen: {appStore.currentScreen}</p>
 *       <button onClick={() => appActions.navigateTo('log-list')}>
 *         Go to Logs
 *       </button>
 *     </div>
 *   );
 * }
 * ```
 */
function createAppStore() {
  const [state, setState] = createStore<AppState>(initialState);

  // Derived signals
  const [isLoading, setIsLoading] = createSignal(false);

  const actions = {
    /**
     * Navigate to a new screen
     */
    navigateTo(screen: ScreenType, data?: unknown): void {
      setState("screenStack", (stack) => [
        ...stack,
        { type: state.currentScreen, state: "hidden", data },
      ]);
      setState("currentScreen", screen);
    },

    /**
     * Go back to previous screen
     */
    goBack(): boolean {
      const stack = state.screenStack;
      if (stack.length === 0) return false;

      const previousScreen = stack[stack.length - 1];
      if (previousScreen) {
        setState("screenStack", (s) => s.slice(0, -1));
        setState("currentScreen", previousScreen.type);
        return true;
      }
      return false;
    },

    /**
     * Reset navigation to initial screen
     */
    resetNavigation(): void {
      setState("screenStack", []);
      setState("currentScreen", "branch-list");
    },

    /**
     * Toggle help overlay visibility
     */
    toggleHelp(): void {
      setState("helpVisible", (v) => !v);
    },

    /**
     * Show help overlay
     */
    showHelp(): void {
      setState("helpVisible", true);
    },

    /**
     * Hide help overlay
     */
    hideHelp(): void {
      setState("helpVisible", false);
    },

    /**
     * Set application version
     */
    setVersion(version: string | null): void {
      setState("version", version);
    },

    /**
     * Set working directory
     */
    setWorkingDirectory(dir: string): void {
      setState("workingDirectory", dir);
    },

    /**
     * Set active profile
     */
    setActiveProfile(profile: string | null): void {
      setState("activeProfile", profile);
    },

    /**
     * Initialize app state
     */
    initialize(config: {
      version?: string | null;
      workingDirectory?: string;
      activeProfile?: string | null;
    }): void {
      if (config.version !== undefined) setState("version", config.version);
      if (config.workingDirectory)
        setState("workingDirectory", config.workingDirectory);
      if (config.activeProfile !== undefined)
        setState("activeProfile", config.activeProfile);
    },
  };

  return {
    state,
    actions,
    // Derived state
    isLoading,
    setIsLoading,
  };
}

// ========================================
// Singleton Export
// ========================================

// Create store in a reactive root for proper cleanup
let _store: ReturnType<typeof createAppStore> | null = null;

export function getAppStore() {
  if (!_store) {
    createRoot(() => {
      _store = createAppStore();
    });
  }
  // Store is guaranteed to be initialized by createRoot
  return _store as ReturnType<typeof createAppStore>;
}

// Convenience exports for direct access
export const appStore = new Proxy({} as AppState, {
  get: (_, prop) => getAppStore().state[prop as keyof AppState],
});

export const appActions = new Proxy(
  {} as ReturnType<typeof createAppStore>["actions"],
  {
    get: (_, prop) =>
      getAppStore().actions[
        prop as keyof ReturnType<typeof createAppStore>["actions"]
      ],
  },
);
