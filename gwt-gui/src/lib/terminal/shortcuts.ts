export type ShortcutKeyEvent = Pick<
  KeyboardEvent,
  "key" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey"
>;

function normalizeKey(key: string): string {
  return key.length === 1 ? key.toLowerCase() : key;
}

export function isCtrlCShortcut(event: ShortcutKeyEvent): boolean {
  return (
    normalizeKey(event.key) === "c" &&
    event.ctrlKey &&
    !event.metaKey &&
    !event.shiftKey &&
    !event.altKey
  );
}

export function isPasteShortcut(event: ShortcutKeyEvent): boolean {
  if (normalizeKey(event.key) !== "v" || event.altKey) {
    return false;
  }

  const isCmdV = event.metaKey && !event.ctrlKey && !event.shiftKey;
  const isCtrlShiftV = event.ctrlKey && event.shiftKey && !event.metaKey;
  return isCmdV || isCtrlShiftV;
}

