import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { dirname as posixDirname, normalize as posixNormalize } from "node:path/posix";
import test from "node:test";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const webRoot = resolve(here, "..");
const embeddedRoutesSource = readFileSync(
  resolve(here, "../../playwright/tests/_helpers/embedded-frontend.ts"),
  "utf8",
);

function webModuleImports(source, importerModuleName) {
  const specifiers = [
    ...source.matchAll(/\bfrom\s+["']([^"']+\.js)["']/g),
    ...source.matchAll(/\bimport\s+["']([^"']+\.js)["']/g),
  ];
  return [
    ...new Set(
      specifiers
        .map((match) => resolveWebModuleSpecifier(importerModuleName, match[1]))
        .filter(Boolean),
    ),
  ].sort();
}

function resolveWebModuleSpecifier(importerModuleName, specifier) {
  if (specifier.startsWith("/")) {
    return specifier.slice(1);
  }
  if (!specifier.startsWith("./") && !specifier.startsWith("../")) {
    return null;
  }
  const importerDir = posixDirname(importerModuleName);
  const resolved = posixNormalize(importerDir === "." ? specifier : `${importerDir}/${specifier}`);
  if (resolved === ".." || resolved.startsWith("../") || resolved.startsWith("/")) {
    return null;
  }
  return resolved;
}

function reachableRootModuleImports(entryModuleName) {
  const seen = new Set();
  const pending = [entryModuleName];
  while (pending.length > 0) {
    const moduleName = pending.pop();
    if (seen.has(moduleName)) continue;
    seen.add(moduleName);
    const source = readFileSync(resolve(webRoot, moduleName), "utf8");
    for (const importedModule of webModuleImports(source, moduleName)) {
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

test("Playwright embedded route import graph follows relative web module imports", () => {
  const imports = reachableRootModuleImports("project-tabs-renderer.js");
  assert.ok(imports.includes("window-runtime-state.js"));
  assert.ok(imports.includes("protocol-enums.js"));
});
