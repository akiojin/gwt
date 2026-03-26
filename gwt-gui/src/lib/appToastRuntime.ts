/**
 * Toast / notification state management extracted from App.svelte.
 *
 * Pure functions that operate on an explicit {@link ToastState} object.
 * The module has no side-effects; callers own state mutation and scheduling.
 */

import type { StructuredError } from "./errorBus";
import type { UpdateState } from "./types";

export type ToastAction =
  | { kind: "apply-update"; latest: string }
  | { kind: "report-error"; error: StructuredError }
  | null;

/** Subset of UpdateState narrowed to `"available"`. */
export type AvailableUpdateState = Extract<UpdateState, { state: "available" }>;

export interface ToastState {
  message: string | null;
  action: ToastAction;
  timeout: ReturnType<typeof setTimeout> | null;
  lastUpdateVersion: string | null;
}

export function createToastState(): ToastState {
  return {
    message: null,
    action: null,
    timeout: null,
    lastUpdateVersion: null,
  };
}

export interface ShowToastCallbacks {
  setState: (message: string | null, action: ToastAction) => void;
}

/**
 * Show a toast notification.
 *
 * @param durationMs Auto-dismiss delay in ms. 0 = sticky (no auto-dismiss).
 */
export function showToast(
  state: ToastState,
  message: string,
  durationMs: number,
  action: ToastAction,
  cb: ShowToastCallbacks,
): void {
  cb.setState(message, action);
  state.message = message;
  state.action = action;

  if (state.timeout) clearTimeout(state.timeout);
  state.timeout = null;

  if (durationMs > 0) {
    state.timeout = setTimeout(() => {
      cb.setState(null, null);
      state.message = null;
      state.action = null;
    }, durationMs);
  }
}

/**
 * Show an "update available" toast, de-duplicated by version string.
 *
 * @returns `true` if a toast was shown; `false` if suppressed.
 */
export function showAvailableUpdateToast(
  state: ToastState,
  update: AvailableUpdateState,
  force: boolean,
  cb: ShowToastCallbacks,
): boolean {
  if (!force && state.lastUpdateVersion === update.latest) return false;
  state.lastUpdateVersion = update.latest;

  if (update.asset_url) {
    showToast(
      state,
      `Update available: v${update.latest} (click update)`,
      0,
      { kind: "apply-update", latest: update.latest },
      cb,
    );
  } else {
    showToast(
      state,
      `Update available: v${update.latest}. Manual download required.`,
      15_000,
      null,
      cb,
    );
  }
  return true;
}

/**
 * Dismiss the current toast (clear message + action).
 */
export function dismissToast(state: ToastState, cb: ShowToastCallbacks): void {
  if (state.timeout) clearTimeout(state.timeout);
  state.timeout = null;
  state.message = null;
  state.action = null;
  cb.setState(null, null);
}

export interface ToastSubscriptionDeps {
  toastBus: { subscribe: (handler: (event: { message: string; durationMs?: number }) => void) => () => void };
  errorBus: { subscribe: (handler: (error: StructuredError) => void) => () => void };
  state: ToastState;
  cb: ShowToastCallbacks;
}

/**
 * Wire up toastBus and errorBus subscriptions.
 *
 * @returns A cleanup function that unsubscribes both listeners.
 */
export function setupToastSubscriptions(deps: ToastSubscriptionDeps): () => void {
  const { toastBus, errorBus, state, cb } = deps;

  const unsubToast = toastBus.subscribe((event) => {
    showToast(state, event.message, event.durationMs ?? 5000, null, cb);
  });

  const unsubError = errorBus.subscribe((error) => {
    if (error.severity === "error" || error.severity === "critical") {
      showToast(state, `Error: ${error.message}`, 0, { kind: "report-error", error }, cb);
    }
  });

  return () => {
    unsubToast();
    unsubError();
  };
}
