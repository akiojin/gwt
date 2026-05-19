const PIXELS_PER_LINE = 32;
const PAGE_LINES = 24;

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

export function createTerminalWheelScrollController({
  terminalRoot,
  terminal,
  window = globalThis,
  isWindowsHost: hostCheck,
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
