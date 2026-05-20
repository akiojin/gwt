import assert from "node:assert/strict";
import test from "node:test";

import { createCanvasWheelGestureClassifier } from "../canvas-wheel-gesture.js";

test("plain wheel gesture keeps pan mode when Cmd/Shift arrives during the idle window", () => {
  let currentTime = 0;
  const classifier = createCanvasWheelGestureClassifier({
    idleMs: 300,
    now: () => currentTime,
  });

  assert.equal(classifier.classify(wheelEvent()), "pan");

  currentTime = 120;
  assert.equal(
    classifier.classify(wheelEvent({ metaKey: true, shiftKey: true })),
    "pan",
  );
});

test("Cmd/Ctrl wheel after the idle window starts a fresh zoom gesture", () => {
  let currentTime = 0;
  const classifier = createCanvasWheelGestureClassifier({
    idleMs: 300,
    now: () => currentTime,
  });

  assert.equal(classifier.classify(wheelEvent()), "pan");

  currentTime = 301;
  assert.equal(classifier.classify(wheelEvent({ metaKey: true })), "zoom");
});

test("modifier wheel gesture keeps zoom mode until the gesture idles", () => {
  let currentTime = 0;
  const classifier = createCanvasWheelGestureClassifier({
    idleMs: 300,
    now: () => currentTime,
  });

  assert.equal(classifier.classify(wheelEvent({ ctrlKey: true })), "zoom");

  currentTime = 100;
  assert.equal(classifier.classify(wheelEvent()), "zoom");
});

test("reset clears the latched wheel gesture mode", () => {
  let currentTime = 0;
  const classifier = createCanvasWheelGestureClassifier({
    idleMs: 300,
    now: () => currentTime,
  });

  assert.equal(classifier.classify(wheelEvent()), "pan");
  classifier.reset();

  currentTime = 50;
  assert.equal(classifier.classify(wheelEvent({ metaKey: true })), "zoom");
});

function wheelEvent(options = {}) {
  return {
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    ...options,
  };
}
