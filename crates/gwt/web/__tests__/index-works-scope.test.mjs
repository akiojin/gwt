// SPEC-2359 US-80 — the project index search window exposes the `works`
// scope so users can find similar prior Work (including completed/discarded)
// before starting duplicate work. The scope must appear both in the togglable
// scope list and in the default selected set.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

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
