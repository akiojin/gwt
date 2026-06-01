import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const embeddedRoutesSource = readFileSync(
  resolve(here, "../../playwright/tests/_helpers/embedded-frontend.ts"),
  "utf8",
);

function rootModuleImports(source) {
  return [...source.matchAll(/from\s+["']\/([^"']+\.js)["']/g)]
    .map((match) => match[1])
    .sort();
}

function playwrightRootModules(source) {
  const declaration = source.match(/const ROOT_MODULES = new Set\(\[([\s\S]*?)\]\);/);
  assert.ok(declaration, "Playwright embedded route helper must declare ROOT_MODULES");
  return [...declaration[1].matchAll(/"([^"]+\.js)"/g)].map((match) => match[1]);
}

test("Playwright embedded routes serve every app.js root module import", () => {
  const modules = new Set(playwrightRootModules(embeddedRoutesSource));
  const appImports = rootModuleImports(appSource);
  const missing = appImports.filter((moduleName) => !modules.has(moduleName));
  assert.deepEqual(missing, []);
  const orphaned = [...modules].filter((moduleName) => !appImports.includes(moduleName));
  assert.ok(
    !orphaned.includes("index-status-controller.js"),
    `removed project-tab Index route must not remain in Playwright helper: ${orphaned.join(", ")}`,
  );
});
