// Issue #2698 PR 1 (B7) — interaction-guard primitive
//
// Coalesces inbound idempotent state updates into "at most one latest
// pending value" while a guard is active. On release the latest
// pending value (if any) is flushed via the onFlush callback.
//
// Wired by app.js to defer `renderLaunchWizard()` while the user has
// a native <select> dropdown open. The destructive DOM swap in
// `renderLaunchWizard()` would otherwise pull the <select> out from
// under the OS dropdown overlay mid-interaction and lose the user's
// selection.

export function createInteractionGuard({ onFlush } = {}) {
  let active = false;
  let pendingValue;
  let hasPending = false;

  function isActive() {
    return active;
  }

  function activate() {
    active = true;
  }

  function release() {
    const wasActive = active;
    active = false;
    if (!wasActive || !hasPending) {
      return;
    }
    const value = pendingValue;
    pendingValue = undefined;
    hasPending = false;
    if (typeof onFlush === "function") {
      onFlush(value);
    }
  }

  function defer(value) {
    if (!active) {
      return false;
    }
    pendingValue = value;
    hasPending = true;
    return true;
  }

  function hasPendingValue() {
    return hasPending;
  }

  function peekPending() {
    return hasPending ? pendingValue : undefined;
  }

  function discard() {
    active = false;
    pendingValue = undefined;
    hasPending = false;
  }

  return {
    isActive,
    activate,
    release,
    defer,
    hasPendingValue,
    peekPending,
    discard,
  };
}
