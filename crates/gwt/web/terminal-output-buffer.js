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
  write,
  onFlush,
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
  const onFlushImpl = typeof onFlush === "function" ? onFlush : null;
  const charsPerFlush = normalizeMaxCharsPerFlush(maxCharsPerFlush);
  const windowsPerFlush = normalizeMaxWindowsPerFlush(maxWindowsPerFlush);
  const totalCharsPerFlush = normalizeMaxTotalCharsPerFlush(maxTotalCharsPerFlush);
  const mergeChunksImpl =
    typeof mergeChunks === "function" ? mergeChunks : defaultMergeChunks;
  const canWriteImpl = typeof canWrite === "function" ? canWrite : () => true;
  const pendingByWindow = new Map();
  const quiescedWindows = new Set();
  let scheduled = false;

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
      if (chunks.length > 0 && chars + next.length > charLimit) {
        break;
      }
      chunks.push(next);
      entry.head += 1;
      chars += next.length;
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

  function flushPending() {
    scheduled = false;
    if (pendingByWindow.size === 0) {
      return false;
    }
    let flushed = false;
    let windowsFlushed = 0;
    let charsFlushed = 0;
    for (const windowId of Array.from(pendingByWindow.keys())) {
      if (windowsFlushed >= windowsPerFlush) {
        break;
      }
      if (windowsFlushed > 0 && charsFlushed >= totalCharsPerFlush) {
        break;
      }
      if (!pendingByWindow.has(windowId)) {
        continue;
      }
      if (quiescedWindows.has(windowId)) {
        continue;
      }
      if (!canWriteImpl(windowId)) {
        quiescedWindows.add(windowId);
        continue;
      }
      const remainingChars = Math.max(1, totalCharsPerFlush - charsFlushed);
      const result = flushWindow(windowId, remainingChars);
      flushed = result.flushed || flushed;
      charsFlushed += result.chars;
      windowsFlushed += 1;
    }
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
    const chunks = takeFlushChunks(entry, maxChars);
    if (pendingChunkCount(entry) === 0) {
      pendingByWindow.delete(windowId);
      quiescedWindows.delete(windowId);
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
      write(windowId, text, () => notifyFlushed(windowId));
    } catch (error) {
      console.warn("[terminal-output-buffer] write failed for %s", windowId, error);
    }
    return { flushed: true, chars: text.length };
  }

  function flushNow(windowId) {
    let flushed = false;
    quiescedWindows.delete(windowId);
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
    return pendingByWindow.delete(windowId);
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
    flushNow,
    clear,
    schedulePending,
    pendingCount,
    pendingWindowCount,
  };
}
