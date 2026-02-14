export type ShortcutKeyEvent = Pick<
  KeyboardEvent,
  "key" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey"
>;

function normalizeKey(key: string): string {
  return key.length === 1 ? key.toLowerCase() : key;
}

export function isCopyShortcut(event: ShortcutKeyEvent): boolean {
  if (normalizeKey(event.key) !== "c" || event.shiftKey || event.altKey) {
    return false;
  }

  const isCmdC = event.metaKey && !event.ctrlKey;
  const isCtrlC = event.ctrlKey && !event.metaKey;
  return isCmdC || isCtrlC;
}

export function isCtrlCShortcut(event: ShortcutKeyEvent): boolean {
  return isCopyShortcut(event);
}

export function isPasteShortcut(event: ShortcutKeyEvent): boolean {
  if (normalizeKey(event.key) !== "v" || event.altKey) {
    return false;
  }

  const isCmdV = event.metaKey && !event.ctrlKey && !event.shiftKey;
  const isCtrlShiftV = event.ctrlKey && event.shiftKey && !event.metaKey;
  return isCmdV || isCtrlShiftV;
}
