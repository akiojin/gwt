// Shared pull-render contract. Models contain data; views subscribe to selectors.
// Notifications are synchronous so hidden windows never depend on a focus/rAF.
const owned = new WeakSet();

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
  if (Array.isArray(value)) {
    copy = value.map(snapshot);
  } else {
    copy = {};
    const property = { __proto__: null, enumerable: true, configurable: true, writable: true, value: undefined };
    // Preserve own data properties without a second entries array or per-key tuples.
    for (const [key, child] of Object.entries(value)) {
      property.value = snapshot(child);
      Object.defineProperty(copy, key, property);
    }
  }
  owned.add(copy);
  return Object.freeze(copy);
}

export function createUiStateStore(initialState) {
  let current = snapshot(initialState);
  let disposed = false;
  let notifying = false;
  let dirty = false;
  const views = new Set();
  const read = () => current;

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
            view.render(selected);
            view.selected = selected;
          } catch (error) { errors.push(error); }
        }
      }
    } finally { notifying = false; }
    if (errors.length) throw new AggregateError(errors, errors[0].message);
  }

  function subscribe(select, render) {
    if (disposed) throw new Error('UI state store is disposed');
    const view = { select, render, selected: select(read()) };
    views.add(view);
    try { render(view.selected); } catch (error) { views.delete(view); throw error; }
    return () => views.delete(view);
  }

  function dispose() { disposed = true; views.clear(); }
  return { read, update, subscribe, dispose };
}
