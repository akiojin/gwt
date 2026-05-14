const DEFAULT_MAX_ENTRIES = 2000;
const BLOCKED_FIELD_NAMES = new Set([
  "body",
  "chunk",
  "data",
  "data_base64",
  "input",
  "payload",
  "text",
]);

function defaultNow() {
  if (typeof performance !== "undefined" && typeof performance.now === "function") {
    return performance.now();
  }
  return Date.now();
}

function defaultSessionId() {
  const random = Math.random().toString(36).slice(2, 10);
  return `ui-trace-${Date.now().toString(36)}-${random}`;
}

function normalizeKey(key) {
  return String(key).replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);
}

function isSafeScalar(value) {
  return (
    value === null
    || typeof value === "boolean"
    || typeof value === "number"
    || typeof value === "string"
  );
}

function sanitizeFields(fields) {
  const sanitized = {};
  if (!fields || typeof fields !== "object") {
    return sanitized;
  }
  for (const [rawKey, value] of Object.entries(fields)) {
    const key = normalizeKey(rawKey);
    if (BLOCKED_FIELD_NAMES.has(key) || value === undefined) {
      continue;
    }
    if (isSafeScalar(value)) {
      sanitized[key] = value;
    }
  }
  return sanitized;
}

function summarizeTarget(target) {
  if (!target || typeof target !== "object") {
    return "";
  }
  const tag = typeof target.tagName === "string"
    ? target.tagName.toLowerCase()
    : "node";
  const id = typeof target.id === "string" && target.id.length > 0
    ? `#${target.id}`
    : "";
  const className = typeof target.className === "string"
    ? target.className.trim().split(/\s+/).filter(Boolean).join(".")
    : "";
  const classes = className.length > 0 ? `.${className}` : "";
  const dataset = target.dataset && typeof target.dataset === "object"
    ? target.dataset
    : {};
  const preset = typeof dataset.preset === "string" && dataset.preset.length > 0
    ? `[preset=${dataset.preset}]`
    : "";
  return `${tag}${id}${classes}${preset}`;
}

export function createUiTraceProfiler({
  maxEntries = DEFAULT_MAX_ENTRIES,
  now = defaultNow,
  sessionId = defaultSessionId,
} = {}) {
  const maxEntryCount = Math.max(1, Number(maxEntries) || DEFAULT_MAX_ENTRIES);
  const nowImpl = typeof now === "function" ? now : defaultNow;
  const sessionIdImpl = typeof sessionId === "function" ? sessionId : defaultSessionId;

  let active = false;
  let entries = [];
  let droppedEntries = 0;
  let currentSessionId = "";
  let startedAt = 0;
  let longTaskObserver = null;
  let rafProbeHandle = null;
  let lastRafTs = null;

  function pushEntry(entry) {
    if (entries.length >= maxEntryCount) {
      entries.shift();
      droppedEntries += 1;
    }
    entries.push(entry);
  }

  function record(kind, fields = {}) {
    if (!active || typeof kind !== "string" || kind.length === 0) {
      return;
    }
    pushEntry({
      ts: nowImpl(),
      kind,
      ...sanitizeFields(fields),
    });
  }

  function startLongTaskProbe() {
    if (
      typeof PerformanceObserver === "undefined"
      || typeof PerformanceObserver !== "function"
    ) {
      return;
    }
    try {
      longTaskObserver = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          record("long_task", {
            name: entry.name,
            duration_ms: entry.duration,
            start_time: entry.startTime,
          });
        }
      });
      longTaskObserver.observe({ entryTypes: ["longtask"] });
    } catch (_) {
      longTaskObserver = null;
    }
  }

  function stopLongTaskProbe() {
    if (!longTaskObserver) {
      return;
    }
    try {
      longTaskObserver.disconnect();
    } catch (_) {
      // Ignore observer teardown failures in diagnostics code.
    }
    longTaskObserver = null;
  }

  function startRafGapProbe() {
    if (typeof requestAnimationFrame !== "function") {
      return;
    }
    lastRafTs = null;
    const tick = (ts) => {
      if (!active) {
        rafProbeHandle = null;
        return;
      }
      if (lastRafTs !== null && ts - lastRafTs > 50) {
        record("raf_gap", {
          gap_ms: ts - lastRafTs,
        });
      }
      lastRafTs = ts;
      rafProbeHandle = requestAnimationFrame(tick);
    };
    rafProbeHandle = requestAnimationFrame(tick);
  }

  function stopRafGapProbe() {
    if (rafProbeHandle === null || typeof cancelAnimationFrame !== "function") {
      rafProbeHandle = null;
      return;
    }
    cancelAnimationFrame(rafProbeHandle);
    rafProbeHandle = null;
  }

  function start() {
    if (active) {
      stopLongTaskProbe();
      stopRafGapProbe();
    }
    active = true;
    entries = [];
    droppedEntries = 0;
    currentSessionId = String(sessionIdImpl());
    startedAt = nowImpl();
    record("trace_start", { session_id: currentSessionId });
    startLongTaskProbe();
    startRafGapProbe();
    return { session_id: currentSessionId, started_at: startedAt };
  }

  function stop() {
    if (!active) {
      return null;
    }
    const stoppedAt = nowImpl();
    stopLongTaskProbe();
    stopRafGapProbe();
    active = false;
    return {
      session_id: currentSessionId,
      started_at: startedAt,
      stopped_at: stoppedAt,
      dropped_entries: droppedEntries,
      entries: entries.slice(),
    };
  }

  function recordPointer(kind, event, fields = {}) {
    if (!active) {
      return;
    }
    if (!event || typeof event !== "object") {
      record(kind, fields);
      return;
    }
    record(kind, {
      ...fields,
      pointer_id: event.pointerId,
      button: event.button,
      buttons: event.buttons,
      client_x: event.clientX,
      client_y: event.clientY,
      target: summarizeTarget(event.target),
    });
  }

  function measure(kind, fields, callback) {
    if (typeof callback !== "function") {
      throw new TypeError("ui trace measure requires a callback");
    }
    if (!active) {
      return callback();
    }
    const startTs = nowImpl();
    try {
      const result = callback();
      record(kind, {
        ...fields,
        duration_ms: nowImpl() - startTs,
      });
      return result;
    } catch (error) {
      record(kind, {
        ...fields,
        duration_ms: nowImpl() - startTs,
        threw: true,
        error_name: error && error.name,
      });
      throw error;
    }
  }

  function isActive() {
    return active;
  }

  return {
    isActive,
    measure,
    record,
    recordPointer,
    start,
    stop,
  };
}
