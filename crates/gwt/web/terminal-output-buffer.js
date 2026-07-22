// SPEC-1939 Phase 24 — per-terminal output batching.
//
// WebSocket receive dispatch is already rAF/budget bounded, but xterm still
// pays per `write()` call once terminal_output reaches app.js. This helper
// batches decoded text per window so dense PTY bursts produce one xterm write
// per frame for normal chunk sizes while preserving per-window ordering.

export const DEFAULT_MAX_CHARS_PER_FLUSH = 65536;
export const DEFAULT_MAX_WINDOWS_PER_FLUSH = 8;
export const DEFAULT_MAX_TOTAL_CHARS_PER_FLUSH = DEFAULT_MAX_CHARS_PER_FLUSH;

function defaultSchedule(callback) {
  if (typeof requestAnimationFrame === "function") {
    return requestAnimationFrame(callback);
  }
  return setTimeout(callback, 0);
}

function queuedChunkText(chunk) {
  return typeof chunk === "string" ? chunk : chunk.text;
}

function normalizeMaxCharsPerFlush(value) {
  if (!Number.isFinite(value) || value <= 0) {
    return DEFAULT_MAX_CHARS_PER_FLUSH;
  }
  return Math.floor(value);
}

function normalizeMaxWindowsPerFlush(value) {
  if (!Number.isFinite(value) || value <= 0) {
    return DEFAULT_MAX_WINDOWS_PER_FLUSH;
  }
  return Math.floor(value);
}

function normalizeMaxTotalCharsPerFlush(value) {
  if (!Number.isFinite(value) || value <= 0) {
    return DEFAULT_MAX_TOTAL_CHARS_PER_FLUSH;
  }
  return Math.floor(value);
}

function defaultMergeChunks(chunks) {
  return chunks.join("");
}

function pendingChunkCount(entry) {
  return Math.max(0, entry.chunks.length - entry.head);
}

function compactConsumedChunks(entry) {
  if (entry.head <= 0) {
    return;
  }
  if (entry.head >= entry.chunks.length) {
    entry.chunks.length = 0;
    entry.head = 0;
    return;
  }
  if (entry.head >= 1024 && entry.head * 2 >= entry.chunks.length) {
    entry.chunks.splice(0, entry.head);
    entry.head = 0;
  }
}

export function createTerminalOutputBatcher({
  schedule = defaultSchedule,
  schedulePaint = defaultSchedule,
  write,
  onFlush,
  onTrace,
  shouldTrace,
  readTraceEpoch,
  maxCharsPerFlush = DEFAULT_MAX_CHARS_PER_FLUSH,
  maxWindowsPerFlush = DEFAULT_MAX_WINDOWS_PER_FLUSH,
  maxTotalCharsPerFlush = DEFAULT_MAX_TOTAL_CHARS_PER_FLUSH,
  mergeChunks = defaultMergeChunks,
  canWrite,
} = {}) {
  if (typeof write !== "function") {
    throw new TypeError("createTerminalOutputBatcher requires a write callback");
  }
  const scheduleImpl = typeof schedule === "function" ? schedule : defaultSchedule;
  const schedulePaintImpl =
    typeof schedulePaint === "function" ? schedulePaint : defaultSchedule;
  const onFlushImpl = typeof onFlush === "function" ? onFlush : null;
  const traceImpl = typeof onTrace === "function" ? onTrace : null;
  const shouldTraceImpl = typeof shouldTrace === "function" ? shouldTrace : null;
  const readTraceEpochImpl =
    typeof readTraceEpoch === "function" ? readTraceEpoch : null;
  const charsPerFlush = normalizeMaxCharsPerFlush(maxCharsPerFlush);
  const windowsPerFlush = normalizeMaxWindowsPerFlush(maxWindowsPerFlush);
  const totalCharsPerFlush = normalizeMaxTotalCharsPerFlush(maxTotalCharsPerFlush);
  const mergeChunksImpl =
    typeof mergeChunks === "function" ? mergeChunks : defaultMergeChunks;
  const canWriteImpl = typeof canWrite === "function" ? canWrite : () => true;
  const pendingByWindow = new Map();
  const quiescedWindows = new Set();
  let scheduled = false;
  let roundRobinNextWindowId = null;
  let priorityWindowId = null;
  // When priority alone consumes a frame, exclude that exact window from the
  // next normal scan once. Keeping the serviced id (instead of a boolean)
  // preserves one-shot hints and lets a different input target replace them.
  let priorityYieldWindowId = null;

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

  function normalizedTraceMetadata(metadata, windowId) {
    if (!metadata || typeof metadata !== "object") {
      return null;
    }
    try {
      let epoch;
      if (readTraceEpochImpl) {
        epoch = readTraceEpochImpl();
        if (
          epoch === null
          || epoch === undefined
          || metadata.epoch !== epoch
        ) {
          return null;
        }
      }
      if (metadata.sequence === undefined) {
        return null;
      }
      const normalized = {
        sequence: metadata.sequence,
        window_id: windowId,
      };
      if (readTraceEpochImpl) {
        normalized.epoch = epoch;
      }
      return Object.freeze(normalized);
    } catch (_) {
      return null;
    }
  }

  function currentTraceFields(metadata) {
    if (!metadata || !traceActive()) {
      return null;
    }
    try {
      if (readTraceEpochImpl) {
        const epoch = readTraceEpochImpl();
        if (
          epoch === null
          || epoch === undefined
          || metadata.epoch !== epoch
        ) {
          return null;
        }
      }
      return {
        sequence: metadata.sequence,
        window_id: metadata.window_id,
      };
    } catch (_) {
      return null;
    }
  }

  function emitTerminalOutputTrace(kind, metadata) {
    if (!traceImpl || !metadata) {
      return false;
    }
    const fields = currentTraceFields(metadata);
    if (!fields) {
      return false;
    }
    try {
      traceImpl(kind, fields);
    } catch (_) {
      // Diagnostics must never affect the terminal output path.
    }
    return true;
  }

  function emitTerminalOutputTraces(kind, metadataList) {
    if (!metadataList) {
      return false;
    }
    let emitted = false;
    for (const metadata of metadataList) {
      emitted = emitTerminalOutputTrace(kind, metadata) || emitted;
    }
    return emitted;
  }

  function entryFor(windowId) {
    let entry = pendingByWindow.get(windowId);
    if (!entry) {
      entry = { chunks: [], head: 0 };
      pendingByWindow.set(windowId, entry);
    }
    return entry;
  }

  function scheduleFlush() {
    if (scheduled) {
      return;
    }
    scheduled = true;
    scheduleImpl(flushPending);
  }

  function enqueue(windowId, text, metadata) {
    if (windowId === null || windowId === undefined || text === "") {
      return false;
    }
    const normalized = String(text);
    if (!normalized) {
      return false;
    }
    const entry = entryFor(windowId);
    const traceMetadata = traceActive()
      ? normalizedTraceMetadata(metadata, windowId)
      : null;
    entry.chunks.push(
      traceMetadata
        ? Object.freeze({ text: normalized, traceMetadata })
        : normalized,
    );
    if (traceMetadata) {
      emitTerminalOutputTrace("terminal_output_enqueue", traceMetadata);
    }
    if (!quiescedWindows.has(windowId) || hasSchedulablePendingWindow()) {
      scheduleFlush();
    }
    return true;
  }

  function takeFlushChunks(entry, maxChars = charsPerFlush) {
    const chunks = [];
    let chars = 0;
    const charLimit =
      Number.isFinite(maxChars) && maxChars > 0
        ? Math.min(charsPerFlush, Math.floor(maxChars))
        : charsPerFlush;
    while (entry.head < entry.chunks.length) {
      const next = entry.chunks[entry.head];
      const nextText = queuedChunkText(next);
      if (chunks.length > 0 && chars + nextText.length > charLimit) {
        break;
      }
      chunks.push(next);
      entry.head += 1;
      chars += nextText.length;
      if (chars >= charLimit) {
        break;
      }
    }
    compactConsumedChunks(entry);
    return chunks;
  }

  function notifyFlushed(windowId) {
    if (!onFlushImpl) {
      return;
    }
    try {
      onFlushImpl(windowId);
    } catch (_) {
      // Refresh callbacks are best-effort UI maintenance and must not break
      // the terminal output drain.
    }
  }

  function flushEligibleWindow(windowId, maxChars) {
    if (!pendingByWindow.has(windowId) || quiescedWindows.has(windowId)) {
      return { serviced: false, flushed: false, chars: 0 };
    }
    if (!canWriteImpl(windowId)) {
      quiescedWindows.add(windowId);
      return { serviced: false, flushed: false, chars: 0 };
    }
    const result = flushWindow(windowId, maxChars);
    return { serviced: true, ...result };
  }

  function hasSchedulablePeer(windowIds, excludedWindowId) {
    for (const windowId of windowIds) {
      if (
        windowId === excludedWindowId
        || !pendingByWindow.has(windowId)
        || quiescedWindows.has(windowId)
      ) {
        continue;
      }
      if (canWriteImpl(windowId)) {
        return true;
      }
      quiescedWindows.add(windowId);
    }
    return false;
  }

  function flushPending() {
    scheduled = false;
    if (pendingByWindow.size === 0) {
      return false;
    }
    const windowIds = Array.from(pendingByWindow.keys());
    let flushed = false;
    let windowsFlushed = 0;
    let charsFlushed = 0;
    let normalWindowsFlushed = 0;
    let servicedPriorityWindowId = null;
    const pendingYieldWindowId = priorityYieldWindowId;
    priorityYieldWindowId = null;
    const deferredPriorityWindowId =
      pendingYieldWindowId !== null
      && hasSchedulablePeer(windowIds, pendingYieldWindowId)
        ? pendingYieldWindowId
        : null;

    if (
      priorityWindowId !== null
      && priorityWindowId !== deferredPriorityWindowId
      && pendingByWindow.has(priorityWindowId)
    ) {
      const priorityResult = flushEligibleWindow(
        priorityWindowId,
        Math.max(1, totalCharsPerFlush - charsFlushed),
      );
      if (priorityResult.serviced) {
        servicedPriorityWindowId = priorityWindowId;
        priorityWindowId = null;
        flushed = priorityResult.flushed || flushed;
        charsFlushed += priorityResult.chars;
        windowsFlushed += 1;
      }
    }

    const requestedStartIndex = windowIds.indexOf(roundRobinNextWindowId);
    const startIndex = requestedStartIndex >= 0 ? requestedStartIndex : 0;
    for (let offset = 0; offset < windowIds.length; offset += 1) {
      const windowIndex = (startIndex + offset) % windowIds.length;
      const windowId = windowIds[windowIndex];
      if (windowsFlushed >= windowsPerFlush) {
        break;
      }
      if (windowsFlushed > 0 && charsFlushed >= totalCharsPerFlush) {
        break;
      }
      if (
        windowId === servicedPriorityWindowId
        || windowId === deferredPriorityWindowId
      ) {
        continue;
      }
      const remainingChars = Math.max(1, totalCharsPerFlush - charsFlushed);
      const result = flushEligibleWindow(windowId, remainingChars);
      if (!result.serviced) {
        continue;
      }
      flushed = result.flushed || flushed;
      charsFlushed += result.chars;
      windowsFlushed += 1;
      normalWindowsFlushed += 1;
      roundRobinNextWindowId =
        windowIds[(windowIndex + 1) % windowIds.length];
    }
    priorityYieldWindowId =
      servicedPriorityWindowId !== null
      && normalWindowsFlushed === 0
      && hasSchedulablePeer(windowIds, servicedPriorityWindowId)
        ? servicedPriorityWindowId
        : null;
    if (hasSchedulablePendingWindow()) {
      scheduleFlush();
    }
    return flushed;
  }

  function hasSchedulablePendingWindow() {
    for (const windowId of pendingByWindow.keys()) {
      if (quiescedWindows.has(windowId)) {
        continue;
      }
      if (canWriteImpl(windowId)) {
        return true;
      }
    }
    return false;
  }

  function flushWindow(windowId, maxChars = charsPerFlush) {
    const entry = pendingByWindow.get(windowId);
    if (!entry) {
      return { flushed: false, chars: 0 };
    }
    if (pendingChunkCount(entry) === 0) {
      pendingByWindow.delete(windowId);
      quiescedWindows.delete(windowId);
      return { flushed: false, chars: 0 };
    }
    const queuedChunks = takeFlushChunks(entry, maxChars);
    if (pendingChunkCount(entry) === 0) {
      pendingByWindow.delete(windowId);
      quiescedWindows.delete(windowId);
    }
    let chunks = queuedChunks;
    let traceMetadataList = null;
    for (let index = 0; index < queuedChunks.length; index += 1) {
      const chunk = queuedChunks[index];
      if (typeof chunk === "string") {
        if (chunks !== queuedChunks) {
          chunks.push(chunk);
        }
        continue;
      }
      if (chunks === queuedChunks) {
        chunks = queuedChunks.slice(0, index);
      }
      chunks.push(chunk.text);
      if (chunk.traceMetadata) {
        traceMetadataList ??= [];
        traceMetadataList.push(chunk.traceMetadata);
      }
    }
    if (traceMetadataList) {
      emitTerminalOutputTraces(
        "terminal_output_flush_start",
        traceMetadataList,
      );
    }
    let text = "";
    try {
      text = mergeChunksImpl(chunks, windowId);
    } catch (error) {
      console.warn("[terminal-output-buffer] merge failed for %s", windowId, error);
      return { flushed: false, chars: 0 };
    }
    if (!text) {
      return { flushed: false, chars: 0 };
    }
    try {
      let writeCompleted = false;
      write(windowId, text, () => {
        if (writeCompleted) {
          return;
        }
        writeCompleted = true;
        if (traceMetadataList) {
          const tracedCompletion = emitTerminalOutputTraces(
            "terminal_output_write_complete",
            traceMetadataList,
          );
          if (tracedCompletion) {
            try {
              schedulePaintImpl(() => {
                emitTerminalOutputTraces(
                  "terminal_output_next_paint",
                  traceMetadataList,
                );
              });
            } catch (_) {
              // Diagnostics scheduling must never affect viewport maintenance.
            }
          }
        }
        notifyFlushed(windowId);
      });
    } catch (error) {
      console.warn("[terminal-output-buffer] write failed for %s", windowId, error);
    }
    return { flushed: true, chars: text.length };
  }

  function flushNow(windowId) {
    let flushed = false;
    quiescedWindows.delete(windowId);
    if (priorityWindowId === windowId) {
      priorityWindowId = null;
    }
    if (priorityYieldWindowId === windowId) {
      priorityYieldWindowId = null;
    }
    while (pendingByWindow.has(windowId)) {
      const result = flushWindow(windowId);
      if (!result.flushed) {
        break;
      }
      flushed = true;
    }
    return flushed;
  }

  function clear(windowId) {
    quiescedWindows.delete(windowId);
    if (priorityWindowId === windowId) {
      priorityWindowId = null;
    }
    if (priorityYieldWindowId === windowId) {
      priorityYieldWindowId = null;
    }
    return pendingByWindow.delete(windowId);
  }

  function prioritize(windowId) {
    if (windowId === null || windowId === undefined) {
      return false;
    }
    priorityWindowId = windowId;
    if (pendingByWindow.has(windowId) && !quiescedWindows.has(windowId)) {
      scheduleFlush();
    }
    return true;
  }

  function schedulePending(windowId) {
    if (!pendingByWindow.has(windowId)) {
      return false;
    }
    quiescedWindows.delete(windowId);
    scheduleFlush();
    return true;
  }

  function pendingCount(windowId) {
    if (windowId !== undefined) {
      const entry = pendingByWindow.get(windowId);
      return entry ? pendingChunkCount(entry) : 0;
    }
    let count = 0;
    for (const entry of pendingByWindow.values()) {
      count += pendingChunkCount(entry);
    }
    return count;
  }

  function pendingWindowCount() {
    return pendingByWindow.size;
  }

  return {
    enqueue,
    prioritize,
    flushNow,
    clear,
    schedulePending,
    pendingCount,
    pendingWindowCount,
  };
}
