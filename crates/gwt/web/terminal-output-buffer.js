// SPEC-1939 Phase 24 — per-terminal output batching.
//
// WebSocket receive dispatch is already rAF/budget bounded, but xterm still
// pays per `write()` call once terminal_output reaches app.js. This helper
// batches decoded text per window so dense PTY bursts produce one xterm write
// per frame for normal chunk sizes while preserving per-window ordering.

export const DEFAULT_MAX_CHARS_PER_FLUSH = 65536;

function defaultSchedule(callback) {
  if (typeof requestAnimationFrame === "function") {
    return requestAnimationFrame(callback);
  }
  return setTimeout(callback, 0);
}

function normalizeMaxCharsPerFlush(value) {
  if (!Number.isFinite(value) || value <= 0) {
    return DEFAULT_MAX_CHARS_PER_FLUSH;
  }
  return Math.floor(value);
}

function defaultMergeChunks(chunks) {
  return chunks.join("");
}

export function createTerminalOutputBatcher({
  schedule = defaultSchedule,
  write,
  onFlush,
  maxCharsPerFlush = DEFAULT_MAX_CHARS_PER_FLUSH,
  mergeChunks = defaultMergeChunks,
} = {}) {
  if (typeof write !== "function") {
    throw new TypeError("createTerminalOutputBatcher requires a write callback");
  }
  const scheduleImpl = typeof schedule === "function" ? schedule : defaultSchedule;
  const onFlushImpl = typeof onFlush === "function" ? onFlush : null;
  const charsPerFlush = normalizeMaxCharsPerFlush(maxCharsPerFlush);
  const mergeChunksImpl =
    typeof mergeChunks === "function" ? mergeChunks : defaultMergeChunks;
  const pendingByWindow = new Map();

  function entryFor(windowId) {
    let entry = pendingByWindow.get(windowId);
    if (!entry) {
      entry = { chunks: [], scheduled: false };
      pendingByWindow.set(windowId, entry);
    }
    return entry;
  }

  function scheduleWindow(windowId, entry) {
    if (entry.scheduled) {
      return;
    }
    entry.scheduled = true;
    scheduleImpl(() => flushWindow(windowId));
  }

  function enqueue(windowId, text) {
    if (windowId === null || windowId === undefined || text === "") {
      return false;
    }
    const normalized = String(text);
    if (!normalized) {
      return false;
    }
    const entry = entryFor(windowId);
    entry.chunks.push(normalized);
    scheduleWindow(windowId, entry);
    return true;
  }

  function takeFlushChunks(entry) {
    const chunks = [];
    let chars = 0;
    while (entry.chunks.length > 0) {
      const next = entry.chunks[0];
      if (chunks.length > 0 && chars + next.length > charsPerFlush) {
        break;
      }
      chunks.push(entry.chunks.shift());
      chars += next.length;
      if (chars >= charsPerFlush) {
        break;
      }
    }
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

  function flushWindow(windowId) {
    const entry = pendingByWindow.get(windowId);
    if (!entry) {
      return false;
    }
    entry.scheduled = false;
    if (entry.chunks.length === 0) {
      pendingByWindow.delete(windowId);
      return false;
    }
    const chunks = takeFlushChunks(entry);
    if (entry.chunks.length === 0) {
      pendingByWindow.delete(windowId);
    } else {
      scheduleWindow(windowId, entry);
    }
    let text = "";
    try {
      text = mergeChunksImpl(chunks, windowId);
    } catch (error) {
      console.warn("[terminal-output-buffer] merge failed for %s", windowId, error);
      return false;
    }
    if (!text) {
      return false;
    }
    try {
      write(windowId, text, () => notifyFlushed(windowId));
    } catch (error) {
      console.warn("[terminal-output-buffer] write failed for %s", windowId, error);
    }
    return true;
  }

  function flushNow(windowId) {
    let flushed = false;
    while (pendingByWindow.has(windowId)) {
      if (!flushWindow(windowId)) {
        break;
      }
      flushed = true;
    }
    return flushed;
  }

  function clear(windowId) {
    return pendingByWindow.delete(windowId);
  }

  function pendingCount(windowId) {
    if (windowId !== undefined) {
      return pendingByWindow.get(windowId)?.chunks.length || 0;
    }
    let count = 0;
    for (const entry of pendingByWindow.values()) {
      count += entry.chunks.length;
    }
    return count;
  }

  function pendingWindowCount() {
    return pendingByWindow.size;
  }

  return {
    enqueue,
    flushNow,
    clear,
    pendingCount,
    pendingWindowCount,
  };
}
