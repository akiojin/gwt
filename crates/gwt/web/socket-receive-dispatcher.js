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
// - string payloads keep full JSON.parse work inside the scheduled flush budget,
// - idempotent global-state kinds (e.g. workspace_state) collapse to the
//   latest occurrence, sparing redundant DOM mutations,
// - long streamed-event backlogs deliver a bounded chunk before latest state so
//   tab/project updates are not starved behind terminal output,
// - per-frame time budget (default 8ms) bounds long tasks; remaining events
//   defer to the next frame.

const DEFAULT_BUDGET_MS = 8;
export const DEFAULT_MAX_STREAMED_BEFORE_STATE = 32;

// Snapshot kinds that must preserve multiplicity and their position relative
// to coalesced state. They are not latency-sensitive streams: moving them
// ahead of workspace_state can make project-scoped snapshots fail their
// active-project fence.
export const DEFAULT_ORDERED_STATE_KINDS = Object.freeze(
  new Set([
    "improvement_candidates",
    "improvement_action_result",
    "improvement_action_error",
  ]),
);

// Idempotent kinds where only the latest occurrence carries information. Any
// kind not in this set preserves original order and every occurrence.
export const DEFAULT_COALESCE_KINDS = Object.freeze(
  new Set([
    "workspace_state",
    "active_work_projection",
    "window_list",
    "provider_usage",
    "runtime_health",
    "project_index_status",
    "launch_wizard_state",
    "launch_wizard_open",
    "agent_options_state",
    "update_state",
    "knowledge_bridge_state",
    "system_status",
    "issue_monitor_status",
  ]),
);

export function createSocketReceiveDispatcher({
  receive,
  schedule,
  now,
  budgetMs = DEFAULT_BUDGET_MS,
  coalesceKinds = DEFAULT_COALESCE_KINDS,
  orderedStateKinds = DEFAULT_ORDERED_STATE_KINDS,
  maxStreamedBeforeState = DEFAULT_MAX_STREAMED_BEFORE_STATE,
  onTrace,
  shouldTrace,
  readTraceEpoch,
  nextTerminalOutputSequence,
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
  const shouldTraceImpl = typeof shouldTrace === "function" ? shouldTrace : null;
  const readTraceEpochImpl =
    typeof readTraceEpoch === "function" ? readTraceEpoch : null;
  const sequenceAllocatorImpl =
    typeof nextTerminalOutputSequence === "function"
      ? nextTerminalOutputSequence
      : null;

  const queue = [];
  let scheduled = false;
  let terminalOutputTraceSequence = 0;

  function traceActive() {
    if (!traceImpl) {
      return false;
    }
    if (!shouldTraceImpl) {
      return true;
    }
    try {
      return Boolean(shouldTraceImpl());
    } catch (_) {
      return false;
    }
  }

  function trace(kind, fieldsFactory = () => ({})) {
    if (!traceActive()) {
      return;
    }
    try {
      const fields = fieldsFactory();
      traceImpl(kind, fields);
    } catch (_) {
      // Diagnostics must never affect the interactive event path.
    }
  }

  function terminalOutputTraceMetadata(event) {
    if (!event || event.kind !== "terminal_output" || !traceActive()) {
      return null;
    }
    let epoch;
    if (readTraceEpochImpl) {
      try {
        epoch = readTraceEpochImpl();
      } catch (_) {
        return null;
      }
      if (epoch === null || epoch === undefined) {
        return null;
      }
    }
    let sequence;
    try {
      if (sequenceAllocatorImpl) {
        sequence = sequenceAllocatorImpl();
      } else {
        terminalOutputTraceSequence += 1;
        sequence = terminalOutputTraceSequence;
      }
    } catch (_) {
      return null;
    }
    if (
      (typeof sequence !== "number" || !Number.isFinite(sequence))
      && (typeof sequence !== "string" || sequence.length === 0)
    ) {
      return null;
    }
    const metadata = {
      sequence,
      window_id: event.id ?? "",
    };
    if (readTraceEpochImpl) {
      metadata.epoch = epoch;
    }
    return Object.freeze(metadata);
  }

  function emitTerminalOutputTrace(kind, metadata) {
    if (!traceImpl || !metadata) {
      return;
    }
    try {
      traceImpl(kind, {
        sequence: metadata.sequence,
        window_id: metadata.window_id,
      });
    } catch (_) {
      // Diagnostics must never affect WebSocket delivery.
    }
  }

  function flush() {
    scheduled = false;
    if (queue.length === 0) {
      return;
    }
    const ready = coalesceQueuedEntries(queue, coalesceKinds, {
      maxStreamedBeforeState,
      orderedStateKinds,
    });
    queue.length = 0;
    const start = nowImpl();
    trace("ws_flush_start", () => ({
      ready_count: ready.length,
    }));
    let cursor = 0;
    while (cursor < ready.length) {
      const entry = ready[cursor];
      const eventKind = queuedEntryKind(entry);
      const receiveStart = nowImpl();
      try {
        const event = queuedEntryPayload(entry);
        const traceMetadata = terminalOutputTraceMetadata(event);
        if (traceMetadata) {
          emitTerminalOutputTrace("terminal_output_ws_receive", traceMetadata);
          receive(event, traceMetadata);
        } else {
          receive(event);
        }
        trace("ws_receive", () => ({
          event_kind: event && event.kind,
          duration_ms: nowImpl() - receiveStart,
          deferred_parse: entry && entry.type === "raw",
        }));
      } catch (error) {
        trace("ws_receive", () => ({
          event_kind: eventKind,
          duration_ms: nowImpl() - receiveStart,
          threw: true,
          error_name: error && error.name,
        }));
        console.warn(
          "[ws-dispatcher] receive threw for %s — continuing with remaining events",
          eventKind,
          error,
        );
      }
      cursor += 1;
      if (cursor < ready.length && nowImpl() - start > budgetMs) {
        for (let i = ready.length - 1; i >= cursor; i -= 1) {
          queue.unshift(ready[i]);
        }
        trace("ws_flush_defer", () => ({
          processed_count: cursor,
          remaining_count: ready.length - cursor,
          duration_ms: nowImpl() - start,
        }));
        scheduled = true;
        scheduleImpl(flush);
        return;
      }
    }
    trace("ws_flush_end", () => ({
      processed_count: cursor,
      duration_ms: nowImpl() - start,
    }));
  }

  function enqueue(event) {
    queue.push(parsedQueueEntry(event));
    if (!scheduled) {
      scheduled = true;
      scheduleImpl(flush);
    }
  }

  function handle(messageEvent) {
    const parseStart = nowImpl();
    if (messageEvent && typeof messageEvent.data === "string") {
      const entry = rawQueueEntry(messageEvent.data);
      trace("ws_message", () => ({
        event_kind: entry.kind,
        parse_ms: nowImpl() - parseStart,
        deferred_parse: true,
      }));
      queue.push(entry);
    } else if (
      messageEvent
      && typeof messageEvent === "object"
      && Object.hasOwn(messageEvent, "kind")
    ) {
      const entry = parsedQueueEntry(messageEvent);
      trace("ws_message", () => ({
        event_kind: entry.kind,
        parse_ms: nowImpl() - parseStart,
      }));
      queue.push(entry);
    } else {
      throw new TypeError(
        "createSocketReceiveDispatcher.handle expects a WebSocket message event or parsed payload",
      );
    }
    if (!scheduled) {
      scheduled = true;
      scheduleImpl(flush);
    }
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

function parsedQueueEntry(event) {
  return {
    type: "parsed",
    kind: event && event.kind,
    payload: event,
  };
}

const KIND_HINT_PATTERN = /"kind"\s*:\s*"([^"\\]*)"/;

function rawQueueEntry(data) {
  return {
    type: "raw",
    kind: extractKindHint(data),
    payload: data,
  };
}

function extractKindHint(data) {
  if (typeof data !== "string") {
    return "";
  }
  const match = KIND_HINT_PATTERN.exec(data);
  return match ? match[1] : "";
}

function queuedEntryKind(entry) {
  return entry && entry.kind;
}

function queuedEntryPayload(entry) {
  if (entry && entry.type === "raw") {
    return JSON.parse(entry.payload);
  }
  return entry ? entry.payload : entry;
}

function coalesceQueuedEntries(
  queue,
  coalesceKinds = DEFAULT_COALESCE_KINDS,
  {
    maxStreamedBeforeState = DEFAULT_MAX_STREAMED_BEFORE_STATE,
    orderedStateKinds = DEFAULT_ORDERED_STATE_KINDS,
  } = {},
) {
  return coalesceByKind(queue, coalesceKinds, {
    maxStreamedBeforeState,
    orderedStateKinds,
    kindFor: queuedEntryKind,
  });
}

export function coalesceEvents(
  queue,
  coalesceKinds = DEFAULT_COALESCE_KINDS,
  {
    maxStreamedBeforeState = DEFAULT_MAX_STREAMED_BEFORE_STATE,
    orderedStateKinds = DEFAULT_ORDERED_STATE_KINDS,
  } = {},
) {
  return coalesceByKind(queue, coalesceKinds, {
    maxStreamedBeforeState,
    orderedStateKinds,
    kindFor: (event) => event && event.kind,
  });
}

function coalesceByKind(
  queue,
  coalesceKinds = DEFAULT_COALESCE_KINDS,
  {
    maxStreamedBeforeState = DEFAULT_MAX_STREAMED_BEFORE_STATE,
    orderedStateKinds = DEFAULT_ORDERED_STATE_KINDS,
    kindFor,
  } = {},
) {
  if (!queue || queue.length <= 1) {
    return queue ? queue.slice() : [];
  }
  const streamedChunkLimit = normalizeStreamedChunkLimit(maxStreamedBeforeState);
  const lastIndexByKind = new Map();
  for (let i = 0; i < queue.length; i += 1) {
    const kind = kindFor(queue[i]);
    if (kind && coalesceKinds.has(kind)) {
      lastIndexByKind.set(kind, i);
    }
  }
  if (lastIndexByKind.size === 0) {
    return queue.slice();
  }
  // Issue #2698 PR 3 — partition the result so streamed events are
  // delivered ahead of state updates. terminal_output / notification / error need low
  // round-trip latency; a single rAF tick that flushes 20 piled-up
  // workspace_state messages before the next keystroke echo makes
  // typing feel sluggish on Windows even when CPU is idle. The
  // relative order WITHIN each partition is preserved from the
  // original queue.
  const streamed = [];
  const activationState = [];
  const state = [];
  for (let i = 0; i < queue.length; i += 1) {
    const event = queue[i];
    const kind = kindFor(event);
    if (kind && coalesceKinds.has(kind)) {
      if (lastIndexByKind.get(kind) === i) {
        if (kind === "workspace_state") {
          activationState.push(event);
        } else {
          state.push(event);
        }
      }
    } else if (kind && orderedStateKinds.has(kind)) {
      state.push(event);
    } else {
      streamed.push(event);
    }
  }
  // Apply the surviving project activation before scoped snapshots and action
  // outcomes. A dropped intermediate workspace_state must not leave a stale
  // project outcome ahead of the latest active-project fence.
  const orderedState = activationState.concat(state);
  if (streamed.length <= streamedChunkLimit || orderedState.length === 0) {
    return streamed.concat(orderedState);
  }
  return streamed
    .slice(0, streamedChunkLimit)
    .concat(orderedState, streamed.slice(streamedChunkLimit));
}

function normalizeStreamedChunkLimit(value) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    return DEFAULT_MAX_STREAMED_BEFORE_STATE;
  }
  return Math.floor(value);
}
