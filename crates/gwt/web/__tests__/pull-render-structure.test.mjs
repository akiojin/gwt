import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

const script = new URL('../../../../scripts/measure-pull-render-structure.mjs', import.meta.url);
test('structure measurement separates lexical sites, surface construction and receiver bypasses', async () => {
  assert.ok(existsSync(script), 'the structural measurement tool must exist');
  const { measureModules } = await import(script);
  const modules = new Map([
    ['crates/gwt/web/pm-chat.js', `import { createUiStateStore } from './ui-state-store.js';
      export function createPmWindowModel() { const model = createUiStateStore({});
        function applyPmWindowReceiveEvent(event) { model.update(() => ({ kind: event.kind })); }
        return { applyPmWindowReceiveEvent }; }
      export function createPmChat() { document.createElement('div'); }`],
    ['crates/gwt/web/app.js', `import { createPmWindowModel } from '/pm-chat.js';
      const { applyPmWindowReceiveEvent } = createPmWindowModel();
      function receive(event) { switch (event.kind) {
        case "pm_reports": applyPmWindowReceiveEvent(event); break;
        case "terminal_output": applyPmWindowReceiveEvent(event); break;
        case "terminal_snapshot": applyPmWindowReceiveEvent(event); break;
        case 9007199254740993n: applyPmWindowReceiveEvent(event); break; } }
      function syncPmChat() { document.createElement('div'); }
      function mountWindowBody() { body.innerHTML = ''; if (surface === "terminal") {
        body.innerHTML = '<div>'; const node = document.createElement('div');
        host.appendChild(node); node.textContent = 'RAW'; } }
      function createTerminalRuntime() { const terminal = new Terminal(); terminal.open(host); }
      function reparentTerminalRuntime() {} function writeOutput() {} function replaceTerminalSnapshot() {}
      // subscribe( fetch( are counted by the original lexical method too.`],
    ['crates/gwt/web/ui-content.js', `export function renderUiContent() {
      const node = document.createElement('div'); node.innerHTML = '<p>'; return node; }`],
    ['crates/gwt/web/other.js', `import { createUiStateStore } from './ui-state-store.js';
      const model = createUiStateStore({});
      const applyOtherReceiveEvent = (event) => {
        if (!event) return;
        switch (event.kind) { case 'other_state': model.update(() => ({ value: event.value })); }
      };`],
  ]);
  const result = measureModules(modules);
  assert.deepEqual(result.lexical_totals,
    { appendChild: 1, textContent_assignment: 1, subscribe: 1, fetch: 1 });
  assert.equal(result.lexical_files['crates/gwt/web/app.js'].appendChild, 1);
  assert.deepEqual(result.receiver_audit.findings, []);
  assert.equal(result.receiver_audit.bypass_probes.length, 3);
  assert.ok(result.receiver_audit.bypass_probes.every(probe => probe.detected));
  assert.equal(result.all_receiver_audit.scanned_modules, modules.size);
  assert.deepEqual(result.all_receiver_audit.findings, []);
  const receive = result.all_receiver_audit.roots.find(root => root.root === 'receive');
  assert.ok(receive, 'the root inventory must include function declarations');
  assert.doesNotThrow(() => JSON.stringify(result));
  assert.equal(receive.switch_labels.at(-1).value, '9007199254740993');
  const other = result.all_receiver_audit.roots.find(root => root.file.endsWith('/other.js'));
  assert.equal(other.root, 'applyOtherReceiveEvent');
  assert.equal(other.if_statements, 1);
  assert.deepEqual(other.switch_labels.map(label => label.value), ['other_state']);
  modules.set('crates/gwt/web/other.js', modules.get('crates/gwt/web/other.js')
    .replace('model.update(', 'document.body.textContent = event.value; model.update('));
  const bypass = measureModules(modules);
  assert.deepEqual(bypass.receiver_audit.findings, [], 'the initial pair audit cannot cover other surfaces');
  assert.ok(bypass.all_receiver_audit.findings.some(finding => finding.file.endsWith('/other.js')
    && finding.root === 'applyOtherReceiveEvent' && finding.kind === 'dom-write'));
  const terminal = result.dom_scopes.find(scope => scope.scope === 'mountWindowBody:terminal');
  assert.deepEqual(terminal.construction_sites.map(site => site.kind), ['innerHTML=', 'createElement']);
  assert.ok(terminal.dom_operation_sites.some(site => site.kind === 'appendChild'));
});

test('source reading excludes dirty tracked files and untracked modules', async t => {
  assert.ok(existsSync(script), 'the structural measurement CLI must exist');
  const { readSourceTree } = await import(script);
  const repository = mkdtempSync(join(tmpdir(), 'gwt-structure-'));
  t.after(() => rmSync(repository, { recursive: true, force: true }));
  const git = (...args) => execFileSync('git', args, { cwd: repository, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  git('init');
  mkdirSync(join(repository, 'crates/gwt/web'), { recursive: true });
  const file = join(repository, 'crates/gwt/web/app.js');
  writeFileSync(file, '// committed source\n');
  git('add', '.');
  git('-c', 'user.name=Test', '-c', 'user.email=test@example.com', 'commit', '-m', 'fixture');
  const sha = git('rev-parse', 'HEAD').trim();
  writeFileSync(file, '// dirty appendChild\n');
  writeFileSync(join(repository, 'crates/gwt/web/untracked.js'), '// fetch(\n');
  const result = readSourceTree('HEAD', repository);
  assert.equal(result.sha, sha);
  assert.deepEqual([...result.modules], [['crates/gwt/web/app.js', '// committed source\n']]);
});
