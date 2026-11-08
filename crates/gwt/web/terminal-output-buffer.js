// SPEC-1939 Phase 24 — per-terminal output batching.
//
// WebSocket receive dispatch is already rAF/budget bounded, but xterm still
// pays per `write()` call once terminal_output reaches app.js. This helper
// batches decoded text per window so dense PTY bursts produce one xterm write
// per frame for normal chunk sizes while preserving per-window ordering.

export const DEFAULT_MAX_CHARS_PER_FLUSH = 65536;
export const DEFAULT_MAX_WINDOWS_PER_FLUSH = 8;
export const DEFAULT_MAX_TOTAL_CHARS_PER_FLUSH = DEFAULT_MAX_CHARS_PER_FLUSH;
const DEFAULT_MAX_PENDING_BYTES_PER_WINDOW = 512 * 1024;
const DEFAULT_MAX_PENDING_BYTES_PER_PROJECT = 4 * 1024 * 1024;

// A document can become hidden after work is queued. Promote pending work to
// a yielding MessageChannel task before frames suspend and timers are clamped.
// Supplying a window keeps visible work coalesced on that window's next frame.
export function scheduleTerminalTask(callback, frameWindow = null) {
  const owner = frameWindow ?? globalThis;
  const visibility = owner.document;
  let finished = false;
  let timer = null;
  let frame = null;
  let task = null;
  const cancel = () => {
    finished = true;
    visibility?.removeEventListener?.("visibilitychange", scheduleHidden);
    owner.clearTimeout(timer);
    if (frame !== null) owner.cancelAnimationFrame?.(frame);
    if (task) {
      task.port1.onmessage = null;
      task.port1.close();
      task.port2.close();
      task = null;
    }
  };
  const run = () => {
    if (finished) return;
    cancel();
    callback();
  };
  const scheduleHidden = () => {
    if (finished || task || !visibility?.hidden || typeof owner.MessageChannel !== "function") return;
    task = new owner.MessageChannel();
    task.port1.onmessage = run;
    task.port2.postMessage(null);
  };
  scheduleHidden();
  if (!task) {
    visibility?.addEventListener?.("visibilitychange", scheduleHidden);
    const hasFrames = frameWindow && !visibility?.hidden && typeof owner.requestAnimationFrame === "function";
    timer = owner.setTimeout(run, hasFrames ? 100 : 0);
    if (hasFrames) frame = owner.requestAnimationFrame(run);
  }
  return cancel;
}

function defaultSchedule(callback) {
  return scheduleTerminalTask(callback, globalThis);
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
  onError,
  maxCharsPerFlush = DEFAULT_MAX_CHARS_PER_FLUSH,
  maxWindowsPerFlush = DEFAULT_MAX_WINDOWS_PER_FLUSH,
  maxTotalCharsPerFlush = DEFAULT_MAX_TOTAL_CHARS_PER_FLUSH,
  mergeChunks = defaultMergeChunks,
  canWrite,
  measureBytes = text => text.length,
  projectForWindow = () => null,
  maxPendingBytesPerWindow = DEFAULT_MAX_PENDING_BYTES_PER_WINDOW,
  maxPendingBytesPerProject = DEFAULT_MAX_PENDING_BYTES_PER_PROJECT,
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
  const failedWindows = new Set();
  const snapshotWindows = new Set();
  const retainedByWindow = new Map();
  const retainedByProject = new Map();
  let scheduled = false;

  // Charges follow the allocation, including xterm's asynchronous write queue.
  // Disposed windows invalidate their account so late callbacks cannot release
  // bytes retained by a new runtime with the same id.
  function release(account, bytes) {
    if (!account?.active) return;
    account.bytes -= bytes;
    account.project.bytes -= bytes;
    if (account.bytes === 0 && retainedByWindow.get(account.id) === account) retainedByWindow.delete(account.id);
    if (account.project.bytes === 0 && retainedByProject.get(account.project.key) === account.project) retainedByProject.delete(account.project.key);
  }

  function discardPending(windowId) {
    const entry = pendingByWindow.get(windowId);
    if (entry) {
      release(entry.account, entry.bytes);
      entry.bytes = 0;
      entry.chunks.length = 0;
      entry.head = 0;
    }
    quiescedWindows.delete(windowId);
    return pendingByWindow.delete(windowId);
  }

  function writable(windowId) {
    return !failedWindows.has(windowId) && !snapshotWindows.has(windowId) && canWriteImpl(windowId);
  }

  function entryFor(windowId, account) {
    let entry = pendingByWindow.get(windowId);
    if (!entry) {
      entry = { chunks: [], head: 0, bytes: 0, account };
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
    if (windowId === null || windowId === undefined || text === ""
      || (failedWindows.has(windowId) && !snapshotWindows.has(windowId))) {
      return false;
    }
    const normalized = String(text);
    if (!normalized) {
      return false;
    }
    const bytes = measureBytes(normalized);
    let account = retainedByWindow.get(windowId);
    const projectKey = account ? account.project.key : projectForWindow(windowId);
    const project = account?.project || retainedByProject.get(projectKey) || { key: projectKey, bytes: 0 };
    if (!Number.isSafeInteger(bytes) || bytes <= 0
      || (account?.bytes || 0) + bytes > maxPendingBytesPerWindow
      || project.bytes + bytes > maxPendingBytesPerProject) {
      failWindow(windowId, new RangeError("Terminal RAW retention limit exceeded"), projectKey);
      return false;
    }
    if (!account) {
      account = { id: windowId, project, bytes: 0, active: true };
      retainedByWindow.set(windowId, account);
      retainedByProject.set(projectKey, project);
    }
    account.bytes += bytes;
    project.bytes += bytes;
    const entry = entryFor(windowId, account);
    entry.bytes += bytes;
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
      entry.chunks[entry.head] = null;
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

  function failWindow(windowId, error, projectKey = retainedByWindow.has(windowId)
    ? retainedByWindow.get(windowId).project.key : projectForWindow(windowId)) {
    // A decoder/write may have consumed only part of an ANSI stream. Discard
    // its suffix and reject new output until a snapshot applies successfully.
    discardPending(windowId);
    snapshotWindows.delete(windowId);
    failedWindows.add(windowId);
    try {
      onError?.(error, windowId, projectKey);
    } catch (recoveryError) {
      console.warn("[terminal-output-buffer] recovery failed for %s", windowId, recoveryError);
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
      if (!writable(windowId)) {
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
      if (writable(windowId)) {
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
    const bytes = chunks.reduce((total, chunk) => total + measureBytes(chunk), 0);
    entry.bytes -= bytes;
    const account = entry.account;
    let released = false;
    const releaseWrite = () => {
      if (released) return;
      released = true;
      release(account, bytes);
    };
    if (pendingChunkCount(entry) === 0) {
      pendingByWindow.delete(windowId);
      quiescedWindows.delete(windowId);
    }
    let text = "";
    try {
      text = mergeChunksImpl(chunks, windowId);
    } catch (error) {
      releaseWrite();
      console.warn("[terminal-output-buffer] merge failed for %s", windowId, error);
      failWindow(windowId, error, account.project.key);
      return { flushed: false, chars: 0 };
    }
    if (!text) {
      releaseWrite();
      return { flushed: false, chars: 0 };
    }
    try {
      write(windowId, text, () => {
        if (released) return;
        releaseWrite();
        if (account.active) notifyFlushed(windowId);
      });
    } catch (error) {
      releaseWrite();
      console.warn("[terminal-output-buffer] write failed for %s", windowId, error);
      failWindow(windowId, error, account.project.key);
      return { flushed: false, chars: 0 };
    }
    return { flushed: true, chars: text.length };
  }

  function flushNow(windowId) {
    if (!writable(windowId)) return false;
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

  function clear(windowId, { preserveFailure = false } = {}) {
    if (!preserveFailure) failedWindows.delete(windowId);
    snapshotWindows.delete(windowId);
    const removed = discardPending(windowId);
    if (!preserveFailure) {
      const account = retainedByWindow.get(windowId);
      if (account) {
        release(account, account.bytes);
        account.active = false;
      }
    }
    return removed;
  }

  function beginSnapshot(windowId) {
    discardPending(windowId);
    snapshotWindows.add(windowId);
  }

  function prune(retainedWindowIds) {
    const removed = [];
    for (const id of new Set([...retainedByWindow.keys(), ...failedWindows, ...snapshotWindows])) {
      if (retainedWindowIds.has(id)) continue;
      clear(id);
      removed.push(id);
    }
    return removed;
  }

  function finishSnapshot(windowId) {
    // A suffix overflow/failure can invalidate this write while xterm is
    // still parsing its snapshot. That old callback must not bridge the gap.
    if (!snapshotWindows.delete(windowId)) return;
    failedWindows.delete(windowId);
    schedulePending(windowId);
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
    beginSnapshot,
    finishSnapshot,
    prune,
    invalidate: failWindow,
    schedulePending,
    pendingCount,
    pendingWindowCount,
  };
}
