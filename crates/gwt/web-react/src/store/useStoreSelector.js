// Bridge from the Issue #5016 UI state store to React (SPEC #6943 AC-1).
// React surfaces read only through this hook: the store is the single input,
// push payloads never reach a component directly.
import { useRef, useSyncExternalStore } from "react";
import { flushSync } from "react-dom";

function createSelection(store, select, frame) {
  let value = select(store.read());
  let subscribing = false;
  return {
    store,
    select,
    frame,
    getSnapshot: () => value,
    subscribe(onChange) {
      const listen = frame ? store.subscribeFrame : store.subscribe;
      subscribing = true;
      try {
        return listen(select, selected => {
          if (Object.is(selected, value)) return;
          value = selected;
          // Store notifications arrive outside React (socket receive or a
          // scheduled frame). Commit synchronously so the DOM matches the
          // store when the notification returns, like the imperative views.
          if (subscribing) onChange();
          else flushSync(onChange);
        });
      } finally {
        subscribing = false;
      }
    },
  };
}

// `select` must be stable (module scope or memoized). With `frame: true` the
// component only sees values delivered by the store's frame batching.
export function useStoreSelector(store, select, { frame = false } = {}) {
  const selection = useRef(null);
  const current = selection.current;
  if (!current || current.store !== store || current.select !== select || current.frame !== frame) {
    selection.current = createSelection(store, select, frame);
  }
  return useSyncExternalStore(selection.current.subscribe, selection.current.getSnapshot);
}
