export function classifyTerminalCopyKeyEvent(event, {
  platform = detectTerminalShortcutPlatform(),
  hasSelection = false,
} = {}) {
  const empty = { copy: false, clearSelectionAfterCopy: false };
  if (event?.type && event.type !== "keydown") {
    return empty;
  }
  if (!isCopyKey(event)) {
    return empty;
  }
  if (event.altKey || event.metaKey || !event.ctrlKey) {
    return empty;
  }

  const family = platformFamily(platform);
  if (family === "mac") {
    return empty;
  }
  if (event.shiftKey) {
    return { copy: true, clearSelectionAfterCopy: false };
  }
  if (family === "windows" && hasSelection) {
    return { copy: true, clearSelectionAfterCopy: true };
  }
  return empty;
}

export function detectTerminalShortcutPlatform(navigatorLike = globalThis.navigator) {
  return navigatorLike?.userAgentData?.platform || navigatorLike?.platform || "";
}

function isCopyKey(event) {
  return String(event?.key ?? "").toLowerCase() === "c";
}

function platformFamily(platform) {
  const value = String(platform || "");
  if (/mac|iphone|ipad|ipod/i.test(value)) {
    return "mac";
  }
  if (/win/i.test(value)) {
    return "windows";
  }
  return "other";
}
