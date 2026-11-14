// SPEC #7007 T-014: window_command_error events carry only an internal
// backend message ({ id, operation, message } in protocol.rs — no error
// kind and no retryable flag). The presentation translates each operation
// into the user-visible impact and the next supported action; the
// original wording travels as `details`, so it stays reachable in the
// notification history and the Log surface without becoming the headline
// text (AC-5). Nothing here promises a storage or recovery outcome the
// backend did not state.
const PRESENTATIONS = {
  resize: {
    title: "Resize failed",
    message: "The terminal size may be out of date. Reopen the window to refresh it.",
  },
  input: {
    title: "Input failed",
    message: "Your input was not delivered to the window. Reopen the window and enter it again.",
  },
  stop: {
    title: "Stop failed",
    message: "The window could not be stopped and may still be running. You can try stopping it again from the window.",
  },
};

const FALLBACK = {
  title: "Window command failed",
  message: "The command was not delivered to the window. Reopen the window and try again.",
};

export function windowCommandErrorPresentation(event) {
  const presentation = PRESENTATIONS[event?.operation] || FALLBACK;
  return {
    key: `window-command:${event?.id}:${event?.operation}`,
    title: presentation.title,
    message: presentation.message,
    details: event?.message ? String(event.message) : null,
  };
}
