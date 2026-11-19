// Issue #7702: an IME commit that lands while the GUI thread was stalled must
// reach the PTY once. xterm 6 finalizes a composition from a non-229 keydown
// and again from compositionend's timeout. These tests drive that sequence
// against the commit gate, then pin the app.js wiring.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { createTerminalInputCommitGate } from "../terminal-input-commit.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

function createTarget() {
  const listeners = [];
  return {
    addEventListener(type, handler, options) {
      const capture = options === true || options?.capture === true;
      listeners.push({ type, handler, capture });
    },
    removeEventListener(type, handler, options) {
      const capture = options === true || options?.capture === true;
      const index = listeners.findIndex(
        (entry) => entry.type === type && entry.handler === handler && entry.capture === capture,
      );
      if (index >= 0) listeners.splice(index, 1);
    },
    dispatch(type, phase) {
      const capture = phase === "capture";
      for (const entry of [...listeners]) {
        if (entry.type === type && entry.capture === capture) entry.handler();
      }
    },
  };
}

function createHarness() {
  const queue = [];
  const gate = createTerminalInputCommitGate({
    schedule(fn) {
      queue.push(fn);
    },
  });
  const target = createTarget();
  const detach = gate.attach(target);
  return {
    gate,
    target,
    detach,
    flush() {
      const fn = queue.shift();
      if (fn) fn();
    },
  };
}

function beginComposition(harness) {
  harness.target.dispatch("compositionstart", "capture");
}

function keydownCommit(harness, text) {
  harness.target.dispatch("keydown", "capture");
  const settled = harness.gate.accept(text);
  harness.target.dispatch("keydown", "bubble");
  return settled;
}

function endComposition(harness) {
  harness.target.dispatch("compositionend", "capture");
}

test("ascii keydown is forwarded on every press", () => {
  const harness = createHarness();
  harness.target.dispatch("keydown", "capture");
  const first = harness.gate.accept("a");
  const second = harness.gate.accept("a");
  harness.target.dispatch("keydown", "bubble");
  assert.deepEqual(first, { forward: true, data: "a" });
  assert.deepEqual(second, { forward: true, data: "a" });
});

test("ime sync finalize is kept and the compositionend echo is dropped", () => {
  const harness = createHarness();
  const phrase = "症状は同じです。";
  beginComposition(harness);
  const committed = keydownCommit(harness, phrase);
  endComposition(harness);
  harness.flush();
  const echo = harness.gate.accept(phrase);
  harness.flush();
  assert.deepEqual(committed, { forward: true, data: phrase });
  assert.deepEqual(echo, { forward: false, data: "" });
});

test("a composition that only echoes from the timeout is forwarded once", () => {
  const harness = createHarness();
  const phrase = "症状は同じです。";
  beginComposition(harness);
  endComposition(harness);
  const committed = harness.gate.accept(phrase);
  assert.deepEqual(committed, { forward: true, data: phrase });
});

test("two different phrases are each forwarded once", () => {
  const harness = createHarness();
  const firstPhrase = "症状は同じです。";
  const secondPhrase = "また、PMの再起動をしましたが、";
  beginComposition(harness);
  const first = keydownCommit(harness, firstPhrase);
  endComposition(harness);
  harness.flush();
  const firstEcho = harness.gate.accept(firstPhrase);
  harness.flush();
  beginComposition(harness);
  const second = keydownCommit(harness, secondPhrase);
  endComposition(harness);
  harness.flush();
  const secondEcho = harness.gate.accept(secondPhrase);
  harness.flush();
  assert.deepEqual(first, { forward: true, data: firstPhrase });
  assert.deepEqual(firstEcho, { forward: false, data: "" });
  assert.deepEqual(second, { forward: true, data: secondPhrase });
  assert.deepEqual(secondEcho, { forward: false, data: "" });
});

test("the same phrase typed twice is forwarded twice", () => {
  const harness = createHarness();
  const phrase = "症状は同じです。";
  beginComposition(harness);
  const first = keydownCommit(harness, phrase);
  endComposition(harness);
  beginComposition(harness);
  const second = keydownCommit(harness, phrase);
  const firstEcho = harness.gate.accept(phrase);
  endComposition(harness);
  harness.flush();
  const secondEcho = harness.gate.accept(phrase);
  harness.flush();
  assert.deepEqual(first, { forward: true, data: phrase });
  assert.deepEqual(second, { forward: true, data: phrase });
  assert.deepEqual(firstEcho, { forward: false, data: "" });
  assert.deepEqual(secondEcho, { forward: false, data: "" });
});

test("an empty keydown finalize does not drop the later composition text", () => {
  const harness = createHarness();
  const phrase = "症状は同じです。";
  beginComposition(harness);
  const empty = keydownCommit(harness, "");
  endComposition(harness);
  const committed = harness.gate.accept(phrase);
  assert.deepEqual(empty, { forward: true, data: "" });
  assert.deepEqual(committed, { forward: true, data: phrase });
});

test("a longer echo forwards only the suffix that was not already sent", () => {
  const harness = createHarness();
  const phrase = "症状は同じです。";
  beginComposition(harness);
  const committed = keydownCommit(harness, phrase);
  endComposition(harness);
  const echo = harness.gate.accept(`${phrase}、`);
  assert.deepEqual(committed, { forward: true, data: phrase });
  assert.deepEqual(echo, { forward: true, data: "、" });
});

test("a paste of the pending echo is forwarded whole", () => {
  const harness = createHarness();
  const phrase = "症状は同じです。";
  beginComposition(harness);
  keydownCommit(harness, phrase);
  endComposition(harness);
  harness.target.dispatch("paste", "capture");
  const pasted = harness.gate.accept(phrase);
  harness.target.dispatch("paste", "bubble");
  assert.deepEqual(pasted, { forward: true, data: phrase });
});

test("after the echo retires, the same phrase is forwarded again", () => {
  const harness = createHarness();
  const phrase = "症状は同じです。";
  beginComposition(harness);
  keydownCommit(harness, phrase);
  endComposition(harness);
  harness.flush();
  harness.gate.accept(phrase);
  harness.flush();
  beginComposition(harness);
  endComposition(harness);
  const again = harness.gate.accept(phrase);
  assert.deepEqual(again, { forward: true, data: phrase });
});

test("a new composition's keydown is kept when the previous echo is still queued", () => {
  const harness = createHarness();
  const firstPhrase = "症状は同じです。";
  const secondPhrase = "また、PMの再起動をしましたが、";
  beginComposition(harness);
  keydownCommit(harness, firstPhrase);
  endComposition(harness);
  beginComposition(harness);
  const second = keydownCommit(harness, secondPhrase);
  const firstEcho = harness.gate.accept(firstPhrase);
  assert.deepEqual(second, { forward: true, data: secondPhrase });
  assert.deepEqual(firstEcho, { forward: false, data: "" });
});

test("an echo that does not match the pending head is forwarded and the head stays", () => {
  const harness = createHarness();
  const phrase = "症状は同じです。";
  beginComposition(harness);
  keydownCommit(harness, phrase);
  endComposition(harness);
  const other = harness.gate.accept("別の入力");
  const echo = harness.gate.accept(phrase);
  assert.deepEqual(other, { forward: true, data: "別の入力" });
  assert.deepEqual(echo, { forward: false, data: "" });
});

test("an earlier textarea capture keydown still arms the commit through the parent", () => {
  const parent = createTarget();
  const textarea = createTarget();
  textarea.parentElement = parent;
  const queue = [];
  const gate = createTerminalInputCommitGate({
    schedule(fn) {
      queue.push(fn);
    },
  });
  const phrase = "症状は同じです。";
  let committed = null;
  textarea.addEventListener("keydown", () => {
    committed = gate.accept(phrase);
  }, true);
  gate.attach(textarea);

  textarea.dispatch("compositionstart", "capture");
  parent.dispatch("keydown", "capture");
  textarea.dispatch("keydown", "capture");
  textarea.dispatch("keydown", "bubble");
  textarea.dispatch("compositionend", "capture");
  const scheduleOuter = queue.shift();
  assert.equal(typeof scheduleOuter, "function");
  scheduleOuter();
  const echo = gate.accept(phrase);
  assert.deepEqual(committed, { forward: true, data: phrase });
  assert.deepEqual(echo, { forward: false, data: "" });
});

test("app.js filters terminal input through the commit gate after the readiness gate", () => {
  assert.match(
    appSource,
    /import \{ createTerminalInputCommitGate \} from "\/terminal-input-commit\.js";/,
  );
  const opened = appSource.indexOf("terminal.open(terminalContainer);");
  const attached = appSource.indexOf("commitGate.attach(");
  const onData = appSource.indexOf("terminal.onData((data) =>");
  const readiness = appSource.indexOf("gateTerminalInputForReadiness", onData);
  const accepted = appSource.indexOf("commitGate.accept(", onData);
  const sent = appSource.indexOf('kind: "terminal_input"', onData);
  assert.ok(opened > 0 && attached > opened, "attach the gate after terminal.open()");
  assert.ok(readiness > onData && accepted > readiness && sent > accepted);
});
