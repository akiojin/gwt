import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
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

test("unrelated remote advances leave the default comparison stable; explicit CI bases still apply", t => {
  const root = mkdtempSync(join(tmpdir(), "gwt-pull-render-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const directory of ["scripts", "crates/gwt/web", "node_modules"]) mkdirSync(join(root, directory), { recursive: true });
  copyFileSync(new URL("../../../../scripts/check-pull-render.mjs", import.meta.url), join(root, "scripts/check-pull-render.mjs"));
  symlinkSync(dirname(dirname(fileURLToPath(import.meta.resolve("acorn")))), join(root, "node_modules/acorn"), "junction");
  const file = join(root, "crates/gwt/web/fixture.js");
  const original = "function receive(event) { external.send(event.value); }";
  writeFileSync(file, original);
  writeFileSync(join(root, "scripts/pull-render-baseline.json"), JSON.stringify(receiverKeys(scanModule(original, "crates/gwt/web/fixture.js"))));
  const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  git("init", "-q");
  git("config", "user.name", "Guard fixture");
  git("config", "user.email", "guard@example.invalid");
  git("config", "core.hooksPath", join(root, ".git/hooks"));
  git("add", "scripts", "crates");
  git("commit", "-qm", "base");
  const common = git("rev-parse", "HEAD");
  git("update-ref", "refs/remotes/origin/develop", common);
  writeFileSync(join(root, "README.md"), "Feature branch\n");
  git("add", "README.md");
  git("commit", "-qm", "feature");
  const head = git("rev-parse", "HEAD");
  const check = base => {
    const env = { ...process.env };
    delete env.GWT_PULL_RENDER_BASE_SHA;
    if (base) env.GWT_PULL_RENDER_BASE_SHA = base;
    return JSON.parse(execFileSync(process.execPath, ["scripts/check-pull-render.mjs"],
      { cwd: root, env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }));
  };
  const before = check();
  // Simulate an unrelated upstream send-protocol change without moving HEAD.
  writeFileSync(file, original.replace("event.value", "event.value, event.request_id"));
  git("add", "crates");
  const advanced = git("commit-tree", git("write-tree"), "-p", common, "-m", "upstream protocol");
  git("restore", "--staged", "--worktree", "crates");
  git("update-ref", "refs/remotes/origin/develop", advanced);
  assert.equal(git("rev-parse", "HEAD"), head);
  assert.deepEqual(check(), before);
  assert.equal(check().base, common, "default comparison reports its immutable merge-base SHA");
  assert.equal(check(common).base, common);
  assert.throws(() => check("origin/develop"), /new finding/, "an explicit CI base remains authoritative");
  writeFileSync(file, original.replace("external.send(event.value);", "external.send(event.value); element.textContent = event.value;"));
  assert.throws(() => check(), /new finding/, "a stable base must still reject new DOM paths");
});

test("checkout baseline satisfies the live and Git base contracts", () => {
  checkRepository();
});
