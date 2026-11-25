// SPEC-2359 US-80 — the project index search window exposes the `works`
// scope so users can find similar prior Work (including completed/discarded)
// before starting duplicate work. The scope must appear both in the togglable
// scope list and in the default selected set.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { INDEX_SCOPE_IDS } from "../index-settings-panel.js";

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

test("Issue #4174: file results cannot spawn the retired File Tree GUI", () => {
  assert.doesNotMatch(surfaceSource, /focusOrSpawnPreset\("file_tree"\)/);
  assert.match(surfaceSource, /if \(selected\.target\?\.kind !== "file"\) \{\s*detail\.appendChild\(makeEl\("button"/);
});

test("Issue #7729 AC-3: the search offers the nine scopes of the health summary, in its order", () => {
  const scopesBlock = surfaceSource.slice(
    surfaceSource.indexOf("const INDEX_SEARCH_SCOPES"),
    surfaceSource.indexOf("const INDEX_SEARCH_DEFAULT_SCOPES"),
  );
  const ids = [...scopesBlock.matchAll(/id:\s*"([^"]+)"/g)].map(match => match[1]);
  assert.equal(ids.length, 9);
  assert.deepEqual(ids, INDEX_SCOPE_IDS);
  const defaults = surfaceSource.slice(
    surfaceSource.indexOf("const INDEX_SEARCH_DEFAULT_SCOPES"),
    surfaceSource.indexOf("function ensureIndexSearchState"),
  );
  assert.deepEqual(
    [...defaults.matchAll(/"([^"]+)"/g)].map(match => match[1]),
    INDEX_SCOPE_IDS.filter(id => !id.startsWith("files")),
    "files and docs stay out of the default selection",
  );
  assert.match(surfaceSource, /Ready \$\{summary\.scopeReadyCount\}\/\$\{summary\.scopeTotal\}/);
  assert.doesNotMatch(surfaceSource, /scopes indexed/);
});
