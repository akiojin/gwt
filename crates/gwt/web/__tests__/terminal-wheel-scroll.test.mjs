// SPEC-2008 Phase 28 / FR-061 - Windows WebView2 terminal wheel events
// must scroll xterm scrollback directly. xterm mouse tracking can consume
// wheel events before the normal viewport path moves, while dragging the
// scrollbar still works. The controller is behavior-tested here so app.js
// only wires a verified primitive into each terminal runtime.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  APPLICATION_SCROLL_PAGE_DOWN,
  APPLICATION_SCROLL_PAGE_UP,
  applicationScrollInputForWheel,
  createTerminalWheelScrollController,
  hasNormalScrollback,
  isTerminalMouseTrackingActive,
  wheelDeltaToScrollLines,
} from "../terminal-wheel-scroll.js";

test("Windows terminal plain wheel scrolls xterm scrollback and stops bubbling", () => {
  const fixture = mountFixture({ isWindowsHost: () => true });
  const event = wheelEvent(fixture.window, { deltaY: 96 });

  fixture.terminalRoot.dispatchEvent(event);

  assert.equal(event.defaultPrevented, true);
  assert.deepEqual(fixture.scrollCalls, [3]);
  assert.equal(fixture.bubbledWheelCount, 0);
  fixture.dispose();
});

test("Windows terminal mouse tracking lets xterm receive plain wheel events", () => {
  const fixture = mountFixture({
    isWindowsHost: () => true,
    terminalOptions: { modes: { mouseTrackingMode: "any" } },
  });
  const event = wheelEvent(fixture.window, { deltaY: 96 });

  fixture.terminalRoot.dispatchEvent(event);

  assert.equal(event.defaultPrevented, false);
  assert.deepEqual(fixture.scrollCalls, []);
  assert.equal(fixture.bubbledWheelCount, 1);
  fixture.dispose();
});

test("Ctrl and Meta wheel bypass terminal override for canvas zoom routing", () => {
  for (const modifier of ["ctrlKey", "metaKey"]) {
    const fixture = mountFixture({ isWindowsHost: () => true });
    const event = wheelEvent(fixture.window, { deltaY: 96, [modifier]: true });

    fixture.terminalRoot.dispatchEvent(event);

    assert.equal(event.defaultPrevented, false, `${modifier} wheel must not be captured`);
    assert.deepEqual(fixture.scrollCalls, []);
    assert.equal(fixture.bubbledWheelCount, 1);
    fixture.dispose();
  }
});

test("non-Windows terminal wheel behavior is unchanged", () => {
  const fixture = mountFixture({ isWindowsHost: () => false });
  const event = wheelEvent(fixture.window, { deltaY: 96 });

  fixture.terminalRoot.dispatchEvent(event);

  assert.equal(event.defaultPrevented, false);
  assert.deepEqual(fixture.scrollCalls, []);
  assert.equal(fixture.bubbledWheelCount, 1);
  fixture.dispose();
});

test("agent fullscreen terminal wheel sends PageUp and PageDown when xterm has no scrollback", () => {
  const fixture = mountFixture({
    isWindowsHost: () => false,
    applicationScrollFallback: true,
    terminalOptions: { buffer: { active: { baseY: 0 } } },
  });

  fixture.terminalRoot.dispatchEvent(wheelEvent(fixture.window, { deltaY: -96 }));
  fixture.terminalRoot.dispatchEvent(wheelEvent(fixture.window, { deltaY: 96 }));

  assert.deepEqual(fixture.inputCalls, [
    APPLICATION_SCROLL_PAGE_UP,
    APPLICATION_SCROLL_PAGE_DOWN,
  ]);
  assert.deepEqual(fixture.scrollCalls, []);
  assert.equal(fixture.bubbledWheelCount, 0);
  fixture.dispose();
});

test("agent fullscreen wheel fallback does not steal normal xterm scrollback", () => {
  const fixture = mountFixture({
    isWindowsHost: () => false,
    applicationScrollFallback: true,
    terminalOptions: { buffer: { active: { baseY: 8 } } },
  });
  const event = wheelEvent(fixture.window, { deltaY: -96 });

  fixture.terminalRoot.dispatchEvent(event);

  assert.equal(event.defaultPrevented, false);
  assert.deepEqual(fixture.inputCalls, []);
  assert.equal(fixture.bubbledWheelCount, 1);
  fixture.dispose();
});

test("dispose removes the terminal wheel listener", () => {
  const fixture = mountFixture({ isWindowsHost: () => true });
  fixture.dispose();
  const event = wheelEvent(fixture.window, { deltaY: 96 });

  fixture.terminalRoot.dispatchEvent(event);

  assert.equal(event.defaultPrevented, false);
  assert.deepEqual(fixture.scrollCalls, []);
  assert.equal(fixture.bubbledWheelCount, 1);
});

test("wheelDeltaToScrollLines handles pixel, line, and page units", () => {
  assert.equal(wheelDeltaToScrollLines({ deltaY: 1, deltaMode: 0 }), 1);
  assert.equal(wheelDeltaToScrollLines({ deltaY: 96, deltaMode: 0 }), 3);
  assert.equal(wheelDeltaToScrollLines({ deltaY: -96, deltaMode: 0 }), -3);
  assert.equal(wheelDeltaToScrollLines({ deltaY: 2, deltaMode: 1 }), 2);
  assert.equal(wheelDeltaToScrollLines({ deltaY: -1, deltaMode: 2 }), -24);
  assert.equal(wheelDeltaToScrollLines({ deltaY: 0, deltaMode: 0 }), 0);
});

test("isTerminalMouseTrackingActive reads xterm public mouse tracking mode", () => {
  assert.equal(isTerminalMouseTrackingActive({ modes: { mouseTrackingMode: "none" } }), false);
  assert.equal(isTerminalMouseTrackingActive({ modes: { mouseTrackingMode: "vt200" } }), true);
  assert.equal(isTerminalMouseTrackingActive({}), false);
});

test("applicationScrollInputForWheel only emits fallback input for plain wheel without normal scrollback", () => {
  assert.equal(
    applicationScrollInputForWheel(
      wheelEvent(null, { deltaY: -1 }),
      { terminal: { buffer: { active: { baseY: 0 } } }, enabled: true },
    ),
    APPLICATION_SCROLL_PAGE_UP,
  );
  assert.equal(
    applicationScrollInputForWheel(
      wheelEvent(null, { deltaY: 1 }),
      { terminal: { buffer: { active: { baseY: 0 } } }, enabled: true },
    ),
    APPLICATION_SCROLL_PAGE_DOWN,
  );
  assert.equal(
    applicationScrollInputForWheel(
      wheelEvent(null, { deltaY: -1, metaKey: true }),
      { terminal: { buffer: { active: { baseY: 0 } } }, enabled: true },
    ),
    null,
  );
  assert.equal(
    applicationScrollInputForWheel(
      wheelEvent(null, { deltaY: -1 }),
      { terminal: { buffer: { active: { baseY: 5 } } }, enabled: true },
    ),
    null,
  );
});

test("hasNormalScrollback reads xterm active buffer baseY", () => {
  assert.equal(hasNormalScrollback({ buffer: { active: { baseY: 0 } } }), false);
  assert.equal(hasNormalScrollback({ buffer: { active: { baseY: 1 } } }), true);
  assert.equal(hasNormalScrollback({}), false);
});

test("app.js wires application wheel fallback only through agent window presets", () => {
  const appSource = readFileSync(new URL("../app.js", import.meta.url), "utf8");

  assert.match(
    appSource,
    /isApplicationScrollFallbackEnabled:\s*\(\)\s*=>\s*isAgentWindowPreset\(workspaceWindowById\(windowId\)\?\.preset\)/,
    "agent fullscreen wheel fallback must be limited to agent presets",
  );
  assert.match(
    appSource,
    /sendTerminalInput:\s*\(data\)\s*=>\s*\{[\s\S]*?send\(\{\s*kind:\s*"terminal_input",\s*id:\s*windowId,\s*data\s*\}\);[\s\S]*?\}/,
    "agent fullscreen wheel fallback must send PageUp/PageDown through terminal_input",
  );
});

function mountFixture({
  isWindowsHost,
  terminalOptions = {},
  applicationScrollFallback = false,
} = {}) {
  const terminalRoot = new FakeTerminalRoot();
  const scrollCalls = [];
  const inputCalls = [];
  let bubbledWheelCount = 0;
  terminalRoot.setBubbleHandler(() => {
    bubbledWheelCount += 1;
  });

  const controller = createTerminalWheelScrollController({
    terminalRoot,
    terminal: {
      scrollLines: (lines) => scrollCalls.push(lines),
      ...terminalOptions,
    },
    isApplicationScrollFallbackEnabled: () => applicationScrollFallback,
    sendTerminalInput: (data) => inputCalls.push(data),
    window: { navigator: { platform: "Win32" } },
    isWindowsHost,
  });

  return {
    terminalRoot,
    scrollCalls,
    inputCalls,
    get bubbledWheelCount() {
      return bubbledWheelCount;
    },
    dispose() {
      controller.dispose();
    },
  };
}

class FakeTerminalRoot {
  constructor() {
    this.listeners = new Map();
    this.bubbleHandler = () => {};
  }

  addEventListener(type, handler) {
    this.listeners.set(type, handler);
  }

  removeEventListener(type, handler) {
    if (this.listeners.get(type) === handler) {
      this.listeners.delete(type);
    }
  }

  setBubbleHandler(handler) {
    this.bubbleHandler = handler;
  }

  dispatchEvent(event) {
    const handler = this.listeners.get(event.type);
    if (handler) {
      handler(event);
    }
    if (!event.propagationStopped) {
      this.bubbleHandler(event);
    }
    return !event.defaultPrevented;
  }
}

function wheelEvent(_window, options = {}) {
  return {
    type: "wheel",
    deltaY: 0,
    deltaMode: 0,
    ctrlKey: false,
    metaKey: false,
    ...options,
    defaultPrevented: false,
    propagationStopped: false,
    preventDefault() {
      this.defaultPrevented = true;
    },
    stopPropagation() {
      this.propagationStopped = true;
    },
  };
}
