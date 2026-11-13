import test from "node:test";
import assert from "node:assert/strict";
import { windowCommandErrorPresentation } from "../window-command-error.js";

// SPEC #7007 T-014: window_command_error events carry only an internal
// backend message. The presentation translates each operation into the
// impact and the next supported action; the original wording stays
// reachable as details and is never the primary text.

const RAW = "owner health: Some(Degraded); owner identity mismatch. Automatic recovery stopped after 10 attempts";

test("resize failures explain the stale size and how to refresh it", () => {
  const presentation = windowCommandErrorPresentation({ id: "agent-1", operation: "resize", message: RAW });
  assert.equal(presentation.key, "window-command:agent-1:resize");
  assert.equal(presentation.title, "Resize failed");
  assert.match(presentation.message, /terminal size may be out of date/i);
  assert.match(presentation.message, /[Rr]eopen the window/);
  assert.doesNotMatch(presentation.message, /owner identity mismatch/);
  assert.equal(presentation.details, RAW);
});

test("input failures say the input was not delivered and to enter it again", () => {
  const presentation = windowCommandErrorPresentation({ id: "agent-1", operation: "input", message: RAW });
  assert.equal(presentation.title, "Input failed");
  assert.match(presentation.message, /was not delivered/);
  assert.match(presentation.message, /enter it again/);
  assert.equal(presentation.details, RAW);
});

test("stop failures warn the window may still be running and offer a retry", () => {
  const presentation = windowCommandErrorPresentation({ id: "agent-1", operation: "stop", message: RAW });
  assert.equal(presentation.title, "Stop failed");
  assert.match(presentation.message, /may still be running/);
  assert.match(presentation.message, /try stopping it again/);
  assert.equal(presentation.details, RAW);
});

test("unknown operations fall back to a generic typed presentation", () => {
  const presentation = windowCommandErrorPresentation({ id: "agent-1", operation: "teleport", message: RAW });
  assert.equal(presentation.key, "window-command:agent-1:teleport");
  assert.equal(presentation.title, "Window command failed");
  assert.match(presentation.message, /was not delivered/);
  assert.equal(presentation.details, RAW);
});

test("presentations never promise storage or recovery outcomes", () => {
  for (const operation of ["resize", "input", "stop", "other"]) {
    const { message } = windowCommandErrorPresentation({ id: "a", operation, message: RAW });
    assert.doesNotMatch(message, /work is kept|restart fixes|will recover|saved work is safe/i);
  }
});

test("a missing backend message leaves details empty instead of inventing one", () => {
  const presentation = windowCommandErrorPresentation({ id: "a", operation: "input", message: null });
  assert.equal(presentation.details, null);
});
