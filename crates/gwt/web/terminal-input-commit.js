// Issue #7702: xterm 6's CompositionHelper emits an IME commit twice when a
// non-229 keydown finalizes the textarea synchronously and compositionend
// schedules that text again. A stalled GUI thread flushes the pair together,
// so the PTY receives each phrase twice. The keydown commit is the one the
// user confirmed. The later timeout is an echo, except when it carries extra
// characters the keydown send did not include.

export function createTerminalInputCommitGate({ schedule } = {}) {
  const enqueue = typeof schedule === "function"
    ? schedule
    : (fn) => {
        setTimeout(fn, 0);
      };
  let inKeydown = false;
  let inPaste = false;
  let composing = false;
  let nextId = 1;
  /** @type {{ id: number, text: string, closed: boolean }[]} */
  let pending = [];

  function retireLater(ids) {
    if (ids.length === 0) return;
    const retire = new Set(ids);
    // compositionend's capture listener queues this before xterm queues the
    // echo timeout. The inner turn therefore runs after that echo.
    enqueue(() => {
      enqueue(() => {
        pending = pending.filter((item) => !retire.has(item.id));
      });
    });
  }

  function onCompositionEnd() {
    composing = false;
    const ids = [];
    for (const item of pending) {
      if (!item.closed) {
        item.closed = true;
        ids.push(item.id);
      }
    }
    retireLater(ids);
  }

  function attach(target) {
    // xterm 6 registers keydown on the textarea during capture, and it does
    // that inside terminal.open(), before this gate can attach. A second
    // capture listener on the same node therefore runs after the synchronous
    // commit. Capture on the ancestor runs first and arms the commit in time.
    const keydownRoot = target.parentElement || target.parentNode || target;
    const onKeydownCapture = () => {
      inKeydown = true;
    };
    const onKeydownBubble = () => {
      inKeydown = false;
    };
    const onPasteCapture = () => {
      inPaste = true;
    };
    const onPasteBubble = () => {
      inPaste = false;
    };
    const onCompositionStart = () => {
      composing = true;
    };
    keydownRoot.addEventListener("keydown", onKeydownCapture, true);
    target.addEventListener("keydown", onKeydownBubble, false);
    target.addEventListener("paste", onPasteCapture, true);
    target.addEventListener("paste", onPasteBubble, false);
    target.addEventListener("compositionstart", onCompositionStart, true);
    target.addEventListener("compositionend", onCompositionEnd, true);
    return () => {
      keydownRoot.removeEventListener("keydown", onKeydownCapture, true);
      target.removeEventListener("keydown", onKeydownBubble, false);
      target.removeEventListener("paste", onPasteCapture, true);
      target.removeEventListener("paste", onPasteBubble, false);
      target.removeEventListener("compositionstart", onCompositionStart, true);
      target.removeEventListener("compositionend", onCompositionEnd, true);
    };
  }

  function accept(data) {
    const text = typeof data === "string" ? data : "";
    if (inKeydown && composing && text.length > 0) {
      pending.push({ id: nextId, text, closed: false });
      nextId += 1;
      return { forward: true, data: text };
    }
    const head = pending[0];
    if (
      !inKeydown
      && !inPaste
      && head?.closed
      && head.text.length > 0
      && text.startsWith(head.text)
    ) {
      pending.shift();
      const suffix = text.slice(head.text.length);
      if (suffix.length === 0) return { forward: false, data: "" };
      return { forward: true, data: suffix };
    }
    return { forward: true, data: text };
  }

  return { attach, accept };
}
