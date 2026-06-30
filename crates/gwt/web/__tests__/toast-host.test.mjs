// SPEC #3206 — shared floating-toast primitive (createToastStack) contract.

import assert from "node:assert/strict";
import { test } from "node:test";
import { parseHTML } from "linkedom";

import { createToastStack } from "../toast-host.js";

function setup(opts = {}) {
  const { document } = parseHTML(
    "<!doctype html><html><head></head><body></body></html>",
  );
  const stack = createToastStack({ document, className: "toast-x", ...opts });
  stack.mount(document.body);
  return { document, stack };
}

test("requires document and className", () => {
  const { document } = parseHTML("<!doctype html>");
  assert.throws(() => createToastStack({ className: "x" }), /document/);
  assert.throws(() => createToastStack({ document }), /className/);
});

test("mount creates a region with role/aria and a list", () => {
  const { document } = setup({
    ariaRole: "log",
    ariaLabel: "Region label",
  });
  const region = document.querySelector(".toast-x");
  assert.ok(region);
  assert.equal(region.getAttribute("role"), "log");
  assert.equal(region.getAttribute("aria-live"), "polite");
  assert.equal(region.getAttribute("aria-label"), "Region label");
  assert.ok(document.querySelector(".toast-x__list"));
});

test("push renders title, dismiss and message in DOM order with derived classes", () => {
  const { document, stack } = setup();
  const item = stack.push({ level: "warn", title: "Heads up", message: "details" });
  assert.equal(item.dataset.level, "warn");
  const children = [...item.children].map((c) => c.className);
  assert.deepEqual(children, ["toast-x__title", "toast-x__dismiss", "toast-x__message"]);
  assert.match(item.textContent, /Heads up/);
  assert.match(item.textContent, /details/);
});

test("newest-on-top by default; oldest-first when disabled", () => {
  const { document, stack } = setup();
  stack.push({ title: "first" });
  stack.push({ title: "second" });
  const items = document.querySelectorAll(".toast-x__item");
  assert.match(items[0].textContent, /second/, "newest prepended");

  const bottomUp = setup({ newestOnTop: false });
  bottomUp.stack.push({ title: "first" });
  bottomUp.stack.push({ title: "second" });
  const ordered = bottomUp.document.querySelectorAll(".toast-x__item");
  assert.match(ordered[0].textContent, /first/, "oldest stays first when appended");
});

test("bounded cap drops the oldest and counts overflow; 0 = unbounded", () => {
  const { document, stack } = setup({ maxRetained: 3 });
  for (let i = 0; i < 10; i += 1) {
    stack.push({ title: `n${i}` });
  }
  assert.equal(stack.count(), 3);
  assert.equal(stack.droppedCount(), 7);
  assert.equal(document.querySelectorAll(".toast-x__item").length, 3);
  // newest retained, oldest dropped
  assert.match(document.querySelector(".toast-x__item").textContent, /n9/);

  const { stack: unbounded } = setup({ maxRetained: 0 });
  for (let i = 0; i < 10; i += 1) {
    unbounded.push({ title: `n${i}` });
  }
  assert.equal(unbounded.count(), 10, "maxRetained:0 never caps");
  assert.equal(unbounded.droppedCount(), 0);
});

test("level normalizes against the allowed set with a fallback", () => {
  const { stack } = setup({ levels: ["info", "error"], defaultLevel: "info" });
  assert.equal(stack.push({ level: "error" }).dataset.level, "error");
  assert.equal(stack.push({ level: "BOGUS" }).dataset.level, "info");
  assert.equal(stack.push({}).dataset.level, "info");
});

test("dismissible:false omits the dismiss button; dismiss removes the item", () => {
  const { document, stack } = setup();
  const sticky = stack.push({ title: "sticky", dismissible: false });
  assert.equal(sticky.querySelector(".toast-x__dismiss"), null);

  stack.push({ title: "closable" });
  const dismiss = document.querySelector(".toast-x__item .toast-x__dismiss");
  assert.ok(dismiss);
  const before = stack.count();
  dismiss.dispatchEvent(new document.defaultView.Event("click"));
  assert.equal(stack.count(), before - 1, "dismiss removes its item");
});

test("message is omitted when not provided", () => {
  const { stack } = setup();
  const item = stack.push({ title: "no message" });
  assert.equal(item.querySelector(".toast-x__message"), null);
});

test("push before mount is a no-op (returns null)", () => {
  const { document } = parseHTML("<!doctype html><html><head></head><body></body></html>");
  const stack = createToastStack({ document, className: "toast-y" });
  assert.equal(stack.push({ title: "x" }), null);
  assert.equal(stack.count(), 0);
});

// --- alerts-region capabilities (SPEC #3206 P1) ---

test("an id replaces the prior toast carrying that id (dedup)", () => {
  const { stack } = setup();
  stack.push({ id: "win-1", title: "first" });
  stack.push({ id: "win-2", title: "other" });
  const replaced = stack.push({ id: "win-1", title: "second" });
  assert.equal(stack.count(), 2, "same id supersedes, not appends");
  assert.equal(replaced.dataset.toastId, "win-1");
  assert.match(replaced.textContent, /second/);
});

test("onActivate makes the item a keyboard button that runs then dismisses", () => {
  const { document, stack } = setup();
  let activated = 0;
  const item = stack.push({ title: "jump", onActivate: () => (activated += 1) });
  assert.equal(item.getAttribute("role"), "button");
  assert.equal(item.getAttribute("tabindex"), "0");
  item.dispatchEvent(new document.defaultView.Event("click"));
  assert.equal(activated, 1, "activation handler ran");
  assert.equal(stack.count(), 0, "activation dismisses the toast");
});

test("the dismiss button does not trigger onActivate", () => {
  const { document, stack } = setup();
  let activated = 0;
  stack.push({ title: "x", onActivate: () => (activated += 1), dismissible: true });
  const dismiss = document.querySelector(".toast-x__dismiss");
  const event = new document.defaultView.Event("click", { bubbles: true });
  dismiss.dispatchEvent(event);
  assert.equal(activated, 0, "dismiss stops propagation to onActivate");
  assert.equal(stack.count(), 0);
});

test("timeoutMs auto-dismisses; absent timeout stays sticky", async () => {
  const { stack } = setup();
  stack.push({ title: "sticky" });
  stack.push({ title: "transient", timeoutMs: 10 });
  assert.equal(stack.count(), 2);
  await new Promise((resolve) => setTimeout(resolve, 40));
  assert.equal(stack.count(), 1, "only the timed toast auto-dismissed");
});

test("animateDismiss marks leaving then removes via the fallback timer", async () => {
  const { document, stack } = setup({ animateDismiss: true, dismissMs: 10 });
  const item = stack.push({ title: "x" });
  document.querySelector(".toast-x__dismiss").dispatchEvent(
    new document.defaultView.Event("click", { bubbles: true }),
  );
  assert.equal(item.dataset.leaving, "true", "collapse begins (not removed instantly)");
  assert.equal(stack.count(), 1, "still present during the collapse");
  await new Promise((resolve) => setTimeout(resolve, 40));
  assert.equal(stack.count(), 0, "fallback timer removes after the collapse");
});
