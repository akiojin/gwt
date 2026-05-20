export function createCanvasWheelGestureClassifier({
  idleMs = 300,
  now,
} = {}) {
  const nowFn = typeof now === "function"
    ? now
    : () => {
        if (typeof performance !== "undefined" && typeof performance.now === "function") {
          return performance.now();
        }
        return Date.now();
      };

  let activeMode = null;
  let lastWheelAt = null;

  function resolveMode(event) {
    return event?.ctrlKey || event?.metaKey ? "zoom" : "pan";
  }

  function classify(event) {
    const timestamp = nowFn();
    if (
      activeMode === null ||
      lastWheelAt === null ||
      timestamp - lastWheelAt > idleMs
    ) {
      activeMode = resolveMode(event);
    }
    lastWheelAt = timestamp;
    return activeMode;
  }

  function reset() {
    activeMode = null;
    lastWheelAt = null;
  }

  return { classify, reset };
}
