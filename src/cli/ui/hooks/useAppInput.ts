import { useEffect, useRef } from "react";
import { useInput, type Key } from "ink";

const ESCAPE_SEQUENCE_TIMEOUT_MS = 25;

type InputHandler = (input: string, key: Key) => void;
type Options = { isActive?: boolean };

type BufferedEvent = { input: string; key: Key };

type PendingEscape = {
  escapeEvent: BufferedEvent;
  bufferedEvents: BufferedEvent[];
  timeoutId: ReturnType<typeof setTimeout> | null;
};

const createEmptyKey = (): Key => ({
  upArrow: false,
  downArrow: false,
  leftArrow: false,
  rightArrow: false,
  pageDown: false,
  pageUp: false,
  return: false,
  escape: false,
  ctrl: false,
  shift: false,
  tab: false,
  backspace: false,
  delete: false,
  meta: false,
});

function parseArrowDirection(
  sequence: string,
): "up" | "down" | "left" | "right" | null {
  if (!sequence) {
    return null;
  }

  const first = sequence[0];
  if (first !== "[" && first !== "O") {
    return null;
  }

  const last = sequence.at(-1);
  if (!last) {
    return null;
  }

  switch (last.toUpperCase()) {
    case "A":
      return "up";
    case "B":
      return "down";
    case "C":
      return "right";
    case "D":
      return "left";
    default:
      return null;
  }
}

export function useAppInput(
  inputHandler: InputHandler,
  options?: Options,
): void {
  const handlerRef = useRef(inputHandler);
  const pendingRef = useRef<PendingEscape | null>(null);

  useEffect(() => {
    handlerRef.current = inputHandler;
  }, [inputHandler]);

  const clearPending = () => {
    const pending = pendingRef.current;
    if (pending?.timeoutId) {
      clearTimeout(pending.timeoutId);
    }
    pendingRef.current = null;
  };

  const flushPending = () => {
    const pending = pendingRef.current;
    if (!pending) {
      return;
    }

    clearPending();
    handlerRef.current(pending.escapeEvent.input, pending.escapeEvent.key);
    for (const event of pending.bufferedEvents) {
      handlerRef.current(event.input, event.key);
    }
  };

  useEffect(() => {
    if (options?.isActive === false) {
      clearPending();
    }
  }, [options?.isActive]);

  useEffect(() => clearPending, []);

  useInput((input, key) => {
    const pending = pendingRef.current;

    if (!pending) {
      if (key.escape) {
        const timeoutId = setTimeout(flushPending, ESCAPE_SEQUENCE_TIMEOUT_MS);
        pendingRef.current = {
          escapeEvent: { input, key },
          bufferedEvents: [],
          timeoutId,
        };
        return;
      }

      handlerRef.current(input, key);
      return;
    }

    if (pending.timeoutId) {
      clearTimeout(pending.timeoutId);
    }

    pending.bufferedEvents.push({ input, key });

    const [firstEvent] = pending.bufferedEvents;
    const firstInput = firstEvent?.input;

    if (firstInput && firstInput !== "[" && firstInput !== "O") {
      flushPending();
      return;
    }

    const sequence = pending.bufferedEvents
      .map((event) => event.input)
      .join("");
    const arrow = parseArrowDirection(sequence);
    if (arrow) {
      clearPending();
      const arrowKey = createEmptyKey();
      if (arrow === "up") {
        arrowKey.upArrow = true;
      } else if (arrow === "down") {
        arrowKey.downArrow = true;
      } else if (arrow === "left") {
        arrowKey.leftArrow = true;
      } else if (arrow === "right") {
        arrowKey.rightArrow = true;
      }
      handlerRef.current("", arrowKey);
      return;
    }

    pending.timeoutId = setTimeout(flushPending, ESCAPE_SEQUENCE_TIMEOUT_MS);
  }, options);
}
