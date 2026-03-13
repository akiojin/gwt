export function applyMenuPasteText(
  target: EventTarget | null,
  text: string,
): boolean {
  if (!text) return false;

  if (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement
  ) {
    const start = target.selectionStart ?? target.value.length;
    const end = target.selectionEnd ?? target.value.length;
    target.setRangeText(text, start, end, "end");

    const event =
      typeof InputEvent === "function"
        ? new InputEvent("input", {
            bubbles: true,
            composed: true,
            data: text,
            inputType: "insertFromPaste",
          })
        : new Event("input", { bubbles: true, composed: true });
    target.dispatchEvent(event);
    return true;
  }

  if (target instanceof HTMLElement && target.isContentEditable) {
    target.focus();
    document.execCommand("insertText", false, text);
    return true;
  }

  return false;
}
