import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { scanModule, assertBaseline, checkRepository } from "../../../../scripts/check-pull-render.mjs";
const keys = findings => findings.map(finding => finding.key);
const receiverKeys = findings => [...new Set(findings.map(finding => JSON.stringify([finding.file, finding.root, finding.context])))];

test("receiver extraction survives strings, templates, and nested breaks", () => {
  const findings = scanModule(`function receive(event) {
    switch (event.kind) {
      case "snapshot": {
        const label = "break; } case ";
        const template = \`case fake: \${"}"}\`;
        for (const item of event.items) { if (!item) break; }
        node.textContent = event.body;
        break;
      }
    }
  }`, "fixture.js");
  assert.equal(findings.filter(finding => finding.kind === "dom-write").length, 1);
  assert.ok(keys(findings).some(key => key.includes("snapshot")));
});

test("receiver follows same-module helpers and diagnoses unresolved calls", () => {
  const findings = scanModule(`function receive(event) { paint(event); external.render(event); }
    function paint(event) { target.appendChild(event.node); }`, "fixture.js");
  assert.ok(findings.some(finding => finding.kind === "dom-call" && finding.path.includes("paint")));
  assert.ok(findings.some(finding => finding.kind === "unresolved-call"));
});

test("model updates, subscriptions, and uncalled local handlers are legal", () => {
  const findings = scanModule(`import { createUiStateStore } from "./ui-state-store.js";
    const model = createUiStateStore({ value: "" });
    model.subscribe(state => state.value, value => { element.textContent = value; });
    function receive(event) {
      function onClick() { element.textContent = "clicked"; }
      element.addEventListener("click", onClick);
      model.update(state => ({ ...state, value: event.value }));
    }`, "fixture.js");
  assert.deepEqual(findings, []);
});

test("factory return aliases resolve model handlers without trusting controller.update", () => {
  const module = `import { createUiStateStore } from "./ui-state-store.js";
    export function createSurface() {
      const model = createUiStateStore(null);
      model.subscribe(value => value, value => { element.textContent = value; });
      function applySnapshot(value) { model.update(() => value); }
      return { applySnapshot };
    }`;
  const source = `import { createSurface } from "./surface.js";
    const { applySnapshot: applyInbox } = createSurface();
    function receive(event) { applyInbox(event.value); }`;
  const modules = new Map([["surface.js", module]]);
  assert.deepEqual(scanModule(source, "entry.js", { modules }), []);
  modules.set("surface.js", module.replace("model.update(() => value)", "element.textContent = value"));
  assert.ok(scanModule(source, "entry.js", { modules }).some(finding => finding.kind === "dom-write"));
  assert.ok(scanModule("function receive(event) { view.controller.update(event.snapshot); }", "entry.js")
    .some(finding => finding.kind === "unresolved-call"));
  assert.ok(scanModule(`function createUiStateStore() { return external; }
    const model = createUiStateStore(); function receive(event) { model.update(event); }`, "entry.js")
    .some(finding => finding.kind === "unresolved-call"));
});

test("synchronous and scheduled callbacks retain the receiver boundary", () => {
  const findings = scanModule(`function receive(event) {
    event.items.forEach(item => { element.textContent = item; });
    setTimeout(() => { element.append(event.node); }, 100);
  }`, "fixture.js");
  assert.equal(findings.filter(finding => finding.kind.startsWith("dom-")).length, 2);
});

test("recursive helpers terminate even when nested switches change case context", () => {
  let findings;
  assert.doesNotThrow(() => {
    findings = scanModule(`function receive(event) { paint(event); }
      function paint(event) { switch (event.mode) { case "nested": paint(event); break; }
        element.textContent = event.value; }`, "fixture.js");
  });
  assert.equal(findings.filter(finding => finding.kind === "dom-write").length, 1);
});

test("new direct, indirect, or unresolved paths cannot expand an allowed root", () => {
  const old = scanModule(`function receive(event) { element.textContent = event.body; }`, "fixture.js");
  for (const extra of ["element.append(event.node);", "paint(event);", "external.update(event);"]) {
    const next = scanModule(`function receive(event) { element.textContent = event.body; ${extra} }
      function paint(event) { element.innerHTML = event.body; }`, "fixture.js");
    assert.throws(() => assertBaseline(next, receiverKeys(old), receiverKeys(old), old), /new finding/i);
  }
  const newRoot = scanModule(`function receive(event) { element.textContent = event.body; }
    function applyNewReceiveEvent(event) { element.append(event.node); }`, "fixture.js");
  assert.throws(() => assertBaseline(newRoot, receiverKeys(old), receiverKeys(old), old), /new finding/i);
});

test("baseline cannot grow and first introduction is bounded by base source findings", () => {
  const old = scanModule(`function receive(event) { element.textContent = event.body; }`, "fixture.js");
  const next = scanModule(`function receive(event) { element.textContent = event.body; external.update(event); }`, "fixture.js");
  assert.throws(() => assertBaseline(next, receiverKeys(next), receiverKeys(old), old), /new finding/i);
  const expanded = scanModule(`function receive(event) { element.textContent = event.body; }
    function applyExtraReceiveEvent(event) { element.innerHTML = event.body; }`, "fixture.js");
  assert.throws(() => assertBaseline(expanded, receiverKeys(expanded), receiverKeys(old), old), /baseline.*grow/i);
  assert.doesNotThrow(() => assertBaseline(old, receiverKeys(old), receiverKeys(old), old));
  assert.throws(() => assertBaseline([], receiverKeys(old), receiverKeys(old), old), /stale/i);
});

test("migrated Issue Monitor and Usage receiver roots contain no DOM sinks", () => {
  for (const file of ["knowledge-kanban-surface.js", "provider-usage-surface.js"]) {
    const source = readFileSync(new URL(`../${file}`, import.meta.url), "utf8");
    const migrated = scanModule(source, file).filter(finding =>
      /^(applyIssueMonitorStatus|applyIssueMonitorInbox|applyProviderUsageUi)$/.test(finding.root)
      && finding.kind.startsWith("dom-"));
    assert.deepEqual(migrated, [], file);
  }
});

test("checkout baseline satisfies the live and Git base contracts", () => {
  checkRepository();
});
