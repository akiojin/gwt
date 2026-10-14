// Shared pull-render contract. Models contain data; views subscribe to selectors.
// Model notifications stay synchronous; DOM views may coalesce at a frame.
const owned = new WeakSet();

function scheduleViewFrame(callback) {
  let active = true, frame;
  const cancel = () => {
    active = false;
    clearTimeout(timer);
    if (frame !== undefined) globalThis.cancelAnimationFrame?.(frame);
  };
  const flush = () => { if (active) { cancel(); callback(); } };
  const timer = setTimeout(flush, 100);
  if (globalThis.requestAnimationFrame) frame = requestAnimationFrame(flush);
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
  let copy;
  if (Array.isArray(value)) copy = value.map(snapshot);
  else {
    copy = {};
    for (const key of Object.keys(value)) {
      const child = snapshot(value[key]);
      if (key === '__proto__') Object.defineProperty(copy, key, { value: child, enumerable: true });
      else copy[key] = child;
    }
  }
  owned.add(copy);
  return Object.freeze(copy);
}

export function createUiStateStore(initialState, { scheduleView = scheduleViewFrame } = {}) {
  let current = snapshot(initialState);
  let disposed = false;
  let notifying = false;
  let dirty = false;
  const views = new Set();
  const read = () => current;

  function queueView(view) {
    const pending = { cancel: null };
    view.pending = pending;
    try {
      pending.cancel = scheduleView(() => {
        if (disposed || !views.has(view) || view.pending !== pending) return;
        view.pending = null;
        const selected = view.select(read());
        if (Object.is(selected, view.selected)) return;
        view.render(selected);
        view.selected = selected;
      });
    } catch (error) {
      if (view.pending === pending) view.pending = null;
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
            if (view.batched) {
              if (!view.pending) queueView(view);
            } else {
              view.render(selected);
              view.selected = selected;
            }
          } catch (error) { errors.push(error); }
        }
      }
    } finally { notifying = false; }
    if (errors.length) throw new AggregateError(errors, errors[0].message);
  }

  function removeView(view) {
    view.pending?.cancel?.();
    view.pending = null;
    return views.delete(view);
  }

  function subscribe(select, render, batched = false) {
    if (disposed) throw new Error('UI state store is disposed');
    const view = { select, render, selected: select(read()), batched, pending: null };
    views.add(view);
    try { render(view.selected); } catch (error) { removeView(view); throw error; }
    return () => removeView(view);
  }

  function subscribeView(select, render) { return subscribe(select, render, true); }
  function dispose() {
    disposed = true;
    for (const view of [...views]) removeView(view);
  }
  return { read, update, subscribe, subscribeView, dispose };
}
