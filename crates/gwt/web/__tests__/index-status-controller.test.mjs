// SPEC-1939 Phase 15 — project-bar Index badge withdrawn. The remaining
// coverage keeps the dedicated Index window and project-tab separation
// contract stable.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const indexHtml = readFileSync(resolve(here, "../index.html"), "utf8");
const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
// SPEC-3064 Phase 3 (E3): the Index window search/health surface moved out
// of app.js into a dedicated module; assertions about the moved code read
// the module source while receive()/Settings wiring stays in app.js.
const indexSurfaceSource = readFileSync(
  resolve(here, "../project-index-search-surface.js"),
  "utf8",
);
// SPEC-3064 Phase 3 (E4): the Settings window renderer, the settings:open
// dispatch listener, and requestFullIndexStatusRefresh moved into the
// extracted settings surface module.
const settingsSurfaceSource = readFileSync(
  resolve(here, "../settings-surface.js"),
  "utf8",
);

async function indexHealthFixture(t, onSettings = () => {}) {
  const source = indexSurfaceSource.replace(/from "\/(.*?)"/g,
    (_, name) => `from "${new URL(`../${name}`, import.meta.url).href}"`);
  const { createProjectIndexSearchSurface } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
  const { document } = parseHTML('<html><body><div data-id="index-1"><div id="body"></div></div></body></html>');
  const previous = { document: globalThis.document, CSS: globalThis.CSS };
  const selectPrototype = document.defaultView.HTMLSelectElement.prototype;
  const previousValue = Object.getOwnPropertyDescriptor(selectPrototype, 'value');
  globalThis.document = document;
  globalThis.CSS = { escape: value => value };
  // linkedom's select.value is getter-only; the browser supports assignment.
  Object.defineProperty(selectPrototype, 'value', {
    configurable: true, get() { return this.getAttribute('value') || ''; },
    set(value) { this.setAttribute('value', value); },
  });
  t.after(() => {
    Object.assign(globalThis, previous);
    Object.defineProperty(selectPrototype, 'value', previousValue);
  });
  const makeEl = (tag, options = {}) => {
    const node = document.createElement(tag);
    node.className = options.className || '';
    if (options.text) node.textContent = options.text;
    for (const [key, value] of Object.entries(options.attrs || {})) node.setAttribute(key, value);
    Object.assign(node.dataset, options.dataset);
    return node;
  };
  let activeRoot = '/repo';
  const sent = [];
  const controller = createProjectIndexSearchSurface({
    send: message => sent.push(message), sendWindowFocus: () => {}, focusWindowLocally: () => {},
    activeProjectTab: () => ({ project_root: activeRoot }), makeEl,
    clearChildren: node => node.replaceChildren(),
    renderIndexPanelInAllSettingsWindows: onSettings,
    requestFullIndexStatusRefresh: () => sent.push({ kind: 'refresh_index_status' }),
  });
  controller.mountProjectIndexSurface(document.getElementById('body'), { id: 'index-1' });
  return { controller, document, sent, activateProject: root => {
    activeRoot = root;
    controller.mountProjectIndexSurface(document.getElementById('body'), { id: 'index-1' });
  } };
}

test("Index health commits an immutable project snapshot and ignores unrelated project updates", async t => {
  let settingsRenders = 0;
  const { controller, document, activateProject } = await indexHealthFixture(t, () => { settingsRenders += 1; });
  const payload = { state: 'ready', scopes: { issues: { healthy: true, document_count: 12 } } };
  controller.setIndexStatus('/repo', payload);
  const committed = controller.getIndexStatus('/repo');
  assert.ok(Object.isFrozen(committed.scopes.issues));
  payload.scopes.issues.document_count = 99;
  assert.equal(committed.scopes.issues.document_count, 12);
  assert.match(document.querySelector('.index-search-health-summary').textContent, /1 ready/);
  controller.setIndexStatus('/other', { state: 'error' });
  assert.equal(settingsRenders, 1);
  assert.strictEqual(controller.getIndexStatus('/repo'), committed);
  activateProject('/other');
  const summary = document.querySelector('.index-search-health-summary').firstChild;
  controller.setIndexStatus('/repo', { state: 'repair_required' });
  assert.equal(settingsRenders, 1, 'switching projects must not turn an unrelated update into a redraw');
  assert.strictEqual(document.querySelector('.index-search-health-summary').firstChild, summary);
});

test("Index search renders committed responses, recovers lost reads and clears closed-window subscriptions", async t => {
  const { controller, document, sent } = await indexHealthFixture(t);
  const submit = query => {
    const input = document.querySelector('.index-search-input');
    input.value = query;
    input.dispatchEvent(new document.defaultView.Event('input'));
    document.querySelector('.index-search-box').dispatchEvent(new document.defaultView.Event('submit'));
    return sent.findLast(message => message.kind === 'search_project_index');
  };
  const request = submit('first');
  const payload = { id: 'index-1', request_id: request.request_id,
    results: [{ title: 'Committed', preview: 'Original preview', target: { kind: 'issue', number: 5016 } }],
    suggestions: [] };
  controller.handleProjectIndexSearchResults(payload);
  payload.results[0].title = 'Mutated';
  payload.results[0].preview = 'Mutated preview';
  controller.mountProjectIndexSurface(document.getElementById('body'), { id: 'index-1' });
  assert.match(document.querySelector('.index-result-list').textContent, /Committed/);
  assert.equal(document.querySelector('.index-detail-preview').textContent, 'Original preview');
  assert.match(document.querySelector('.index-search-status').textContent, /1 results/);
  const pending = submit('second');
  // Issue #5531: recover a real pending read through the canonical response model.
  controller.resyncProjectIndexSearch(['index-1', 'other']);
  const replacement = sent.at(-1);
  assert.equal(replacement.query, 'second');
  assert.deepEqual(replacement.scopes, pending.scopes);
  assert.equal(replacement.match_mode, pending.match_mode);
  assert.ok(replacement.request_id > pending.request_id);
  controller.handleProjectIndexSearchResults({ id: 'index-1', request_id: pending.request_id, results: [] });
  assert.match(document.querySelector('.index-search-status').textContent, /Updating results/);
  assert.match(document.querySelector('.index-result-list').textContent, /Committed/);
  controller.handleProjectIndexSearchResults({ id: 'index-1', request_id: replacement.request_id,
    results: [{ title: 'Fresh snapshot' }], suggestions: [] });
  assert.match(document.querySelector('.index-search-status').textContent, /1 results/);
  assert.match(document.querySelector('.index-result-list').textContent, /Fresh snapshot/);
  submit('');
  controller.handleProjectIndexSearchError({ id: 'index-1', request_id: pending.request_id, message: 'stale' });
  assert.match(document.querySelector('.index-search-status').textContent, /^Ready/);
  const closedRequest = submit('closing');
  controller.disposeIndexSearchWindow('index-1');
  const oldContent = document.getElementById('body').innerHTML;
  controller.handleProjectIndexSearchResults({ ...payload, request_id: closedRequest.request_id });
  assert.equal(document.getElementById('body').innerHTML, oldContent);
  controller.mountProjectIndexSurface(document.getElementById('body'), { id: 'index-1' });
  assert.match(document.querySelector('.index-result-list').textContent, /Search indexed content/);
  const recovered = submit('recovered');
  controller.handleProjectIndexSearchError({ id: 'index-1', request_id: recovered.request_id, message: 'Try again' });
  assert.equal(document.querySelector('.index-search-status').textContent, 'Try again');
  controller.handleProjectIndexSearchResults({ ...payload, request_id: closedRequest.request_id });
  assert.equal(document.querySelector('.index-search-status').textContent, 'Try again');
});

test("Index search isolates window selectors and cancels pending searches on close", async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const { controller, document, sent } = await indexHealthFixture(t);
  const other = document.createElement('div');
  other.dataset.id = 'index-2';
  const body = document.createElement('div');
  other.appendChild(body);
  document.body.appendChild(other);
  controller.mountProjectIndexSurface(body, { id: 'index-2' });
  const firstNode = document.querySelector('[data-id="index-1"] .index-result-list').firstChild;
  const input = body.querySelector('.index-search-input');
  input.value = 'pending';
  input.dispatchEvent(new document.defaultView.Event('input'));
  body.querySelector('.index-search-box').dispatchEvent(new document.defaultView.Event('submit'));
  const request = sent.at(-1);
  controller.handleProjectIndexSearchResults({ id: 'index-2', request_id: request.request_id, results: [{ title: 'Second' }] });
  assert.strictEqual(document.querySelector('[data-id="index-1"] .index-result-list').firstChild, firstNode);
  input.value = 'close before debounce';
  input.dispatchEvent(new document.defaultView.Event('input'));
  controller.disposeIndexSearchWindow('index-2');
  const sendCount = sent.length;
  t.mock.timers.tick(1000);
  assert.equal(sent.length, sendCount);
  assert.equal(controller.indexSearchStateMap.has('index-2'), false);
  controller.mountProjectIndexSurface(body, { id: 'index-2' });
  const freshInput = body.querySelector('.index-search-input');
  freshInput.value = 'fresh';
  freshInput.dispatchEvent(new document.defaultView.Event('input'));
  body.querySelector('.index-search-box').dispatchEvent(new document.defaultView.Event('submit'));
  assert.notEqual(sent.at(-1).request_id, request.request_id);
  controller.handleProjectIndexSearchResults({ id: 'index-2', request_id: request.request_id, results: [{ title: 'Stale' }] });
  assert.doesNotMatch(body.querySelector('.index-result-list').textContent, /Stale/);
});

test("Index health still renders a committed snapshot when another view fails", async t => {
  const { controller, document } = await indexHealthFixture(t, () => { throw new Error('settings failed'); });
  assert.throws(() => controller.setIndexStatus('/repo', { state: 'ready', scopes: { issues: { healthy: true } } }), /settings failed/);
  assert.match(document.querySelector('.index-search-health-summary').textContent, /1 ready/);
  assert.equal(controller.getIndexStatus('/repo').scopes.issues.healthy, true);
});

test("project-bar Index badge has been withdrawn (SPEC-1939 Phase 13)", () => {
  const { document } = parseHTML(indexHtml);
  assert.equal(
    document.getElementById("index-status"),
    null,
    "#index-status badge must not exist in the embedded HTML",
  );
  assert.ok(
    !indexHtml.includes(".index-status "),
    "embedded inline <style> must not declare .index-status rules",
  );
  assert.ok(
    !componentsCss.includes(".index-status"),
    "components.css must not declare .index-status rules",
  );
  assert.ok(
    !indexHtml.includes("index-status-toast"),
    "embedded HTML must not declare the index-status progress toast",
  );
});

test("project tab state cues no longer wire Project Index health", () => {
  assert.ok(
    !appSource.includes("aggregateProjectTabDotState"),
    "app.js must not import or call the removed project-tab Index health helper",
  );
  assert.ok(
    !appSource.includes("formatIndexStatusLabel"),
    "app.js must not import or call the removed formatIndexStatusLabel helper",
  );
  assert.ok(
    !appSource.includes("showRepairingProgressToast"),
    "app.js must not retain the badge progress toast helper",
  );
  assert.ok(
    !appSource.includes("indexStatusLabel"),
    "app.js must not retain references to the removed badge element",
  );
});

test("settings target=index opens the dedicated Index window", () => {
  assert.ok(
    !settingsSurfaceSource.includes('buildSettingsTab("index"'),
    "Settings must not expose an Index tab",
  );
  assert.ok(
    !settingsSurfaceSource.includes('dataset.settingsPanel = "index"'),
    "Settings must not mount an Index panel",
  );
  assert.ok(
    settingsSurfaceSource.includes('if (target === "index")') &&
      settingsSurfaceSource.includes('focusOrSpawnPreset("index");'),
    "settings:open target=index must spawn the dedicated Index window",
  );
});

test("Index window exposes semantic search and health refresh contract", () => {
  assert.ok(
    settingsSurfaceSource.includes("function requestFullIndexStatusRefresh()"),
    "expected a dedicated full index status refresh helper",
  );
  assert.ok(
    settingsSurfaceSource.includes('send({ kind: "refresh_index_status", project_root: activeProjectRoot })'),
    "Index window Health tab must request the expensive all-worktree status on demand",
  );
  assert.ok(
    indexSurfaceSource.includes('kind: "search_project_index"') &&
      appSource.includes('case "project_index_search_results"') &&
    appSource.includes('case "project_index_search_error"'),
    "Index window must wire search request, result, and error events",
  );
});

test("Index search UI exposes explicit search controls and readable result scoring", () => {
  assert.ok(
    indexSurfaceSource.includes("index-run-button") &&
      indexSurfaceSource.includes("formatIndexSearchMatch") &&
      indexSurfaceSource.includes("% match"),
    "Index search should have an explicit search action and user-facing match scores",
  );
  assert.ok(
    indexSurfaceSource.includes("indexFileScopesSelected(state)") &&
      indexSurfaceSource.includes("File worktree"),
    "worktree selection should be scoped to Files / Docs search instead of looking globally required",
  );
  assert.ok(
    indexSurfaceSource.includes("moveIndexResultSelection") &&
      indexSurfaceSource.includes('event.key === "ArrowDown"') &&
      indexSurfaceSource.includes('event.key === "ArrowUp"'),
    "result lists should support keyboard movement",
  );
  assert.ok(
    indexSurfaceSource.includes("inFlightSignature") &&
    indexSurfaceSource.includes("indexSearchModel.read()[windowId].searching && state.inFlightSignature === searchSignature"),
    "explicit search clicks should not duplicate an identical debounced search already in flight",
  );
  assert.ok(
    indexSurfaceSource.includes("state.query = input.value;") &&
      indexSurfaceSource.includes("renderProjectIndexSearch(windowData.id);\n            scheduleProjectIndexSearch(windowData.id);"),
    "typing in the search field should immediately enable the explicit Search button before debounce fires",
  );
});

test("Index search UI exposes semantic and all-terms match modes", () => {
  assert.ok(
    indexSurfaceSource.includes("index-match-mode-list") &&
      indexSurfaceSource.includes('data-match-mode="semantic"') &&
      indexSurfaceSource.includes('data-match-mode="all_terms"'),
    "Index search should expose a Semantic / All terms segmented control",
  );
  assert.ok(
    indexSurfaceSource.includes("match_mode: state.matchMode") &&
      indexSurfaceSource.includes("matchMode") &&
      indexSurfaceSource.includes("searchSignature = JSON.stringify({ query, scopes, worktreeHash, matchMode"),
    "match mode should be sent to the backend and included in the request signature",
  );
  assert.ok(
    indexSurfaceSource.includes("state.suggestions") &&
      indexSurfaceSource.includes("Semantic suggestions") &&
      indexSurfaceSource.includes("Matched:") &&
      indexSurfaceSource.includes("Searching all terms"),
    "All terms mode should render suggestions separately, show concise matched-term evidence, and use matching loading copy",
  );
  assert.ok(
    indexSurfaceSource.includes("function indexSearchPlaceholder(state)") &&
      indexSurfaceSource.includes("Search by meaning, e.g. work lifecycle") &&
      indexSurfaceSource.includes("All terms required, e.g. Work discussion") &&
      indexSurfaceSource.includes("input.placeholder = indexSearchPlaceholder(state);"),
    "Index search placeholder should explain the active Semantic / All terms mode",
  );
});

test("Index search clears invalidate any in-flight request immediately", () => {
  assert.ok(
    indexSurfaceSource.includes("function clearProjectIndexSearchState(state)") &&
      indexSurfaceSource.includes("invalidateProjectIndexSearchRequest(state);"),
    "clearing the query should invalidate stale backend responses through a shared helper",
  );
  assert.ok(
    indexSurfaceSource.includes("if (!state.query.trim()) {\n              clearProjectIndexSearchState(state);"),
    "the input handler must clear and invalidate immediately instead of waiting for debounce",
  );
});

test("Index result Open uses target numbers for Issue and SPEC hits", () => {
  const openKnowledgeTargetBlock = indexSurfaceSource
    .split("function openKnowledgeIndexResultTarget(preset, target)")[1]
    ?.split("// SPEC-3064 Phase 3 (E3)")[0];
  assert.ok(openKnowledgeTargetBlock, "expected the Knowledge target handoff helper");
  assert.ok(
    indexSurfaceSource.includes("function openKnowledgeIndexResultTarget(preset, target)") &&
      indexSurfaceSource.includes("requestKnowledgeDetail(windowId, knowledgeKind, number)") &&
      indexSurfaceSource.includes('pendingIndexOpenTargetsByPreset.set(knowledgeKind === "issue" ? "settings" : preset'),
    "Issue/SPEC result Open should select the indexed target number, including newly created windows",
  );
  assert.ok(
    indexSurfaceSource.includes('openKnowledgeIndexResultTarget("issue", target)') &&
      indexSurfaceSource.includes('openKnowledgeIndexResultTarget("spec", target)'),
    "Issue and SPEC index results must use target-aware navigation",
  );
  assert.doesNotMatch(
    openKnowledgeTargetBlock,
    /requestKnowledgeDetail\(windowId, knowledgeKind, number\);[\s\S]*renderKnowledgeBridge\(windowId\);/,
    "the selection dispatcher already renders by kind; Index handoff must not rebuild the full Knowledge list again",
  );
});

test("Index search tab refreshes missing health once per project root", async t => {
  const { controller, document, sent } = await indexHealthFixture(t);
  controller.mountProjectIndexSurface(document.getElementById('body'), { id: 'index-1' });
  assert.equal(sent.filter(message => message.kind === 'refresh_index_status').length, 1);
  assert.ok(
    indexSurfaceSource.includes("function ensureIndexStatusRefresh(state, status)") &&
      indexSurfaceSource.includes("state.statusRefreshProjectRoot !== activeProjectRoot") &&
      indexSurfaceSource.includes("state.statusRefreshProjectRoot = activeProjectRoot;") &&
      indexSurfaceSource.includes("requestFullIndexStatusRefresh();"),
    "Search mount should request a full status refresh when the active project has no health payload yet",
  );
  assert.ok(
    indexSurfaceSource.includes("state.statusRefreshProjectRoot = \"\";"),
    "Search refresh guard should reset when project context disappears or a status payload arrives",
  );
});

test("Index search tab renders abnormal-first health summary without repair controls", () => {
  assert.ok(
    indexSurfaceSource.includes("function renderIndexSearchHealthSummary(") &&
      indexSurfaceSource.includes('data-role="index-search-health-summary"'),
    "Search tab should render a dedicated health summary near the search controls",
  );
  assert.ok(
    indexSurfaceSource.includes("index-search-health-inline") &&
      indexSurfaceSource.includes("open-index-health") &&
      indexSurfaceSource.includes("requestFullIndexStatusRefresh();"),
    "degraded Search health should stay inline and provide a Health tab action",
  );
  assert.ok(
    indexSurfaceSource.includes("buildIndexHealthSummary(") &&
      indexSurfaceSource.includes("readyCount") &&
      indexSurfaceSource.includes("degradedScopes"),
    "Search health summary should be built from ready/degraded scope counts",
  );
  assert.ok(
    !indexSurfaceSource.includes('data-action="open-project"') &&
      !indexSurfaceSource.includes("Open Project"),
    "Index window empty states must not offer an Open Project action",
  );
});
