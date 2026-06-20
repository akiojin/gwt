import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const webRoot = resolve(here, "..");
const embeddedRoutesSource = readFileSync(
  resolve(here, "../../playwright/tests/_helpers/embedded-frontend.ts"),
  "utf8",
);

function rootModuleImports(source) {
  return [...source.matchAll(/from\s+["']\/([^"']+\.js)["']/g)]
    .map((match) => match[1])
    .sort();
}

function reachableRootModuleImports(entryModuleName) {
  const seen = new Set();
  const pending = [entryModuleName];
  while (pending.length > 0) {
    const moduleName = pending.pop();
    if (seen.has(moduleName)) continue;
    seen.add(moduleName);
    const source = readFileSync(resolve(webRoot, moduleName), "utf8");
    for (const importedModule of rootModuleImports(source)) {
      if (!seen.has(importedModule)) {
        pending.push(importedModule);
      }
    }
  }
  return [...seen].sort();
}

function playwrightRootModules(source) {
  const declaration = source.match(/const ROOT_MODULES = new Set\(\[([\s\S]*?)\]\);/);
  assert.ok(declaration, "Playwright embedded route helper must declare ROOT_MODULES");
  return [...declaration[1].matchAll(/"([^"]+\.js)"/g)].map((match) => match[1]);
}

test("Playwright embedded routes serve every app.js transitive root module import", () => {
  const modules = new Set(playwrightRootModules(embeddedRoutesSource));
  const appImports = reachableRootModuleImports("app.js");
  const missing = appImports.filter((moduleName) => !modules.has(moduleName));
  assert.deepEqual(missing, []);
  const orphaned = [...modules].filter((moduleName) => !appImports.includes(moduleName));
  assert.ok(
    !orphaned.includes("index-status-controller.js"),
    `removed project-tab Index route must not remain in Playwright helper: ${orphaned.join(", ")}`,
  );
});
