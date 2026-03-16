const TEXT_LIKE_INPUT_TYPES = new Set([
  "text",
  "password",
  "search",
  "email",
  "url",
  "tel",
]);

function buildPasteInputEvent(text: string): Event {
  return typeof InputEvent === "function"
    ? new InputEvent("input", {
        bubbles: true,
        composed: true,
        data: text,
        inputType: "insertFromPaste",
      })
    : new Event("input", { bubbles: true, composed: true });
}

function isTextLikeInput(target: HTMLInputElement): boolean {
  return TEXT_LIKE_INPUT_TYPES.has(target.type.toLowerCase());
}

function insertIntoContentEditable(target: HTMLElement, text: string): void {
  target.focus();
  const selection = target.ownerDocument.defaultView?.getSelection() ?? null;
  if (!selection || selection.rangeCount === 0) {
    target.textContent = `${target.textContent ?? ""}${text}`;
    return;
  }

  const range = selection.getRangeAt(0);
  if (!target.contains(range.commonAncestorContainer)) {
    target.textContent = `${target.textContent ?? ""}${text}`;
    return;
  }

  range.deleteContents();
  const textNode = target.ownerDocument.createTextNode(text);
  range.insertNode(textNode);
  range.setStartAfter(textNode);
  range.collapse(true);
  selection.removeAllRanges();
  selection.addRange(range);
}

/**
 * Apply menu-paste text into a DOM editing target.
 *
 * Supports textareas, text-like inputs, other inputs via value fallback, and
 * contenteditable elements. Dispatches a bubbling `input` event when handled.
 *
 * @param target DOM event target that may receive pasted text.
 * @param text Clipboard text to insert.
 * @returns `true` when the paste was applied locally, otherwise `false`.
 */
export function applyMenuPasteText(
  target: EventTarget | null,
  text: string,
): boolean {
  if (!text) return false;

  if (target instanceof HTMLTextAreaElement) {
    const start = target.selectionStart ?? target.value.length;
    const end = target.selectionEnd ?? target.value.length;
    target.setRangeText(text, start, end, "end");
    target.dispatchEvent(buildPasteInputEvent(text));
    return true;
  }

  if (target instanceof HTMLInputElement) {
    const start = target.selectionStart ?? target.value.length;
    const end = target.selectionEnd ?? target.value.length;
    if (isTextLikeInput(target)) {
      target.setRangeText(text, start, end, "end");
    } else {
      target.value = `${target.value.slice(0, start)}${text}${target.value.slice(end)}`;
    }
    target.dispatchEvent(buildPasteInputEvent(text));
    return true;
  }

  if (target instanceof HTMLElement && target.isContentEditable) {
    insertIntoContentEditable(target, text);
    target.dispatchEvent(buildPasteInputEvent(text));
    return true;
  }

  return false;
}
