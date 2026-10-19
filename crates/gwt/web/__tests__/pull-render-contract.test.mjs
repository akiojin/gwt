import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { parseHTML } from "linkedom";
import { scanModule, assertBaseline, checkRepository } from "../../../../scripts/check-pull-render.mjs";
const keys = findings => findings.map(finding => finding.key);
const receiverKeys = findings => [...new Set(findings.map(finding => JSON.stringify([finding.file, finding.root, finding.context])))];

test("Open Project result receiver commits shared state and rejects bypass rendering", () => {
  const file = 'crates/gwt/web/project-clone-modal.js';
  const source = readFileSync(new URL('../project-clone-modal.js', import.meta.url), 'utf8');
  const findings = code => scanModule(code, file)
    .filter(finding => finding.root === 'applyOpenProjectPathResultReceiveEvent');
  assert.match(source, /receive: applyOpenProjectPathResultReceiveEvent/);
  assert.deepEqual(findings(source), []);
  const bypass = source.replace('function applyOpenProjectPathResultReceiveEvent(event) {',
    'function applyOpenProjectPathResultReceiveEvent(event) { status.textContent = event.message;');
  assert.notEqual(bypass, source);
  assert.ok(findings(bypass).length > 0);
});

test("Clone Project receivers commit shared state and reject bypass rendering", () => {
  const file = 'crates/gwt/web/project-shell-surface.js';
  const source = readFileSync(new URL('../project-shell-surface.js', import.meta.url), 'utf8');
  const modules = new Map([['crates/gwt/web/project-clone-modal.js', readFileSync(new URL('../project-clone-modal.js', import.meta.url), 'utf8')]]);
  const findings = code => scanModule(code, file, { modules })
    .filter(finding => finding.root === 'applyCloneProjectReceiveEvent');
  assert.deepEqual(findings(source), []);
  for (const kind of ['clone_project_parent_selected', 'github_repository_search_results', 'github_repository_search_error', 'clone_project_progress', 'clone_project_done', 'clone_project_error']) {
    const bypass = source.replace(`case "${kind}":`,
      `case "${kind}": renderProjectCloneModal();`);
    assert.notEqual(bypass, source);
    assert.ok(findings(bypass).length > 0);
  }
});

test("Migration receivers commit shared state and reject bypass rendering", () => {
  const file = 'crates/gwt/web/project-shell-surface.js';
  const source = readFileSync(new URL('../project-shell-surface.js', import.meta.url), 'utf8');
  const modules = new Map([['crates/gwt/web/migration-modal.js', readFileSync(new URL('../migration-modal.js', import.meta.url), 'utf8')]]);
  const findings = code => scanModule(code, file, { modules })
    .filter(finding => finding.root === 'applyMigrationReceiveEvent');
  assert.deepEqual(findings(source), []);
  for (const kind of ['migration_detected', 'migration_progress', 'migration_done', 'migration_error']) {
    const bypass = source.replace(`case "${kind}":`,
      `case "${kind}": renderMigrationModal();`);
    assert.notEqual(bypass, source);
    assert.ok(findings(bypass).length > 0);
  }
});

test("Update availability and auto-apply receivers commit shared state and reject bypass rendering", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const update = readFileSync(new URL('../update-cta.js', import.meta.url), 'utf8');
  const findings = source => scanModule(source, 'crates/gwt/web/app.js', {
    modules: new Map([['crates/gwt/web/update-cta.js', update]]),
  }).filter(finding => finding.root === 'receive'
    && ['/update_state', '/update_auto_apply'].includes(finding.context));
  assert.deepEqual(findings(app), []);
  for (const kind of ['update_state', 'update_auto_apply']) {
    const bypass = app.replace(`case "${kind}":`,
      `case "${kind}": document.body.textContent = event.version;`);
    assert.notEqual(bypass, app);
    assert.ok(findings(bypass).length > 0);
  }
});

test("Update staged-ready receivers commit shared state and reject bypass rendering", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const update = readFileSync(new URL('../update-cta.js', import.meta.url), 'utf8');
  const findings = source => scanModule(source, 'crates/gwt/web/app.js', {
    modules: new Map([['crates/gwt/web/update-cta.js', update]]),
  }).filter(finding => finding.root === 'receive'
    && ['/update_ready', '/update_apply_pending_persisted'].includes(finding.context));
  assert.deepEqual(findings(app), []);
  for (const kind of ['update_ready', 'update_apply_pending_persisted']) {
    const bypass = app.replace(`case "${kind}":`,
      `case "${kind}": document.body.textContent = event.version;`);
    assert.notEqual(bypass, app);
    assert.ok(findings(bypass).length > 0);
  }
});

test("Update apply error receiver commits shared state and rejects bypass rendering", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const update = readFileSync(new URL('../update-cta.js', import.meta.url), 'utf8');
  const findings = source => scanModule(source, 'crates/gwt/web/app.js', {
    modules: new Map([['crates/gwt/web/update-cta.js', update]]),
  }).filter(finding => finding.root === 'receive' && finding.context === '/update_apply_error');
  assert.deepEqual(findings(app), []);
  const bypass = app.replace('case "update_apply_error":',
    'case "update_apply_error": document.body.textContent = event.reason;');
  assert.notEqual(bypass, app);
  assert.ok(findings(bypass).length > 0);
});

test("Update progress receiver commits model data and rejects bypass rendering", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const update = readFileSync(new URL('../update-cta.js', import.meta.url), 'utf8');
  const findings = source => scanModule(source, 'crates/gwt/web/app.js', {
    modules: new Map([['crates/gwt/web/update-cta.js', update]]),
  }).filter(finding => finding.root === 'receive' && finding.context === '/update_progress');
  assert.deepEqual(findings(app), []);
  const bypass = app.replace('case "update_progress":',
    'case "update_progress": document.body.textContent = event.downloaded;');
  assert.notEqual(bypass, app);
  assert.ok(findings(bypass).length > 0);
});

test("Project metadata receiver commits shared state and rejects bypass rendering", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const metadata = readFileSync(new URL('../project-page-metadata.js', import.meta.url), 'utf8');
  const findings = source => scanModule(source, 'crates/gwt/web/app.js', {
    modules: new Map([['crates/gwt/web/project-page-metadata.js', metadata]]),
  }).filter(finding => finding.root === 'receive' && finding.context === '/project_agent_aggregate');
  assert.deepEqual(findings(app), []);
  const bypass = app.replace('case "project_agent_aggregate":',
    'case "project_agent_aggregate": document.body.textContent = event.aggregate.revision;');
  assert.notEqual(bypass, app);
  assert.ok(findings(bypass).length > 0);
});

test("Runtime health receiver updates shared state and rejects bypass rendering", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const findings = source => scanModule(source, 'crates/gwt/web/app.js')
    .filter(finding => finding.root === 'receive' && finding.context === '/runtime_health');
  assert.deepEqual(findings(app), []);
  const bypass = app.replace('case "runtime_health":',
    'case "runtime_health": document.body.textContent = event.snapshot.state;');
  assert.notEqual(bypass, app);
  assert.ok(findings(bypass).length > 0);
});

test("System Settings receivers update the shared model and reject bypass rendering", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const surface = readFileSync(new URL('../settings-surface.js', import.meta.url), 'utf8');
  const findings = source => scanModule(source, 'crates/gwt/web/app.js', {
    modules: new Map([['crates/gwt/web/settings-surface.js', surface]]),
  }).filter(finding => finding.root === 'receive' && /^\/system_settings(?:\|system_settings_updated|_updated|_error)?$/.test(finding.context));
  assert.deepEqual(findings(app), []);
  const bypass = app.replace('case "system_settings_updated":',
    'case "system_settings_updated": document.body.textContent = event.language;');
  assert.notEqual(bypass, app);
  assert.ok(findings(bypass).length > 0);
});

test("Board auth receiver commits shared state and rejects bypass rendering", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const surface = readFileSync(new URL('../settings-surface.js', import.meta.url), 'utf8');
  const findings = source => scanModule(source, 'crates/gwt/web/app.js', {
    modules: new Map([['crates/gwt/web/settings-surface.js', surface]]),
  }).filter(finding => finding.root === 'receive' && finding.context === '/board_auth_status');
  assert.deepEqual(findings(app), [], 'Board auth replies update the shared model without drawing');
  const bypass = app.replace('case "board_auth_status":',
    'case "board_auth_status": document.body.textContent = event.message;');
  assert.notEqual(bypass, app);
  assert.ok(findings(bypass).length > 0);
});

test("Agent Backend receivers commit model data and reject bypass rendering", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const settings = readFileSync(new URL('../settings-surface.js', import.meta.url), 'utf8');
  const findings = source => scanModule(source, 'crates/gwt/web/app.js', {
    modules: new Map([['crates/gwt/web/settings-surface.js', settings]]),
  }).filter(finding => finding.root === 'receive'
    && /\/agent_backend_(list|saved|deleted|error)(\||$)/.test(finding.context));
  assert.deepEqual(findings(app), []);
  const bypass = app.replace('case "agent_backend_list":',
    'case "agent_backend_list": document.body.textContent = event.agent;');
  assert.notEqual(bypass, app);
  assert.ok(findings(bypass).length > 0);
});

test("Custom Agent receivers only commit model data and reject bypass rendering", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const settings = readFileSync(new URL('../settings-surface.js', import.meta.url), 'utf8');
  const findings = source => scanModule(source, 'crates/gwt/web/app.js', {
    modules: new Map([['crates/gwt/web/settings-surface.js', settings]]),
  }).filter(finding => finding.root === 'receive'
    && /\/(custom_agent_(list|saved|deleted|error)|backend_connection_result)(\||$)/.test(finding.context));
  assert.deepEqual(findings(app), []);
  const bypass = app.replace('applyCustomAgentsReceiveEvent(event);',
    'applyCustomAgentsReceiveEvent(event); document.body.textContent = event.message;');
  assert.notEqual(bypass, app);
  assert.ok(findings(bypass).length > 0);
});

test("Autostart receivers commit shared state and reject bypass rendering", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const settings = readFileSync(new URL('../settings-surface.js', import.meta.url), 'utf8');
  const findings = source => scanModule(source, 'crates/gwt/web/app.js', {
    modules: new Map([['crates/gwt/web/settings-surface.js', settings]]),
  }).filter(finding => finding.root === 'receive' && /^\/autostart_(status|error)$/.test(finding.context));
  assert.deepEqual(findings(app), []);
  const bypass = app.replace('applyAutostartError(event);',
    'applyAutostartError(event); document.body.textContent = event.message;');
  assert.notEqual(bypass, app);
  assert.ok(findings(bypass).length > 0);
});

test("Monitor notification receiver only commits model data and rejects direct history rendering", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const monitorFindings = source => scanModule(source, 'crates/gwt/web/app.js')
    .filter(finding => finding.root === 'receive' && finding.context === '/issue_monitor_toast');
  assert.deepEqual(monitorFindings(app), []);
  const bypass = app.replace('issueMonitorNotificationModel.update(() => ({ ...event }));',
    'notificationCenter.record(event);');
  assert.notEqual(bypass, app);
  assert.ok(monitorFindings(bypass).length > 0);
});

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

test("subscribing inside a receiver includes the immediately rendered view", () => {
  const findings = scanModule(`import { createUiStateStore } from "./ui-state-store.js";
    const model = createUiStateStore({ value: "" });
    function receive(event) {
      model.subscribe(state => state.value, value => { element.textContent = value; });
    }`, "fixture.js");
  assert.equal(findings.filter(finding => finding.kind === "dom-write").length, 1);
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

test("Reports and raw terminal receivers resolve only shared model updates", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const file = 'crates/gwt/web/app.js';
  const modules = new Map([['crates/gwt/web/pm-chat.js', readFileSync(new URL('../pm-chat.js', import.meta.url), 'utf8')]]);
  const migrated = findings => findings.filter(finding => finding.root === 'receive'
    && /^\/(pm_reports|terminal_output|terminal_snapshot)$/.test(finding.context));
  assert.deepEqual(migrated(scanModule(app, file, { modules })), []);
  const bypass = app.replace('applyPmWindowReceiveEvent(event);', 'applyPmWindowReceiveEvent(event); element.textContent = event.kind;');
  assert.ok(migrated(scanModule(bypass, file, { modules })).some(finding => finding.kind === 'dom-write'));
});

test("Index health receiver resolves only shared model updates", () => {
  const file = 'crates/gwt/web/project-index-search-surface.js';
  const source = readFileSync(new URL('../project-index-search-surface.js', import.meta.url), 'utf8');
  assert.deepEqual(scanModule(source, file, { rootName: 'setIndexStatus' }), []);
  const bypass = source.replace('function setIndexStatus(projectRoot, status) {',
    'function setIndexStatus(projectRoot, status) { renderProjectIndexWindows();');
  assert.notEqual(bypass, source);
  assert.ok(scanModule(bypass, file, { rootName: 'setIndexStatus' })
    .some(finding => finding.kind.startsWith('dom-')));
});

test("Index search receivers resolve only shared model updates", () => {
  const file = 'crates/gwt/web/project-index-search-surface.js';
  const source = readFileSync(new URL('../project-index-search-surface.js', import.meta.url), 'utf8');
  for (const name of ['handleProjectIndexSearchResults', 'handleProjectIndexSearchError']) {
    assert.deepEqual(scanModule(source, file, { rootName: name }), []);
    const bypass = source.replace(`function ${name}(event) {`,
      `function ${name}(event) { renderProjectIndexSearch(event.id);`);
    assert.notEqual(bypass, source);
    assert.ok(scanModule(bypass, file, { rootName: name }).some(finding => finding.kind.startsWith('dom-')));
  }
});

test("Release notes receivers resolve only shared model updates", () => {
  const appFile = 'crates/gwt/web/app.js';
  const surfaceFile = 'crates/gwt/web/release-notes-window.js';
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const surface = readFileSync(new URL('../release-notes-window.js', import.meta.url), 'utf8');
  const receivers = source => scanModule(source, appFile, { modules: new Map([[surfaceFile, surface]]) })
    .filter(finding => finding.root === 'receive' && /^\/release_notes_(payload|error)$/.test(finding.context));
  assert.deepEqual(receivers(app), []);
  for (const name of ['handlePayload', 'handleError']) {
    assert.deepEqual(scanModule(surface, surfaceFile, { rootName: name }), []);
    const bypass = surface.replace(name === 'handlePayload' ? 'function handlePayload(payload) {' : 'function handleError(message) {',
        `function ${name}(${name === 'handlePayload' ? 'payload' : 'message'}) { renderSidebar(model.read());`);
    assert.notEqual(bypass, surface);
    assert.ok(scanModule(bypass, surfaceFile, { rootName: name }).some(finding => finding.kind.startsWith('dom-')));
  }
});

test("Workspace state notice receiver resolves only shared model updates", () => {
  const appFile = 'crates/gwt/web/app.js';
  const noticeFile = 'crates/gwt/web/workspace-state-notice.js';
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const notice = readFileSync(new URL('../workspace-state-notice.js', import.meta.url), 'utf8');
  const receiverFindings = source => scanModule(source, appFile, { modules: new Map([[noticeFile, notice]]) })
    .filter(finding => finding.root === 'receive' && finding.context === '/workspace_state_notice');
  assert.deepEqual(scanModule(notice, noticeFile), []);
  assert.deepEqual(receiverFindings(app), []);
  const bypass = app.replace('case "workspace_state_notice":',
    'case "workspace_state_notice": element.textContent = event.kind;');
  assert.ok(receiverFindings(bypass).some(finding => finding.kind === 'dom-write'));
  const directRender = notice.replace('function receive(notice) {',
    'function receive(notice) { render(notice);');
  assert.notEqual(directRender, notice, 'mutation must target the real notice receiver');
  assert.ok(scanModule(directRender, noticeFile).some(finding => finding.kind.startsWith('dom-')));
});

test("UI trace feedback receivers resolve only shared model updates", () => {
  const appFile = 'crates/gwt/web/app.js';
  const traceFile = 'crates/gwt/web/ui-trace-wiring.js';
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const trace = readFileSync(new URL('../ui-trace-wiring.js', import.meta.url), 'utf8');
  const receivers = source => scanModule(source, appFile, { modules: new Map([[traceFile, trace]]) })
    .filter(finding => finding.root === 'receive' && /^\/ui_trace_(saved|error)(\||\/|$)/.test(finding.context));
  assert.deepEqual(receivers(app), []);
  const bypass = app.replace('case "ui_trace_saved":',
    'case "ui_trace_saved": window.alert(event.path);');
  assert.notEqual(bypass, app);
  assert.ok(receivers(bypass).length > 0);
  const directRender = trace.replace('function receive(event) {',
    'function receive(event) { alert(event.message);');
  assert.notEqual(directRender, trace);
  assert.ok(scanModule(directRender, traceFile).length > 0);
});

test("PM settings receiver resolves only shared model updates", () => {
  const appFile = 'crates/gwt/web/app.js';
  const panelFile = 'crates/gwt/web/pm-settings-panel.js';
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const panel = readFileSync(new URL('../pm-settings-panel.js', import.meta.url), 'utf8');
  const modules = new Map([[panelFile, panel]]);
  const receiverFindings = source => scanModule(source, appFile, { modules })
    .filter(finding => finding.root === 'receive' && /^\/pm_status(\/|$)/.test(finding.context));
  assert.deepEqual(receiverFindings(app), []);
  const bypass = app.replace('applyPmSettingsReceiveEvent(event);',
    'applyPmSettingsReceiveEvent(event); element.textContent = event.kind;');
  assert.notEqual(bypass, app, 'mutation must target the real PM settings receiver');
  assert.ok(receiverFindings(bypass).some(finding => finding.kind === 'dom-write'));

  const directRender = panel.replace('function applyPmSettingsReceiveEvent(status) {',
    'function applyPmSettingsReceiveEvent(status) { renderView(mounts.values().next().value, model.read());');
  assert.notEqual(directRender, panel, 'mutation must target the factory receiver');
  assert.ok(scanModule(directRender, panelFile, { rootName: 'applyPmSettingsReceiveEvent' })
    .some(finding => finding.kind.startsWith('dom-')));
});

test("Profile receivers resolve only shared model updates", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const appFile = 'crates/gwt/web/app.js';
  const profileFile = 'crates/gwt/web/profile-window-surface.js';
  const profile = readFileSync(new URL('../profile-window-surface.js', import.meta.url), 'utf8');
  const modules = new Map([[profileFile, profile]]);
  const profileReceivers = findings => findings.filter(finding => finding.root === 'receive'
    && /^\/profile_(snapshot|error)(\/|$)/.test(finding.context));
  const moduleFindings = scanModule(profile, profileFile, { rootName: 'applyProfileReceiveEvent' });
  assert.equal(moduleFindings.length, 0, 'Profile module receiver must contain no DOM or unresolved calls');
  assert.deepEqual(profileReceivers(scanModule(app, appFile, { modules })), []);

  const directRender = profile.replace('function applyProfileReceiveEvent(event) {',
    'function applyProfileReceiveEvent(event) { renderProfile(event.id);');
  assert.notEqual(directRender, profile, 'the direct-render mutation must target the real receiver');
  assert.ok(scanModule(directRender, profileFile, { rootName: 'applyProfileReceiveEvent' })
    .some(finding => finding.kind.startsWith('dom-')));
  assert.ok(profileReceivers(scanModule(app, appFile, { modules: new Map([[profileFile, directRender]]) }))
    .some(finding => finding.kind.startsWith('dom-')), 'factory tracing must detect the old rendering path');
  const bypass = app.replace('applyProfileReceiveEvent(event);',
    'applyProfileReceiveEvent(event); element.textContent = event.kind;');
  assert.ok(profileReceivers(scanModule(bypass, appFile, { modules }))
    .some(finding => finding.kind === 'dom-write'));
});

test("Profile disposal cancels pending saves and remounts without closed-window data", async t => {
  const source = readFileSync(new URL('../profile-window-surface.js', import.meta.url), 'utf8')
    .replace('"/ui-state-store.js"', JSON.stringify(new URL('../ui-state-store.js', import.meta.url).href));
  const { createProfileWindowSurface } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
  const { document, window } = parseHTML('<html><body></body></html>');
  const previousDocument = globalThis.document;
  globalThis.document = document;
  t.after(() => { globalThis.document = previousDocument; });
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const sends = [], windowMap = new Map();
  const surface = createProfileWindowSurface({
    send: message => sends.push(message),
    createNode: (tag, className, text = '') => {
      const node = document.createElement(tag);
      node.className = className;
      node.textContent = text;
      return node;
    },
    windowMap,
    focusWindowLocally() {},
    sendWindowFocus() {},
  });
  const mount = () => {
    const pane = document.createElement('div');
    pane.innerHTML = '<div class="window-body"></div>';
    windowMap.set('profile', pane);
    surface.mountProfileWindow({ id: 'profile' }, pane.firstChild);
    return pane;
  };
  const snapshot = {
    active_profile: 'default', selected_profile: 'default', os_env: [],
    profiles: [{ name: 'default', description: 'Original', env_vars: [], disabled_env: [] }],
  };
  const pane = mount();
  surface.applyProfileReceiveEvent({ kind: 'profile_snapshot', id: 'profile', snapshot });
  const input = pane.querySelector('.profile-field input');
  input.value = 'pending';
  input.dispatchEvent(new window.Event('input'));
  surface.disposeProfileWindow('profile');
  windowMap.delete('profile');
  t.mock.timers.tick(1000);
  assert.equal(sends.filter(message => message.kind === 'save_profile').length, 0);
  assert.equal(surface.profileStateMap.has('profile'), false);
  surface.applyProfileReceiveEvent({ kind: 'profile_snapshot', id: 'profile', snapshot });
  surface.applyProfileReceiveEvent({ kind: 'profile_error', id: 'profile', message: 'late error' });
  assert.equal(surface.profileStateMap.has('profile'), false);
  const remounted = mount();
  assert.equal(sends.filter(message => message.kind === 'load_profile').length, 2);
  assert.equal(remounted.querySelector('.profile-field input'), null);
  surface.applyProfileReceiveEvent({ kind: 'profile_snapshot', id: 'profile', snapshot });
  assert.equal(remounted.querySelector('.profile-field input').value, 'default');
  assert.match(remounted.querySelector('.profile-status').textContent, /^Active /);
  surface.disposeProfileWindow('profile');
});

test("Window List receiver updates the catalog model and rejects direct rendering bypass", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const file = 'crates/gwt/web/app.js';
  const modules = new Map([['crates/gwt/web/project-shell-surface.js', readFileSync(new URL('../project-shell-surface.js', import.meta.url), 'utf8')]]);
  const migrated = findings => findings.filter(finding => finding.root === 'receive' && finding.context === '/window_list');
  assert.deepEqual(migrated(scanModule(app, file, { modules })), []);
  const bypass = app.replace('case "window_list":', 'case "window_list": element.textContent = event.kind;');
  assert.ok(migrated(scanModule(bypass, file, { modules })).some(finding => finding.kind === 'dom-write'));
});

test("workspace receivers cannot manually sync Agents or Split outside the catalog", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const workspace = app.slice(app.indexOf('function renderWorkspace('), app.indexOf('const socketTransport'));
  assert.doesNotMatch(workspace, /(?:splitSurfaces|agentsSurface)\??\.sync\s*\(/);
  const bypasses = source => {
    const lines = source.split('\n');
    return scanModule(source, 'crates/gwt/web/app.js').filter(finding =>
      /(?:splitSurfaces|agentsSurface).*\.sync\s*\(/.test(lines[finding.line - 1]));
  };
  assert.deepEqual(bypasses(app), []);
  const bypass = app.replace('case "workspace_state":',
    'case "workspace_state": splitSurfaces.sync(); agentsSurface.sync(event.windows);');
  assert.ok(bypasses(bypass).length > 0, "the old fan-out must be detected even inside an allowed receiver");
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


test("cleanup progress/result receivers commit shared data and reject direct rendering", () => {
  const appFile = 'crates/gwt/web/app.js';
  const surfaceFile = 'crates/gwt/web/branches-cleanup-surface.js';
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const surface = readFileSync(new URL('../branches-cleanup-surface.js', import.meta.url), 'utf8');
  const modules = new Map([[surfaceFile, surface]]);
  const selected = findings => findings.filter(finding =>
    /\/branch_cleanup_(progress|result)$/.test(finding.context));
  assert.deepEqual(selected(scanModule(surface, surfaceFile, { rootName: 'applyBranchCleanupReceiveEvent' })), []);
  assert.deepEqual(selected(scanModule(app, appFile, { modules })), []);
  const bypass = surface.replace('case "branch_cleanup_result": {',
    'case "branch_cleanup_result": { renderBranches(event.id);');
  assert.notEqual(bypass, surface);
  assert.ok(selected(scanModule(bypass, surfaceFile, { rootName: 'applyBranchCleanupReceiveEvent' }))
    .some(finding => finding.kind.startsWith('dom-')));
});


test("Close Project receiver commits shared data and rejects direct rendering", () => {
  const file = 'crates/gwt/web/close-project-confirm-modal.js';
  const source = readFileSync(new URL('../close-project-confirm-modal.js', import.meta.url), 'utf8');
  assert.deepEqual(scanModule(source, file), []);
  const bypass = source.replace("if (event.kind === 'project_closed') {",
    "if (event.kind === 'project_closed') { hide();");
  assert.notEqual(bypass, source);
  assert.ok(scanModule(bypass, file).some(finding => finding.kind.startsWith('dom-')));
});


test("folder picker receiver commits shared state and rejects direct rendering", () => {
  const file = 'crates/gwt/web/project-clone-modal.js';
  const source = readFileSync(new URL('../project-clone-modal.js', import.meta.url), 'utf8');
  assert.match(source, /receive: applyFolderPickerReceiveEvent/);
  const options = { rootName: 'applyFolderPickerReceiveEvent' };
  assert.deepEqual(scanModule(source, file, options), []);
  const bypass = source.replace('function applyFolderPickerReceiveEvent(event) {',
    'function applyFolderPickerReceiveEvent(event) { element.textContent = event.message;');
  assert.notEqual(bypass, source);
  assert.ok(scanModule(bypass, file, options).some(finding => finding.kind === 'dom-write'));
});
