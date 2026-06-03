const PIXELS_PER_LINE = 32;
const PAGE_LINES = 24;
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

export function shouldOverrideTerminalWheel(event, { window, isWindowsHost: hostCheck } = {}) {
  if (!event || event.ctrlKey || event.metaKey) {
    return false;
  }
  const check = typeof hostCheck === "function" ? hostCheck : () => isWindowsHost(window);
  return check();
}

export function isTerminalMouseTrackingActive(terminal) {
  const mouseTrackingMode = terminal?.modes?.mouseTrackingMode;
  return typeof mouseTrackingMode === "string" && mouseTrackingMode !== "none";
}

export function hasNormalScrollback(terminal) {
  return Number(terminal?.buffer?.active?.baseY || 0) > 0;
}

export function applicationScrollInputForWheel(
  event,
  { terminal, enabled } = {},
) {
  const isEnabled = typeof enabled === "function" ? enabled() : Boolean(enabled);
  if (!isEnabled || !event || event.ctrlKey || event.metaKey || hasNormalScrollback(terminal)) {
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

  const handleWheel = (event) => {
    const applicationScrollInput = applicationScrollInputForWheel(event, {
      terminal,
      enabled: isApplicationScrollFallbackEnabled,
    });
    if (applicationScrollInput && typeof sendTerminalInput === "function") {
      event.preventDefault();
      event.stopPropagation();
      sendTerminalInput(applicationScrollInput);
      return;
    }

    if (
      !shouldOverrideTerminalWheel(event, {
        window,
        isWindowsHost: hostCheck,
      }) ||
      isTerminalMouseTrackingActive(terminal)
    ) {
      return;
    }

    const lines = wheelDeltaToScrollLines(event, { pixelsPerLine, pageLines });
    if (lines === 0) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();
    terminal.scrollLines(lines);
  };

  terminalRoot.addEventListener("wheel", handleWheel, {
    capture: true,
    passive: false,
  });

  return {
    dispose() {
      terminalRoot.removeEventListener("wheel", handleWheel, {
        capture: true,
      });
    },
  };
}
