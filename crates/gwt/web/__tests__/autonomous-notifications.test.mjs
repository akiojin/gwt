// SPEC #3200 FR-034 / FR-035 — autonomous notification side-stack contract.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { parseHTML } from "linkedom";

import { createAutonomousNotifications } from "../autonomous-notifications.js";

function setup({ maxRetained } = {}) {
  const { document } = parseHTML(
    "<!doctype html><html><head></head><body></body></html>",
  );
  const stack = createAutonomousNotifications({ document, maxRetained });
  stack.mount(document.body);
  return { document, stack };
}

test("mount creates an aria-live log region", () => {
  const { document } = setup();
  const region = document.querySelector(".autonomous-notifications");
  assert.ok(region, "region is mounted");
  assert.equal(region.getAttribute("role"), "log");
  assert.equal(region.getAttribute("aria-live"), "polite");
  assert.ok(document.querySelector(".autonomous-notifications__list"));
});

test("push appends newest notice on top with level, title, issue and message", () => {
  const { document, stack } = setup();
  stack.push({ level: "info", title: "Launched", issueNumber: 42, message: "started" });
  stack.push({ level: "error", title: "Needs human", issueNumber: 43, message: "escalated" });

  const items = document.querySelectorAll(".autonomous-notifications__item");
  assert.equal(items.length, 2);
  // newest (43) is first
  assert.equal(items[0].dataset.level, "error");
  assert.match(items[0].textContent, /Needs human #43/);
  assert.match(items[0].textContent, /escalated/);
  assert.equal(items[1].dataset.level, "info");
  assert.match(items[1].textContent, /Launched #42/);
});

test("unknown level falls back to info", () => {
  const { document, stack } = setup();
  stack.push({ level: "bogus", title: "x", message: "y" });
  assert.equal(
    document.querySelector(".autonomous-notifications__item").dataset.level,
    "info",
  );
});

test("the stack is bounded and SCROLLS rather than growing unboundedly", () => {
  // FR-035: when many notices accumulate the retained count is capped and the
  // list container is scrollable (overflow-y: auto + a max-height).
  const { document, stack } = setup({ maxRetained: 5 });
  for (let i = 0; i < 20; i += 1) {
    stack.push({ level: "info", title: `n${i}`, message: "m" });
  }
  assert.equal(stack.count(), 5, "retained notices capped");
  assert.equal(stack.droppedCount(), 15, "overflow counted, not silently lost");
  assert.equal(
    document.querySelectorAll(".autonomous-notifications__item").length,
    5,
    "DOM does not grow unboundedly",
  );
  const style = stack.styleText();
  assert.match(style, /overflow-y:\s*auto/, "list scrolls on overflow");
  assert.match(style, /max-height:/, "list is height-bounded");
});

test("a dismiss button removes its notice", () => {
  const { document, stack } = setup();
  stack.push({ level: "info", title: "x", message: "y" });
  const dismiss = document.querySelector(".autonomous-notifications__dismiss");
  assert.ok(dismiss);
  dismiss.dispatchEvent(new document.defaultView.Event("click"));
  assert.equal(stack.count(), 0, "dismiss removes the notice");
});

test("styles use Operator tokens, never raw hex/rgb", () => {
  // AGENTS.md GUI rule: new UI CSS must use Operator Design System tokens.
  const style = createAutonomousNotifications({
    document: parseHTML("<!doctype html>").document,
  }).styleText();
  assert.doesNotMatch(style, /#[0-9a-fA-F]{3,8}\b/, "no raw hex colors");
  assert.doesNotMatch(style, /\brgba?\(/, "no raw rgb/rgba colors");
  assert.match(style, /var\(--color-/, "uses color tokens");
});

test("SPEC #3206 P2: every token the log-region style references is defined", () => {
  // A var() pointing at a token that exists nowhere silently falls back (or
  // no-ops), so the style sheet must only reference tokens that tokens.css /
  // typography.css actually define.
  const stylesDir = new URL("../styles/", import.meta.url);
  const defined = new Set();
  for (const file of ["tokens.css", "typography.css"]) {
    const source = readFileSync(new URL(file, stylesDir), "utf8");
    for (const m of source.matchAll(/(--[a-z0-9-]+)\s*:/g)) {
      defined.add(m[1]);
    }
  }
  const style = createAutonomousNotifications({
    document: parseHTML("<!doctype html>").document,
  }).styleText();
  for (const m of style.matchAll(/var\(\s*(--[a-z0-9-]+)/g)) {
    assert.ok(defined.has(m[1]), `log region references undefined token ${m[1]}`);
  }
});
