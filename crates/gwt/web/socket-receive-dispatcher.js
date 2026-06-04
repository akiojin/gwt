// Issue #2694 Phase C — coalesced, rAF-flushed dispatch for WebSocket inbound
// events.
//
// Previously `handleSocketMessage(event)` ran `JSON.parse(event.data)` then
// invoked the 150+ case `receive()` switch synchronously, so a burst of
// inbound events (Codex thinking stream, board updates, workspace_state
// during window operations, ...) saturated the main thread and made clicks /
// tab switches / settings interactions feel stuck on Windows.
//
// `createSocketReceiveDispatcher` wraps `receive` so:
// - inbound events accumulate in a queue,
// - the queue is flushed on the next animation frame,
// - idempotent global-state kinds (e.g. workspace_state) collapse to the
//   latest occurrence, sparing redundant DOM mutations,
// - long streamed-event backlogs deliver a bounded chunk before latest state so
//   tab/project updates are not starved behind terminal output,
// - per-frame time budget (default 8ms) bounds long tasks; remaining events
//   defer to the next frame.

const DEFAULT_BUDGET_MS = 8;
export const DEFAULT_MAX_STREAMED_BEFORE_STATE = 32;

// Idempotent kinds where only the latest occurrence carries information. Any
// kind not in this set preserves original order and every occurrence.
export const DEFAULT_COALESCE_KINDS = Object.freeze(
  new Set([
    "workspace_state",
    "active_work_projection",
    "window_list",
    "provider_usage",
    "project_index_status",
    "launch_wizard_state",
    "launch_wizard_open",
    "agent_options_state",
    "update_state",
    "knowledge_bridge_state",
    "system_status",
  ]),
);

export function createSocketReceiveDispatcher({
  receive,
  schedule,
  now,
  budgetMs = DEFAULT_BUDGET_MS,
  coalesceKinds = DEFAULT_COALESCE_KINDS,
  maxStreamedBeforeState = DEFAULT_MAX_STREAMED_BEFORE_STATE,
  onTrace,
} = {}) {
  if (typeof receive !== "function") {
    throw new TypeError(
      "createSocketReceiveDispatcher requires a receive callback",
    );
  }
  const scheduleImpl = schedule
    ?? ((cb) => {
      if (typeof requestAnimationFrame === "function") {
        return requestAnimationFrame(cb);
      }
      return setTimeout(cb, 0);
    });
  const nowImpl = now ?? (() => {
    if (typeof performance !== "undefined" && typeof performance.now === "function") {
      return performance.now();
    }
    return Date.now();
  });
  const traceImpl = typeof onTrace === "function" ? onTrace : null;

  const queue = [];
  let scheduled = false;

  function trace(kind, fields = {}) {
    if (!traceImpl) {
      return;
    }
    try {
      traceImpl(kind, fields);
    } catch (_) {
      // Diagnostics must never affect the interactive event path.
    }
  }

  function flush() {
    scheduled = false;
    if (queue.length === 0) {
      return;
    }
    const ready = coalesceEvents(queue, coalesceKinds, {
      maxStreamedBeforeState,
    });
    queue.length = 0;
    const start = nowImpl();
    trace("ws_flush_start", {
      ready_count: ready.length,
    });
    let cursor = 0;
    while (cursor < ready.length) {
      const event = ready[cursor];
      const receiveStart = nowImpl();
      try {
        receive(event);
        trace("ws_receive", {
          event_kind: event && event.kind,
          duration_ms: nowImpl() - receiveStart,
        });
      } catch (error) {
        trace("ws_receive", {
          event_kind: event && event.kind,
          duration_ms: nowImpl() - receiveStart,
          threw: true,
          error_name: error && error.name,
        });
        console.warn(
          "[ws-dispatcher] receive threw for %s — continuing with remaining events",
          event && event.kind,
          error,
        );
      }
      cursor += 1;
      if (cursor < ready.length && nowImpl() - start > budgetMs) {
        for (let i = ready.length - 1; i >= cursor; i -= 1) {
          queue.unshift(ready[i]);
        }
        trace("ws_flush_defer", {
          processed_count: cursor,
          remaining_count: ready.length - cursor,
          duration_ms: nowImpl() - start,
        });
        scheduled = true;
        scheduleImpl(flush);
        return;
      }
    }
    trace("ws_flush_end", {
      processed_count: cursor,
      duration_ms: nowImpl() - start,
    });
  }

  function enqueue(event) {
    queue.push(event);
    if (!scheduled) {
      scheduled = true;
      scheduleImpl(flush);
    }
  }

  function handle(messageEvent) {
    let payload;
    const parseStart = nowImpl();
    if (messageEvent && typeof messageEvent.data === "string") {
      payload = JSON.parse(messageEvent.data);
    } else if (
      messageEvent
      && typeof messageEvent === "object"
      && Object.hasOwn(messageEvent, "kind")
    ) {
      payload = messageEvent;
    } else {
      throw new TypeError(
        "createSocketReceiveDispatcher.handle expects a WebSocket message event or parsed payload",
      );
    }
    trace("ws_message", {
      event_kind: payload && payload.kind,
      parse_ms: nowImpl() - parseStart,
    });
    enqueue(payload);
  }

  function flushNow() {
    if (scheduled || queue.length > 0) {
      flush();
    }
  }

  function pendingCount() {
    return queue.length;
  }

  return { handle, enqueue, flushNow, pendingCount };
}

export function coalesceEvents(
  queue,
  coalesceKinds = DEFAULT_COALESCE_KINDS,
  { maxStreamedBeforeState = DEFAULT_MAX_STREAMED_BEFORE_STATE } = {},
) {
  if (!queue || queue.length <= 1) {
    return queue ? queue.slice() : [];
  }
  const streamedChunkLimit = normalizeStreamedChunkLimit(maxStreamedBeforeState);
  const lastIndexByKind = new Map();
  for (let i = 0; i < queue.length; i += 1) {
    const event = queue[i];
    const kind = event && event.kind;
    if (kind && coalesceKinds.has(kind)) {
      lastIndexByKind.set(kind, i);
    }
  }
  if (lastIndexByKind.size === 0) {
    return queue.slice();
  }
  // Issue #2698 PR 3 — partition the result so streamed (non-
  // coalesced) events are delivered ahead of idempotent state
  // updates. terminal_output / notification / error need low
  // round-trip latency; a single rAF tick that flushes 20 piled-up
  // workspace_state messages before the next keystroke echo makes
  // typing feel sluggish on Windows even when CPU is idle. The
  // relative order WITHIN each partition is preserved from the
  // original queue.
  const streamed = [];
  const idempotent = [];
  for (let i = 0; i < queue.length; i += 1) {
    const event = queue[i];
    const kind = event && event.kind;
    if (kind && coalesceKinds.has(kind)) {
      if (lastIndexByKind.get(kind) === i) {
        idempotent.push(event);
      }
    } else {
      streamed.push(event);
    }
  }
  if (streamed.length <= streamedChunkLimit || idempotent.length === 0) {
    return streamed.concat(idempotent);
  }
  return streamed
    .slice(0, streamedChunkLimit)
    .concat(idempotent, streamed.slice(streamedChunkLimit));
}

function normalizeStreamedChunkLimit(value) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    return DEFAULT_MAX_STREAMED_BEFORE_STATE;
  }
  return Math.floor(value);
}
