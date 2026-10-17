import assert from "node:assert/strict";
import test from "node:test";
import { auditModules, readWebModules } from "../../../../scripts/check-web-unused.mjs";

test("unused export guard follows aliases, re-exports and namespace consumers", () => {
  const modules = new Map([
    ["entry.js", 'import { renamed } from "./bridge.js"; import * as ns from "./namespace.js"; renamed(); ns.used(); const { second: alias } = ns; alias();'],
    ["bridge.js", 'export { used as renamed } from "./helpers.js";'],
    ["helpers.js", 'export function used() {} export function abandoned() {}'],
    ["namespace.js", 'export const used = 1; export const second = 2; export const abandoned = 3;'],
    ["orphan.js", 'export const retired = true;'],
  ]);
  assert.deepEqual(auditModules(modules, ["entry.js"]), {
    unusedModules: ["orphan.js"],
    unusedExports: ["helpers.js:abandoned", "namespace.js:abandoned"],
  });
});

test("test imports preserve test seams without making orphan modules reachable", () => {
  const modules = new Map([
    ["entry.js", 'import { live } from "/helpers.js"; live();'],
    ["helpers.js", 'export function live() {} export function testSeam() {} export const abandoned = true;'],
    ["orphan.js", 'export const retired = true;'],
    ["__tests__/example.test.mjs", 'const { testSeam: seam } = await import("../helpers.js"); seam(); import "../orphan.js";'],
  ]);
  assert.deepEqual(auditModules(modules, ["entry.js"]), {
    unusedModules: ["orphan.js"], unusedExports: ["helpers.js:abandoned"],
  });
});

test("unknown namespace property reads and escaped namespaces remain conservative", () => {
  const modules = new Map([
    ["entry.js", 'import * as computed from "./computed.js"; import * as escaped from "./escaped.js"; computed[key](); consume(escaped);'],
    ["computed.js", 'export const possiblyUsed = 1;'],
    ["escaped.js", 'export const possiblyUsed = 1;'],
  ]);
  assert.deepEqual(auditModules(modules, ["entry.js"]), {
    unusedModules: [], unusedExports: [],
  });
});

test("rewritten browser-module test imports retain only the referenced exports", () => {
  const modules = new Map([
    ["entry.js", 'import "./helpers.js";'],
    ["helpers.js", 'export function seam() {} export const retired = true;'],
    ["__tests__/example.test.mjs", 'const source = readFileSync(new URL("../helpers.js", import.meta.url)); const { seam } = await loadRewritten(source); seam();'],
  ]);
  assert.deepEqual(auditModules(modules, ["entry.js"]), {
    unusedModules: [], unusedExports: ["helpers.js:retired"],
  });
});

test("shipped web modules have no unused modules or exports (Issue #5601)", () => {
  const modules = readWebModules(new URL("../", import.meta.url));
  // app.js is inserted by frontend-bootstrap as a module script on Project routes.
  assert.deepEqual(auditModules(modules, ["frontend-bootstrap.js", "app.js"]), {
    unusedModules: [], unusedExports: [],
  });
});
