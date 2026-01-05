/**
 * UI Store - UI state management for SolidJS
 *
 * This store manages UI-level concerns like notifications, terminal size,
 * loading states, and modal dialogs.
 *
 * @see specs/SPEC-d27be71b/spec.md - OpenTUI migration spec
 */

import { createRoot } from "solid-js";
import { createStore } from "solid-js/store";
import type {
  Notification,
  NotificationTone,
  TerminalSize,
  LoadingState,
} from "../core/types.js";

// ========================================
// Types
// ========================================

export interface UIStoreState {
  /** Current notification */
  notification: Notification | null;
  /** Terminal dimensions */
  terminalSize: TerminalSize;
  /** Global loading state */
  loading: LoadingState;
  /** Is search mode active */
  searchMode: boolean;
  /** Is confirm dialog visible */
  confirmDialogVisible: boolean;
  /** Confirm dialog message */
  confirmDialogMessage: string;
}

// ========================================
// Initial State
// ========================================

const initialState: UIStoreState = {
  notification: null,
  terminalSize: { width: 80, height: 24 },
  loading: { isLoading: false },
  searchMode: false,
  confirmDialogVisible: false,
  confirmDialogMessage: "",
};

// ========================================
// Store Creation
// ========================================

function createUIStore() {
  const [state, setState] = createStore<UIStoreState>(initialState);

  // Notification timeout handle
  let notificationTimeout: ReturnType<typeof setTimeout> | null = null;

  const actions = {
    /**
     * Show a notification
     */
    showNotification(
      message: string,
      tone: NotificationTone = "info",
      duration = 3000,
    ): void {
      // Clear existing timeout
      if (notificationTimeout) {
        clearTimeout(notificationTimeout);
      }

      setState("notification", {
        message,
        tone,
        timestamp: new Date(),
      });

      // Auto-hide after duration
      if (duration > 0) {
        notificationTimeout = setTimeout(() => {
          setState("notification", null);
        }, duration);
      }
    },

    /**
     * Show success notification
     */
    showSuccess(message: string, duration = 3000): void {
      actions.showNotification(message, "success", duration);
    },

    /**
     * Show error notification
     */
    showError(message: string, duration = 5000): void {
      actions.showNotification(message, "error", duration);
    },

    /**
     * Show warning notification
     */
    showWarning(message: string, duration = 4000): void {
      actions.showNotification(message, "warning", duration);
    },

    /**
     * Clear notification
     */
    clearNotification(): void {
      if (notificationTimeout) {
        clearTimeout(notificationTimeout);
        notificationTimeout = null;
      }
      setState("notification", null);
    },

    /**
     * Update terminal size
     */
    setTerminalSize(size: TerminalSize): void {
      setState("terminalSize", size);
    },

    /**
     * Set loading state
     */
    setLoading(isLoading: boolean, message?: string): void {
      const loadingState: LoadingState = { isLoading };
      if (message !== undefined) loadingState.message = message;
      setState("loading", loadingState);
    },

    /**
     * Set loading progress
     */
    setLoadingProgress(progress: number, message?: string): void {
      const loadingState: LoadingState = { isLoading: true, progress };
      if (message !== undefined) loadingState.message = message;
      setState("loading", loadingState);
    },

    /**
     * Clear loading state
     */
    clearLoading(): void {
      setState("loading", { isLoading: false });
    },

    /**
     * Enter search mode
     */
    enterSearchMode(): void {
      setState("searchMode", true);
    },

    /**
     * Exit search mode
     */
    exitSearchMode(): void {
      setState("searchMode", false);
    },

    /**
     * Toggle search mode
     */
    toggleSearchMode(): void {
      setState("searchMode", (v) => !v);
    },

    /**
     * Show confirm dialog
     */
    showConfirmDialog(message: string): void {
      setState("confirmDialogVisible", true);
      setState("confirmDialogMessage", message);
    },

    /**
     * Hide confirm dialog
     */
    hideConfirmDialog(): void {
      setState("confirmDialogVisible", false);
      setState("confirmDialogMessage", "");
    },
  };

  return {
    state,
    actions,
  };
}

// ========================================
// Singleton Export
// ========================================

let _store: ReturnType<typeof createUIStore> | null = null;

export function getUIStore() {
  if (!_store) {
    createRoot(() => {
      _store = createUIStore();
    });
  }
  // Store is guaranteed to be initialized by createRoot
  return _store as ReturnType<typeof createUIStore>;
}

// Convenience exports
export const uiStore = new Proxy({} as UIStoreState, {
  get: (_, prop) => getUIStore().state[prop as keyof UIStoreState],
});

export const uiActions = new Proxy(
  {} as ReturnType<typeof createUIStore>["actions"],
  {
    get: (_, prop) =>
      getUIStore().actions[
        prop as keyof ReturnType<typeof createUIStore>["actions"]
      ],
  },
);
