// SPEC-2359 US-80 — the project index search window exposes the `works`
// scope so users can find similar prior Work (including completed/discarded)
// before starting duplicate work. The scope must appear both in the togglable
// scope list and in the default selected set.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import vm from "node:vm";

const here = dirname(fileURLToPath(import.meta.url));
const surfaceSource = readFileSync(
  resolve(here, "../project-index-search-surface.js"),
  "utf8",
);

test("INDEX_SEARCH_SCOPES offers a togglable Work scope", () => {
  const scopesBlock = surfaceSource.slice(
    surfaceSource.indexOf("const INDEX_SEARCH_SCOPES"),
    surfaceSource.indexOf("const INDEX_SEARCH_DEFAULT_SCOPES"),
  );
  assert.match(scopesBlock, /id:\s*"works"/);
  assert.match(scopesBlock, /label:\s*"Work"/);
});

test("works is part of the default selected search scopes", () => {
  const defaultsBlock = surfaceSource.slice(
    surfaceSource.indexOf("const INDEX_SEARCH_DEFAULT_SCOPES"),
    surfaceSource.indexOf("function ensureIndexSearchState"),
  );
  assert.match(defaultsBlock, /"works"/);
});

test("Issue #5531: resync replaces a lost index search without losing its query or results", () => {
  const createSurface = vm.runInNewContext(surfaceSource.slice(surfaceSource.indexOf("export function"))
    .replace("export function", "function") + "; createProjectIndexSearchSurface;", {
    document: { querySelector: () => null }, CSS: { escape: value => value }, clearTimeout,
  });
  const sent = [];
  const surface = createSurface({ send: message => sent.push(message), activeProjectTab: () => null });
  const state = { query: "pending query", selectedScopes: new Set(["issues"]), matchMode: "all_terms",
    requestId: 7, inFlightRequestId: 7, inFlightSignature: "lost", searching: true,
    results: [{ title: "Previous result" }], searchTimer: 0 };
  surface.indexSearchStateMap.set("index", state);
  surface.resyncProjectIndexSearch(["index", "other"]);
  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, "search_project_index");
  assert.equal(sent[0].query, state.query);
  assert.deepEqual(Array.from(sent[0].scopes), ["issues"]);
  assert.equal(sent[0].match_mode, "all_terms");
  assert.ok(sent[0].request_id > 7, "the lost request can no longer settle the replacement");
  assert.equal(state.results[0].title, "Previous result");
  surface.handleProjectIndexSearchResults({ id: "index", request_id: 7, results: [] });
  assert.equal(state.searching, true, "late old replies stay fenced");
  surface.handleProjectIndexSearchResults({ id: "index", request_id: sent[0].request_id,
    results: [{ title: "Fresh snapshot" }], suggestions: [] });
  assert.equal(state.searching, false);
  assert.equal(state.results[0].title, "Fresh snapshot");
});
