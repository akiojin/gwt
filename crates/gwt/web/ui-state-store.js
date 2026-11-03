// Shared pull-render contract. Models contain data; views subscribe to selectors.
// Stream notifications stay synchronous; views can opt into frame batching.
const owned = new WeakSet();

function defaultScheduleFrame(callback) {
  let finished = false;
  let frame = null;
  const cancel = () => {
    finished = true;
    clearTimeout(timer);
    if (frame !== null && typeof cancelAnimationFrame === 'function') cancelAnimationFrame(frame);
  };
  const run = () => {
    if (finished) return;
    cancel();
    callback();
  };
  // Background WebViews may suspend rAF. Focus is never a render trigger.
  const timer = setTimeout(run, 100);
  if (typeof requestAnimationFrame === 'function') frame = requestAnimationFrame(run);
  return cancel;
}

function snapshot(value) {
  if (value === null || typeof value !== 'object') {
    if (typeof value === 'function' || typeof value === 'symbol') {
      throw new TypeError('UI state must contain data, not renderer functions');
    }
    return value;
  }
  if (owned.has(value)) return value;
  if (!Array.isArray(value) && Object.getPrototypeOf(value) !== Object.prototype && Object.getPrototypeOf(value) !== null) {
    throw new TypeError('UI state must contain plain objects and arrays');
  }
  const copy = Array.isArray(value) ? value.map(snapshot)
    : Object.fromEntries(Object.entries(value).map(([key, child]) => [key, snapshot(child)]));
  owned.add(copy);
  return Object.freeze(copy);
}

export function createUiStateStore(initialState, { scheduleFrame = defaultScheduleFrame } = {}) {
  let current = snapshot(initialState);
  let disposed = false;
  let notifying = false;
  let dirty = false;
  const views = new Set();
  const read = () => current;
  let pendingFrame = null;

  function cancelFrame() {
    const pending = pendingFrame;
    pendingFrame = null;
    pending?.cancel();
  }

  function scheduleViews() {
    if (pendingFrame) return;
    const pending = { cancel() {} };
    pendingFrame = pending;
    try {
      pending.cancel = scheduleFrame(() => {
        if (disposed || pendingFrame !== pending) return;
        pendingFrame = null;
        const state = read();
        const errors = [];
        for (const view of [...views]) {
          if (!view.frame || !views.has(view)) continue;
          try {
            const selected = view.select(state);
            if (Object.is(selected, view.selected)) continue;
            view.render(selected);
            view.selected = selected;
          } catch (error) { errors.push(error); }
        }
        // A render can restore the previous selection before its rendered
        // value is committed above. That update still needs a follow-up frame.
        if (!disposed && !Object.is(current, state) && [...views].some(view => view.frame)) scheduleViews();
        if (errors.length) throw new AggregateError(errors, errors[0].message);
      });
    } catch (error) {
      if (pendingFrame === pending) pendingFrame = null;
      throw error;
    }
  }

  function update(reduce) {
    if (disposed) throw new Error('UI state store is disposed');
    const next = snapshot(reduce(current));
    if (next === current) return;
    current = next;
    dirty = true;
    if (notifying) return;
    notifying = true;
    const errors = [];
    try {
      while (dirty) {
        dirty = false;
        for (const view of [...views]) {
          if (!views.has(view)) continue;
          try {
            const selected = view.select(read());
            if (Object.is(selected, view.selected)) continue;
            if (view.frame) { scheduleViews(); continue; }
            view.render(selected);
            view.selected = selected;
          } catch (error) { errors.push(error); }
        }
      }
    } finally { notifying = false; }
    if (errors.length) throw new AggregateError(errors, errors[0].message);
  }

  function subscribe(select, render, frame = false) {
    if (disposed) throw new Error('UI state store is disposed');
    const view = { select, render, frame, selected: select(read()) };
    views.add(view);
    try { render(view.selected); } catch (error) { views.delete(view); throw error; }
    return () => {
      views.delete(view);
      if (![...views].some(view => view.frame)) cancelFrame();
    };
  }

  function dispose() { disposed = true; views.clear(); cancelFrame(); }
  const subscribeFrame = (select, render) => subscribe(select, render, true);
  return { read, update, subscribe, subscribeFrame, dispose };
}
