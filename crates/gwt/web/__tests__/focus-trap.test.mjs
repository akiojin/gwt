import { test } from "node:test";
import assert from "node:assert/strict";
import { createFocusTrap } from "../focus-trap.js";

// linkedom doesn't track focus()/activeElement faithfully, so we use a
// hand-rolled stub document that lets us drive activeElement explicitly
// and verify the trap's behavior via the focus calls it makes.

function makeStubDoc({ focusables = [], container } = {}) {
  const listeners = new Map();
  let activeElement = null;
  const focusCalls = [];

  function makeFocusable(id) {
    const el = {
      id,
      get isContainer() { return false; },
      offsetParent: { id: "parent" }, // pretend visible
      focus(opts) {
        focusCalls.push({ id, opts });
        activeElement = el;
      },
    };
    return el;
  }

  const items = focusables.map(makeFocusable);
  const containerEl = container || {
    id: "container",
    isContainer: true,
    offsetParent: null,
    focus(opts) { focusCalls.push({ id: "container", opts }); activeElement = containerEl; },
    contains(el) {
      if (el === containerEl) return true;
      return items.includes(el);
    },
    querySelectorAll() { return items; },
  };

  return {
    container: containerEl,
    items,
    focusCalls,
    setActive(el) { activeElement = el; },
    doc: {
      get activeElement() { return activeElement; },
      addEventListener(type, fn, opts) {
        if (!listeners.has(type)) listeners.set(type, []);
        listeners.get(type).push({ fn, opts });
      },
      removeEventListener(type, fn) {
        const arr = listeners.get(type) || [];
        listeners.set(type, arr.filter((entry) => entry.fn !== fn));
      },
      dispatch(type, event) {
        const arr = listeners.get(type) || [];
        for (const { fn } of arr) fn(event);
      },
    },
  };
}

function tabEvent(shiftKey = false) {
  let prevented = false;
  return {
    key: "Tab",
    shiftKey,
    preventDefault() { prevented = true; },
    get defaultPrevented() { return prevented; },
  };
}

test("createFocusTrap with no container returns a no-op release", () => {
  const release = createFocusTrap(null);
  assert.equal(typeof release, "function");
  release(); // must not throw
});

test("createFocusTrap with no document returns a no-op release", () => {
  const stub = makeStubDoc({ focusables: ["b1", "b2"] });
  const release = createFocusTrap(stub.container, { document: null });
  assert.equal(typeof release, "function");
  release();
});

test("Tab on the last focusable wraps focus to the first", () => {
  const stub = makeStubDoc({ focusables: ["b1", "b2"] });
  const release = createFocusTrap(stub.container, { document: stub.doc });

  const last = stub.items[1];
  const first = stub.items[0];
  stub.setActive(last);
  const event = tabEvent();
  stub.doc.dispatch("keydown", event);
  assert.equal(event.defaultPrevented, true, "trap must preventDefault on wrap");
  assert.equal(stub.focusCalls.length, 1);
  assert.equal(stub.focusCalls[0].id, first.id, "focus should wrap to first");
  assert.equal(stub.focusCalls[0].opts?.preventScroll, true);
  release();
});

test("Shift+Tab on the first focusable wraps focus to the last", () => {
  const stub = makeStubDoc({ focusables: ["b1", "b2"] });
  const release = createFocusTrap(stub.container, { document: stub.doc });

  const first = stub.items[0];
  const last = stub.items[1];
  stub.setActive(first);
  const event = tabEvent(true);
  stub.doc.dispatch("keydown", event);
  assert.equal(event.defaultPrevented, true);
  assert.equal(stub.focusCalls[0].id, last.id);
  release();
});

test("Tab in the middle of the trap does NOT wrap", () => {
  const stub = makeStubDoc({ focusables: ["b1", "i1", "b2"] });
  const release = createFocusTrap(stub.container, { document: stub.doc });

  const middle = stub.items[1];
  stub.setActive(middle);
  const event = tabEvent();
  stub.doc.dispatch("keydown", event);
  assert.equal(event.defaultPrevented, false);
  assert.equal(stub.focusCalls.length, 0);
  release();
});

test("Tab from outside the trap pulls focus into the first element", () => {
  const stub = makeStubDoc({ focusables: ["b1", "b2"] });
  const outside = { id: "outside", focus() {} };
  // outside is NOT in container.contains; trap should pull focus into first.
  const release = createFocusTrap(stub.container, { document: stub.doc });

  stub.setActive(outside);
  const event = tabEvent();
  stub.doc.dispatch("keydown", event);
  assert.equal(event.defaultPrevented, true);
  assert.equal(stub.focusCalls[0].id, stub.items[0].id);
  release();
});

test("Shift+Tab from outside the trap pulls focus to the last element", () => {
  const stub = makeStubDoc({ focusables: ["b1", "b2", "b3"] });
  const outside = { id: "outside", focus() {} };
  const release = createFocusTrap(stub.container, { document: stub.doc });

  stub.setActive(outside);
  const event = tabEvent(true); // Shift+Tab
  stub.doc.dispatch("keydown", event);
  assert.equal(event.defaultPrevented, true);
  // Shift+Tab from outside wraps to LAST, mirroring how Tab-out-from-end
  // wraps to first. Without this, Shift+Tab from outside would land on
  // first which violates the natural reverse-tab cycle.
  assert.equal(stub.focusCalls[0].id, stub.items[2].id);
  release();
});

test("release() detaches the listener", () => {
  const stub = makeStubDoc({ focusables: ["b1", "b2"] });
  const release = createFocusTrap(stub.container, { document: stub.doc });
  release();
  stub.setActive(stub.items[1]);
  const event = tabEvent();
  stub.doc.dispatch("keydown", event);
  assert.equal(event.defaultPrevented, false);
  assert.equal(stub.focusCalls.length, 0);
});

test("non-Tab keys are ignored", () => {
  const stub = makeStubDoc({ focusables: ["b1", "b2"] });
  const release = createFocusTrap(stub.container, { document: stub.doc });
  stub.setActive(stub.items[1]);
  for (const key of ["Enter", "Escape", "ArrowDown", "ArrowUp", " "]) {
    let prevented = false;
    const event = { key, shiftKey: false, preventDefault() { prevented = true; } };
    stub.doc.dispatch("keydown", event);
    assert.equal(prevented, false, `trap must not intercept ${key}`);
  }
  release();
});

test("focusable selector excludes [disabled] and [aria-disabled=\"true\"]", async () => {
  // Verify the constant in the module matches the documented contract.
  // Use module URL import to read the file content.
  const { readFileSync } = await import("node:fs");
  const { fileURLToPath } = await import("node:url");
  const { dirname, resolve } = await import("node:path");
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(resolve(here, "../focus-trap.js"), "utf8");

  // Each focusable role must exclude the disabled state — the trap should
  // skip programmatically-disabled buttons (e.g. wizard Migrate when
  // hasLocked is true).
  for (const role of ["button", "input", "select", "textarea"]) {
    assert.ok(
      src.includes(`${role}:not([disabled]):not([aria-disabled="true"])`),
      `${role} entry must exclude both [disabled] and [aria-disabled="true"]`,
    );
  }
  assert.ok(
    src.includes('[href]:not([aria-disabled="true"])'),
    "[href] must exclude [aria-disabled=\"true\"]",
  );
  assert.ok(
    src.includes('[tabindex]:not([tabindex="-1"]):not([aria-disabled="true"])'),
    "[tabindex] must exclude tabindex=-1 and aria-disabled=true",
  );
});

test("trap with no focusable children pins focus on the container", () => {
  const stub = makeStubDoc({ focusables: [] });
  const release = createFocusTrap(stub.container, { document: stub.doc });

  stub.setActive(stub.container);
  const event = tabEvent();
  stub.doc.dispatch("keydown", event);
  assert.equal(event.defaultPrevented, true);
  // The container's focus is recorded
  assert.equal(stub.focusCalls.find((c) => c.id === "container") !== undefined, true);
  release();
});
