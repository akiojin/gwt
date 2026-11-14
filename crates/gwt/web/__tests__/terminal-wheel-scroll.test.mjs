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

test("retained history takes wheel ownership while the application tracks the mouse", () => {
  for (const windows of [true, false]) {
    const fixture = mountFixture({
      isWindowsHost: () => windows,
      applicationScrollFallback: true,
      terminalOptions: {
        buffer: { active: { type: "normal", baseY: 180 } },
        modes: { mouseTrackingMode: "any" },
      },
    });
    const event = wheelEvent(fixture.window, { deltaY: -96 });

    fixture.terminalRoot.dispatchEvent(event);

    assert.equal(event.defaultPrevented, true);
    assert.deepEqual(fixture.scrollCalls, [-3]);
    assert.deepEqual(fixture.inputCalls, [], "history must not send PageUp or mouse input");
    assert.equal(fixture.bubbledWheelCount, 0);
    fixture.dispose();
  }
});

test("Ctrl and Meta wheel bypass terminal override for canvas zoom routing", () => {
  for (const modifier of ["ctrlKey", "metaKey"]) {
    const fixture = mountFixture({
      isWindowsHost: () => true, isRuntimeStopped: () => true,
      terminalOptions: {
        buffer: { active: { baseY: 180 } },
        modes: { mouseTrackingMode: "any" },
      },
    });
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
  assert.deepEqual(fixture.inputOptions, [{ focus: true }, { focus: true }]);
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

// Issue #4909 AC-6: Codex enables mouse tracking, so xterm already forwards
// the wheel as mouse events; a synthesized PageUp opens its transcript
// overlay and sends later keystrokes to Find.
test("agent fullscreen wheel fallback stays silent while the application tracks the mouse", () => {
  for (const mouseTrackingMode of ["any", "vt200"]) {
    const fixture = mountFixture({
      isWindowsHost: () => false,
      applicationScrollFallback: true,
      terminalOptions: {
        buffer: { active: { baseY: 0 } },
        modes: { mouseTrackingMode },
      },
    });
    const event = wheelEvent(fixture.window, { deltaY: -96 });

    fixture.terminalRoot.dispatchEvent(event);

    assert.equal(event.defaultPrevented, false, `${mouseTrackingMode} wheel must reach xterm`);
    assert.deepEqual(fixture.inputCalls, [], `${mouseTrackingMode} must not send PageUp`);
    assert.deepEqual(fixture.scrollCalls, []);
    assert.equal(fixture.bubbledWheelCount, 1);
    fixture.dispose();
  }
});

test("agent fullscreen wheel fallback still sends PageUp when mouse tracking is off", () => {
  const fixture = mountFixture({
    isWindowsHost: () => false,
    applicationScrollFallback: true,
    terminalOptions: {
      buffer: { active: { baseY: 0 } },
      modes: { mouseTrackingMode: "none" },
    },
  });
  const event = wheelEvent(fixture.window, { deltaY: -96 });

  fixture.terminalRoot.dispatchEvent(event);

  assert.equal(event.defaultPrevented, true);
  assert.deepEqual(fixture.inputCalls, [APPLICATION_SCROLL_PAGE_UP]);
  assert.equal(fixture.bubbledWheelCount, 0);
  fixture.dispose();
});

test("stopped fullscreen wheel scrolls locally without input or mouse reports", () => {
  for (const mouseTrackingMode of ["none", "any"]) {
    const fixture = mountFixture({
      isWindowsHost: () => false,
      applicationScrollFallback: true,
      isRuntimeStopped: () => true,
      terminalOptions: { modes: { mouseTrackingMode } },
    });
    const event = wheelEvent(fixture.window, { deltaY: -96 });
    fixture.terminalRoot.dispatchEvent(event);
    assert.deepEqual(fixture.inputCalls, []);
    assert.deepEqual(fixture.scrollCalls, [-3]);
    assert.equal(event.defaultPrevented, true);
    assert.equal(fixture.bubbledWheelCount, 0);
    fixture.dispose();
  }
});

test("stopped touch scrolling follows the local wheel path and observes resume", () => {
  let stopped = true;
  const fixture = mountFixture({
    isWindowsHost: () => false,
    applicationScrollFallback: true,
    isRuntimeStopped: () => stopped,
  });
  fixture.swipe([100, 148]);
  assert.deepEqual(fixture.inputCalls, []);
  assert.deepEqual(fixture.scrollCalls, [-3]);
  stopped = false;
  fixture.swipe([100, 148]);
  assert.deepEqual(fixture.inputCalls, [APPLICATION_SCROLL_PAGE_UP]);
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

// SPEC #6476 T-005 (FR-002): a phone scrolls the terminal with a finger. The
// swipe is routed through the same wheel policy so fullscreen agents receive
// exactly what a wheel would send them, and retained history scrolls locally.
test("swipe over a fullscreen agent without mouse tracking sends PageUp/PageDown via the wheel policy", () => {
  const fixture = mountFixture({
    isWindowsHost: () => false,
    applicationScrollFallback: true,
    terminalOptions: { buffer: { active: { baseY: 0 } } },
  });

  // The first 4px jitter is below the axis lock but must stay cancelable.
  const moves = fixture.swipe([100, 104, 124, 148]);
  fixture.swipe([300, 252]);

  assert.deepEqual(fixture.inputCalls, [
    APPLICATION_SCROLL_PAGE_UP,
    APPLICATION_SCROLL_PAGE_DOWN,
  ]);
  assert.deepEqual(fixture.inputOptions, [{ focus: false }, { focus: false }],
    "scrolling must not focus the terminal and open a software keyboard");
  assert.ok(moves.every((event) => event.defaultPrevented), "handled swipe must not scroll the page");
  assert.deepEqual(fixture.scrollCalls, []);
  fixture.dispose();
});

test("swipe over a mouse-tracking fullscreen agent reaches xterm as a wheel event", () => {
  const fixture = mountFixture({
    isWindowsHost: () => false,
    applicationScrollFallback: true,
    terminalOptions: {
      buffer: { active: { baseY: 0 } },
      modes: { mouseTrackingMode: "any" },
    },
  });

  fixture.swipe([100, 148]);

  assert.deepEqual(fixture.inputCalls, [], "mouse-tracking agents must not get PageUp");
  assert.equal(fixture.bubbledWheelCount, 1);
  assert.ok(fixture.bubbledWheels[0].deltaY < 0, "finger moving down asks for earlier output");
  fixture.dispose();
});

test("swipe over retained history scrolls xterm lines without application input", () => {
  const observedScrolls = [];
  const fixture = mountFixture({
    isWindowsHost: () => false,
    applicationScrollFallback: true,
    terminalOptions: { buffer: { active: { baseY: 180 } } },
    onUserScroll: () => observedScrolls.push(fixture.scrollCalls.at(-1)),
  });

  fixture.swipe([100, 132]);
  fixture.swipe([300, 284]);

  assert.deepEqual(fixture.scrollCalls, [-2, 1]);
  assert.deepEqual(observedScrolls, [-2, 1], "each user scroll is observed after xterm moves");
  assert.deepEqual(fixture.inputCalls, []);
  assert.equal(fixture.bubbledWheelCount, 0);
  fixture.dispose();
});

test("horizontal and multi-finger gestures are left to the browser", () => {
  const fixture = mountFixture({
    isWindowsHost: () => false,
    applicationScrollFallback: true,
    terminalOptions: { buffer: { active: { baseY: 180 } } },
  });

  const horizontal = fixture.swipe([100, 104, 108], { xs: [100, 160, 220] });
  const pinch = fixture.swipe([100, 160], { fingers: 2 });

  assert.ok(horizontal.every((event) => !event.defaultPrevented));
  assert.ok(pinch.every((event) => !event.defaultPrevented));
  assert.deepEqual(fixture.scrollCalls, []);
  assert.deepEqual(fixture.inputCalls, []);
  fixture.dispose();
});

test("dispose removes the terminal touch listeners", () => {
  const fixture = mountFixture({
    isWindowsHost: () => false,
    terminalOptions: { buffer: { active: { baseY: 180 } } },
  });
  fixture.dispose();

  const moves = fixture.swipe([100, 160]);

  assert.ok(moves.every((event) => !event.defaultPrevented));
  assert.deepEqual(fixture.scrollCalls, []);
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

test("applicationScrollInputForWheel defers to applications that track the mouse", () => {
  for (const mouseTrackingMode of ["any", "vt200", "x10", "drag"]) {
    assert.equal(
      applicationScrollInputForWheel(
        wheelEvent(null, { deltaY: -1 }),
        {
          terminal: { buffer: { active: { baseY: 0 } }, modes: { mouseTrackingMode } },
          enabled: true,
        },
      ),
      null,
      `${mouseTrackingMode} must not synthesize PageUp`,
    );
  }
  assert.equal(
    applicationScrollInputForWheel(
      wheelEvent(null, { deltaY: -1 }),
      {
        terminal: { buffer: { active: { baseY: 0 } }, modes: { mouseTrackingMode: "none" } },
        enabled: true,
      },
    ),
    APPLICATION_SCROLL_PAGE_UP,
  );
  // Terminals without xterm's public `modes` (older builds, test doubles)
  // keep the previous behaviour.
  assert.equal(
    applicationScrollInputForWheel(
      wheelEvent(null, { deltaY: 1 }),
      { terminal: { buffer: { active: { baseY: 0 } } }, enabled: true },
    ),
    APPLICATION_SCROLL_PAGE_DOWN,
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
    /sendTerminalInput:\s*\(data,\s*\{\s*focus\s*=\s*true\s*\}\s*=\s*\{\}\)\s*=>\s*\{[\s\S]*?send\(\{\s*kind:\s*"terminal_input",\s*id:\s*windowId,\s*data\s*\}\);[\s\S]*?\}/,
    "agent fullscreen wheel fallback must send PageUp/PageDown through terminal_input",
  );
});

test("terminal bindings explain forced selection and enable the macOS modifier", () => {
  const appSource = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  assert.match(appSource, /terminalContainer\.title\s*=\s*[\s\S]*?Shift\+drag[\s\S]*?Option\+drag/);
  assert.match(appSource, /macOptionClickForcesSelection:\s*true/);
  assert.match(appSource, /rightClickSelectsWord:\s*false/, "opening Copy must preserve the dragged range on macOS");
});

test("vendored xterm uses browser platforms despite the process shim's title", () => {
  const source = readFileSync(new URL("../vendor/xterm/xterm.mjs", import.meta.url), "utf8");
  const start = source.indexOf("var Fs=");
  const end = source.indexOf(",er=class", start);
  const forceSelection = source.match(/shouldForceSelection\(e\)\{([^}]+)\}/)?.[1];
  assert.ok(start >= 0 && end > start && forceSelection, "pinned xterm platform and selection logic is present");
  const evaluate = new Function("k", "navigator", `${source.slice(start, end)};
    return { isNode: Fs, isMac: Xs, isWindows: Js, isLinux: Zs,
      forceSelection(e) { ${forceSelection} } };`);
  for (const platform of ["MacIntel", "Win32", "Linux x86_64"]) {
    const browser = evaluate({ title: "browser", browser: true }, { platform, userAgent: "Chrome" });
    assert.equal(browser.isNode, false, platform);
    assert.equal(browser.isMac, platform === "MacIntel", platform);
    assert.equal(browser.isWindows, platform === "Win32", platform);
    assert.equal(browser.isLinux, platform === "Linux x86_64", platform);
    const selection = { _optionsService: { rawOptions: { macOptionClickForcesSelection: true } } };
    assert.equal(browser.forceSelection.call(selection, { altKey: true, shiftKey: false }), browser.isMac);
    assert.equal(browser.forceSelection.call(selection, { altKey: false, shiftKey: true }), !browser.isMac);
    assert.equal(browser.forceSelection.call(selection, { altKey: false, shiftKey: false }), false);
  }
  assert.equal(evaluate({ title: "node" }, undefined).isNode, true);
});

function mountFixture({
  isWindowsHost,
  terminalOptions = {},
  applicationScrollFallback = false,
  isRuntimeStopped,
  onUserScroll,
} = {}) {
  const terminalRoot = new FakeTerminalRoot();
  const scrollCalls = [];
  const inputCalls = [];
  const inputOptions = [];
  const bubbledWheels = [];
  terminalRoot.setBubbleHandler((event) => {
    if (event.type === "wheel") {
      bubbledWheels.push(event);
    }
  });

  const controller = createTerminalWheelScrollController({
    terminalRoot,
    terminal: {
      scrollLines: (lines) => scrollCalls.push(lines),
      ...terminalOptions,
    },
    isApplicationScrollFallbackEnabled: () => applicationScrollFallback,
    isRuntimeStopped,
    onUserScroll,
    sendTerminalInput: (data, options) => {
      inputCalls.push(data);
      inputOptions.push(options);
    },
    window: { navigator: { platform: "Win32" }, WheelEvent: FakeWheelEvent },
    isWindowsHost,
  });

  return {
    terminalRoot,
    scrollCalls,
    inputCalls,
    inputOptions,
    bubbledWheels,
    get bubbledWheelCount() {
      return bubbledWheels.length;
    },
    // Dispatches touchstart at ys[0], a touchmove per remaining y, then
    // touchend. Returns the touchmove events.
    swipe(ys, { xs, fingers = 1 } = {}) {
      const point = (index) => ({ clientX: xs ? xs[index] : 100, clientY: ys[index] });
      const touches = (index) => Array.from({ length: fingers }, () => point(index));
      terminalRoot.dispatchEvent(touchEvent("touchstart", touches(0), terminalRoot));
      const moves = ys.slice(1).map((_, offset) => {
        const event = touchEvent("touchmove", touches(offset + 1), terminalRoot);
        terminalRoot.dispatchEvent(event);
        return event;
      });
      terminalRoot.dispatchEvent(touchEvent("touchend", [], terminalRoot));
      return moves;
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

class FakeWheelEvent {
  constructor(type, init = {}) {
    Object.assign(this, wheelEvent(null, { ...init, type }));
  }
}

function touchEvent(type, touches, target) {
  return {
    type,
    target,
    touches,
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
