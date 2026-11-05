const PIXELS_PER_LINE = 32;
const PAGE_LINES = 24;
// Finger travel per scrollback line, and per synthesized wheel notch for
// fullscreen applications (one notch = one PageUp or one wheel report).
const TOUCH_PIXELS_PER_LINE = 16;
const TOUCH_PIXELS_PER_NOTCH = 48;
const TOUCH_NOTCH_DELTA_Y = 96;
// Movement below this is still a tap; the first larger move fixes the axis.
const TOUCH_AXIS_LOCK_PIXELS = 8;
export const APPLICATION_SCROLL_PAGE_UP = "\x1b[5~";
export const APPLICATION_SCROLL_PAGE_DOWN = "\x1b[6~";

export function isWindowsHost(windowRef = globalThis) {
  const navigator = windowRef?.navigator || globalThis.navigator || {};
  const platform =
    navigator.userAgentData?.platform ||
    navigator.platform ||
    navigator.userAgent ||
    "";
  return /\bWin/i.test(String(platform));
}

export function wheelDeltaToScrollLines(
  event,
  { pixelsPerLine = PIXELS_PER_LINE, pageLines = PAGE_LINES } = {},
) {
  const deltaY = Number(event?.deltaY || 0);
  if (deltaY === 0) {
    return 0;
  }

  let rawLines = deltaY / pixelsPerLine;
  if (event?.deltaMode === 1) {
    rawLines = deltaY;
  } else if (event?.deltaMode === 2) {
    rawLines = deltaY * pageLines;
  }

  return Math.sign(rawLines) * Math.max(1, Math.ceil(Math.abs(rawLines)));
}

function shouldOverrideTerminalWheel(event, { terminal, window, isWindowsHost: hostCheck } = {}) {
  if (!event || event.ctrlKey || event.metaKey) {
    return false;
  }
  const check = typeof hostCheck === "function" ? hostCheck : () => isWindowsHost(window);
  return (hasNormalScrollback(terminal) && isTerminalMouseTrackingActive(terminal)) || check();
}

export function isTerminalMouseTrackingActive(terminal) {
  const mouseTrackingMode = terminal?.modes?.mouseTrackingMode;
  return typeof mouseTrackingMode === "string" && mouseTrackingMode !== "none";
}

export function hasNormalScrollback(terminal) {
  return Number(terminal?.buffer?.active?.baseY || 0) > 0;
}

// Issue #4909 AC-6: an application that enabled mouse tracking already receives
// the wheel as mouse events from xterm.js, so a synthesized PageUp/PageDown
// would reach it twice. Codex TUI opens its transcript overlay on PageUp and
// routes the following keystrokes to Find, so the fallback must stay silent
// there and only serve applications that handle no mouse input (Claude Code).
export function applicationScrollInputForWheel(
  event,
  { terminal, enabled } = {},
) {
  const isEnabled = typeof enabled === "function" ? enabled() : Boolean(enabled);
  if (
    !isEnabled ||
    !event ||
    event.ctrlKey ||
    event.metaKey ||
    hasNormalScrollback(terminal) ||
    isTerminalMouseTrackingActive(terminal)
  ) {
    return null;
  }
  const deltaY = Number(event.deltaY || 0);
  if (deltaY < 0) {
    return APPLICATION_SCROLL_PAGE_UP;
  }
  if (deltaY > 0) {
    return APPLICATION_SCROLL_PAGE_DOWN;
  }
  return null;
}

export function createTerminalWheelScrollController({
  terminalRoot,
  terminal,
  window = globalThis,
  isWindowsHost: hostCheck,
  isApplicationScrollFallbackEnabled,
  sendTerminalInput,
  pixelsPerLine = PIXELS_PER_LINE,
  pageLines = PAGE_LINES,
} = {}) {
  if (!terminalRoot || typeof terminalRoot.addEventListener !== "function") {
    throw new Error("terminal wheel scroll requires a terminalRoot");
  }
  if (!terminal || typeof terminal.scrollLines !== "function") {
    throw new Error("terminal wheel scroll requires terminal.scrollLines");
  }

  let dispatchingTouchWheel = false;
  const handleWheel = (event) => {
    const applicationScrollInput = applicationScrollInputForWheel(event, {
      terminal,
      enabled: isApplicationScrollFallbackEnabled,
    });
    if (applicationScrollInput && typeof sendTerminalInput === "function") {
      event.preventDefault();
      event.stopPropagation();
      sendTerminalInput(applicationScrollInput, { focus: !dispatchingTouchWheel });
      return;
    }

    if (
      !shouldOverrideTerminalWheel(event, {
        terminal,
        window,
        isWindowsHost: hostCheck,
      }) ||
      (isTerminalMouseTrackingActive(terminal) && !hasNormalScrollback(terminal))
    ) {
      return;
    }

    const lines = wheelDeltaToScrollLines(event, { pixelsPerLine, pageLines });
    if (lines === 0) {
      return;
    }

    // Retained history belongs to the terminal viewport, even when an agent
    // tracks the mouse. Fullscreen buffers without history still reach xterm.
    event.preventDefault();
    event.stopPropagation();
    terminal.scrollLines(lines);
  };

  // SPEC #6476 T-005 (FR-002): a phone scrolls with a finger. Retained history
  // scrolls the xterm viewport directly; a buffer without history gets one
  // synthesized wheel notch per TOUCH_PIXELS_PER_NOTCH, dispatched at the
  // touched element so handleWheel and xterm apply the wheel policy unchanged
  // (PageUp/PageDown fallback, mouse wheel reports, alternate-screen keys).
  let touch = null;

  const handleTouchStart = (event) => {
    const point = event.touches?.length === 1 ? event.touches[0] : null;
    touch = point
      ? { target: event.target, x: point.clientX, y: point.clientY, axis: null, pending: 0 }
      : null;
  };

  const handleTouchMove = (event) => {
    const point = event.touches?.length === 1 ? event.touches[0] : null;
    if (!touch || !point) {
      touch = null;
      return;
    }
    const dx = point.clientX - touch.x;
    const dy = point.clientY - touch.y;
    if (!touch.axis) {
      if (Math.max(Math.abs(dx), Math.abs(dy)) < TOUCH_AXIS_LOCK_PIXELS) {
        // Keep a vertical start cancelable; once the browser starts panning
        // the page, later touchmove events can no longer be prevented.
        if (Math.abs(dy) >= Math.abs(dx)) {
          event.preventDefault();
        }
        return;
      }
      touch.axis = Math.abs(dy) > Math.abs(dx) ? "y" : "x";
    }
    if (touch.axis !== "y") {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    touch.x = point.clientX;
    touch.y = point.clientY;
    touch.pending += dy;

    if (hasNormalScrollback(terminal)) {
      const lines = Math.trunc(touch.pending / TOUCH_PIXELS_PER_LINE);
      if (lines !== 0) {
        touch.pending -= lines * TOUCH_PIXELS_PER_LINE;
        // A finger moving down reveals earlier output.
        terminal.scrollLines(-lines);
      }
      return;
    }

    const WheelEventCtor = window?.WheelEvent || globalThis.WheelEvent;
    const target = touch.target || terminalRoot;
    while (Math.abs(touch.pending) >= TOUCH_PIXELS_PER_NOTCH && WheelEventCtor) {
      const direction = Math.sign(touch.pending);
      touch.pending -= direction * TOUCH_PIXELS_PER_NOTCH;
      dispatchingTouchWheel = true;
      try {
        target.dispatchEvent(
          new WheelEventCtor("wheel", {
            bubbles: true,
            cancelable: true,
            clientX: point.clientX,
            clientY: point.clientY,
            deltaMode: 0,
            deltaY: -direction * TOUCH_NOTCH_DELTA_Y,
          }),
        );
      } finally {
        dispatchingTouchWheel = false;
      }
    }
  };

  const handleTouchEnd = () => {
    touch = null;
  };

  terminalRoot.addEventListener("wheel", handleWheel, {
    capture: true,
    passive: false,
  });
  terminalRoot.addEventListener("touchstart", handleTouchStart, {
    capture: true,
    passive: true,
  });
  terminalRoot.addEventListener("touchmove", handleTouchMove, {
    capture: true,
    passive: false,
  });
  terminalRoot.addEventListener("touchend", handleTouchEnd, { capture: true });
  terminalRoot.addEventListener("touchcancel", handleTouchEnd, { capture: true });

  return {
    dispose() {
      terminalRoot.removeEventListener("wheel", handleWheel, {
        capture: true,
      });
      terminalRoot.removeEventListener("touchstart", handleTouchStart, {
        capture: true,
      });
      terminalRoot.removeEventListener("touchmove", handleTouchMove, {
        capture: true,
      });
      terminalRoot.removeEventListener("touchend", handleTouchEnd, { capture: true });
      terminalRoot.removeEventListener("touchcancel", handleTouchEnd, { capture: true });
      touch = null;
    },
  };
}
