// Issue #2698 PR 2 (B1) — viewport-persist-throttle primitive
//
// Reduces the WS `update_viewport` rate from ~60-120 msg/sec
// (every wheel/zoom event) to at most a tiny handful per second:
// - tail debounce of `tailMs` after the last schedule call sends
//   the latest payload once, and
// - a max-wait bound of `maxWaitMs` forces a flush during sustained
//   gestures so the backend never lags too far behind the user's
//   actual viewport.
//
// `flushNow()` is provided for definitive commit points
// (pointerup, window close, visibility change) that should not
// wait for the debounce window to close.
//
// The send callback is invoked with the most recent payload value
// at the time of dispatch; intermediate values are dropped.

export function createViewportPersistThrottle({
  send,
  tailMs = 100,
  maxWaitMs = 500,
  now,
  setTimeoutImpl,
  clearTimeoutImpl,
} = {}) {
  if (typeof send !== "function") {
    throw new TypeError("createViewportPersistThrottle requires a send callback");
  }
  const nowFn = now ?? (() => {
    if (typeof performance !== "undefined" && typeof performance.now === "function") {
      return performance.now();
    }
    return Date.now();
  });
  const setT = setTimeoutImpl
    ?? ((cb, delay) => setTimeout(cb, delay));
  const clearT = clearTimeoutImpl ?? ((id) => clearTimeout(id));

  let timerId = null;
  let firstPendingAt = 0;
  let latestPayload;
  let hasPending = false;

  function clearTimer() {
    if (timerId !== null) {
      clearT(timerId);
      timerId = null;
    }
  }

  function dispatch() {
    timerId = null;
    if (!hasPending) {
      return;
    }
    const payload = latestPayload;
    hasPending = false;
    latestPayload = undefined;
    firstPendingAt = 0;
    send(payload);
  }

  function schedule(payload) {
    latestPayload = payload;
    if (!hasPending) {
      hasPending = true;
      firstPendingAt = nowFn();
    }
    clearTimer();
    const elapsed = nowFn() - firstPendingAt;
    const remainingMaxWait = Math.max(0, maxWaitMs - elapsed);
    const delay = Math.min(tailMs, remainingMaxWait);
    timerId = setT(dispatch, delay);
  }

  function flushNow() {
    clearTimer();
    if (!hasPending) {
      return;
    }
    const payload = latestPayload;
    hasPending = false;
    latestPayload = undefined;
    firstPendingAt = 0;
    send(payload);
  }

  function hasPendingValue() {
    return hasPending;
  }

  return { schedule, flushNow, hasPendingValue };
}
