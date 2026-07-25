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
const WORKSPACE_REVISION_ABSENT = Object.freeze({
  status: "absent",
  value: null,
});
const WORKSPACE_REVISION_INVALID = Object.freeze({
  status: "invalid",
  value: null,
});

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

// Result events delimit independent navigation/coalescing segments. A later
// workspace_state may replace an earlier state only inside the same segment;
// crossing a result would let a rapid A→B→A acknowledgement overtake the
// canonical snapshot that belongs before it.
export const DEFAULT_ORDERING_BARRIER_KINDS = Object.freeze(
  new Set(["navigation_result"]),
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
  orderingBarrierKinds = DEFAULT_ORDERING_BARRIER_KINDS,
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

  let queue = [];
  let scheduled = false;
  let terminalOutputTraceSequence = 0;
  let lastWorkspaceRevision = null;
  let receivedVersionedWorkspace = false;

  function acceptsWorkspaceRevision(event) {
    if (!event || event.kind !== "workspace_state") {
      return true;
    }
    const revisionInfo = workspaceRevisionInfo(event);
    if (revisionInfo.status === "invalid") {
      return false;
    }
    if (revisionInfo.status === "absent") {
      return !receivedVersionedWorkspace;
    }
    const revision = revisionInfo.value;
    if (
      lastWorkspaceRevision !== null
      && revision < lastWorkspaceRevision
    ) {
      return false;
    }
    receivedVersionedWorkspace = true;
    lastWorkspaceRevision =
      lastWorkspaceRevision === null
        ? revision
        : Math.max(lastWorkspaceRevision, revision);
    return true;
  }

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
      orderingBarrierKinds,
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
        if (!acceptsWorkspaceRevision(event)) {
          trace("ws_receive", () => ({
            event_kind: event && event.kind,
            duration_ms: nowImpl() - receiveStart,
            deferred_parse: entry && entry.type === "raw",
            stale_revision: true,
          }));
        } else {
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
        }
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
        queue = ready.slice(cursor).concat(queue);
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
    revisionInfo: workspaceRevisionInfo(event),
    payload: event,
  };
}

const KIND_HINT_PATTERN = /"kind"\s*:\s*"([^"\\]*)"/;
const REVISION_KEY_HINT = '"revision"';
const NON_NEGATIVE_INTEGER_PATTERN = /^(?:0|[1-9]\d*)$/;

function rawQueueEntry(data) {
  const kind = extractKindHint(data);
  return {
    type: "raw",
    kind,
    revisionInfo:
      kind === "workspace_state"
        ? extractWorkspaceRevisionInfoHint(data)
        : WORKSPACE_REVISION_ABSENT,
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

function extractWorkspaceRevisionInfoHint(data) {
  if (typeof data !== "string") {
    return WORKSPACE_REVISION_ABSENT;
  }
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let index = 0; index < data.length; index += 1) {
    const char = data[index];
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (char === "\\") {
        escaped = true;
      } else if (char === '"') {
        inString = false;
      }
      continue;
    }
    if (char === '"') {
      if (depth === 1 && data.startsWith(REVISION_KEY_HINT, index)) {
        let cursor = index + REVISION_KEY_HINT.length;
        while (/\s/.test(data[cursor] || "")) {
          cursor += 1;
        }
        if (data[cursor] === ":") {
          cursor += 1;
          while (/\s/.test(data[cursor] || "")) {
            cursor += 1;
          }
          const valueStart = cursor;
          while (
            cursor < data.length &&
            data[cursor] !== "," &&
            data[cursor] !== "}"
          ) {
            cursor += 1;
          }
          const token = data.slice(valueStart, cursor).trim();
          if (!NON_NEGATIVE_INTEGER_PATTERN.test(token)) {
            return WORKSPACE_REVISION_INVALID;
          }
          const revision = safeRevision(Number(token));
          return revision === null
            ? WORKSPACE_REVISION_INVALID
            : { status: "valid", value: revision };
        }
      }
      inString = true;
      continue;
    }
    if (char === "{" || char === "[") {
      depth += 1;
    } else if (char === "}" || char === "]") {
      depth = Math.max(0, depth - 1);
    }
  }
  return WORKSPACE_REVISION_ABSENT;
}

function safeRevision(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function workspaceRevisionInfo(event) {
  if (
    event?.kind !== "workspace_state" ||
    !Object.hasOwn(event, "revision")
  ) {
    return WORKSPACE_REVISION_ABSENT;
  }
  const revision = safeRevision(event.revision);
  return revision === null
    ? WORKSPACE_REVISION_INVALID
    : { status: "valid", value: revision };
}

function queuedEntryKind(entry) {
  return entry && entry.kind;
}

function queuedEntryRevisionInfo(entry) {
  return entry?.revisionInfo ?? WORKSPACE_REVISION_ABSENT;
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
    orderingBarrierKinds = DEFAULT_ORDERING_BARRIER_KINDS,
  } = {},
) {
  return coalesceByKind(queue, coalesceKinds, {
    maxStreamedBeforeState,
    orderedStateKinds,
    orderingBarrierKinds,
    kindFor: queuedEntryKind,
    revisionInfoFor: queuedEntryRevisionInfo,
  });
}

export function coalesceEvents(
  queue,
  coalesceKinds = DEFAULT_COALESCE_KINDS,
  {
    maxStreamedBeforeState = DEFAULT_MAX_STREAMED_BEFORE_STATE,
    orderedStateKinds = DEFAULT_ORDERED_STATE_KINDS,
    orderingBarrierKinds = DEFAULT_ORDERING_BARRIER_KINDS,
  } = {},
) {
  return coalesceByKind(queue, coalesceKinds, {
    maxStreamedBeforeState,
    orderedStateKinds,
    orderingBarrierKinds,
    kindFor: (event) => event && event.kind,
    revisionInfoFor: workspaceRevisionInfo,
  });
}

function coalesceByKind(
  queue,
  coalesceKinds = DEFAULT_COALESCE_KINDS,
  {
    maxStreamedBeforeState = DEFAULT_MAX_STREAMED_BEFORE_STATE,
    orderedStateKinds = DEFAULT_ORDERED_STATE_KINDS,
    orderingBarrierKinds = DEFAULT_ORDERING_BARRIER_KINDS,
    kindFor,
    revisionInfoFor,
  } = {},
) {
  if (!queue || queue.length <= 1) {
    return queue ? queue.slice() : [];
  }
  const result = [];
  let segmentStart = 0;
  for (let index = 0; index < queue.length; index += 1) {
    if (!orderingBarrierKinds.has(kindFor(queue[index]))) {
      continue;
    }
    result.push(
      ...coalesceSegmentByKind(
        queue.slice(segmentStart, index),
        coalesceKinds,
        {
          maxStreamedBeforeState,
          orderedStateKinds,
          kindFor,
          revisionInfoFor,
        },
      ),
      queue[index],
    );
    segmentStart = index + 1;
  }
  result.push(
    ...coalesceSegmentByKind(queue.slice(segmentStart), coalesceKinds, {
      maxStreamedBeforeState,
      orderedStateKinds,
      kindFor,
      revisionInfoFor,
    }),
  );
  return result;
}

function coalesceSegmentByKind(
  queue,
  coalesceKinds,
  {
    maxStreamedBeforeState,
    orderedStateKinds,
    kindFor,
    revisionInfoFor,
  },
) {
  if (!queue || queue.length <= 1) {
    return queue ? queue.slice() : [];
  }
  const streamedChunkLimit = normalizeStreamedChunkLimit(maxStreamedBeforeState);
  const lastIndexByKind = new Map();
  for (let i = 0; i < queue.length; i += 1) {
    const kind = kindFor(queue[i]);
    if (kind && coalesceKinds.has(kind)) {
      const currentIndex = lastIndexByKind.get(kind);
      if (
        currentIndex === undefined
        || shouldReplaceCoalescedEntry(
          kind,
          queue[currentIndex],
          queue[i],
          revisionInfoFor,
        )
      ) {
        lastIndexByKind.set(kind, i);
      }
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

function shouldReplaceCoalescedEntry(
  kind,
  current,
  candidate,
  revisionInfoFor,
) {
  if (
    kind !== "workspace_state" ||
    typeof revisionInfoFor !== "function"
  ) {
    return true;
  }
  const currentRevision = revisionInfoFor(current);
  const candidateRevision = revisionInfoFor(candidate);
  if (candidateRevision.status === "invalid") {
    return false;
  }
  if (currentRevision.status === "invalid") {
    return true;
  }
  if (candidateRevision.status === "absent") {
    return currentRevision.status === "absent";
  }
  if (currentRevision.status === "absent") {
    return true;
  }
  return candidateRevision.value >= currentRevision.value;
}

function normalizeStreamedChunkLimit(value) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    return DEFAULT_MAX_STREAMED_BEFORE_STATE;
  }
  return Math.floor(value);
}
