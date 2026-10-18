import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import test from "node:test";
import { parse } from "acorn";

function pathImports(source) {
  const lines = [];
  function visit(node) {
    if (!node || typeof node !== "object") return;
    if (node.type === "ImportExpression" && node.source.type === "CallExpression") {
      const callee = node.source.callee;
      const name = callee.type === "Identifier" ? callee.name
        : callee.type === "MemberExpression" && !callee.computed
          && callee.object.type === "Identifier" && callee.object.name === "path"
          ? callee.property.name : null;
      if (name === "resolve" || name === "join") lines.push(node.loc.start.line);
    }
    for (const child of Object.values(node)) {
      if (Array.isArray(child)) child.forEach(visit);
      else if (child && typeof child === "object") visit(child);
    }
  }
  visit(parse(source, { ecmaVersion: "latest", sourceType: "module", locations: true }));
  return lines;
}

test("dynamic import guard detects bare path calls including multiline imports", () => {
  assert.deepEqual(pathImports(`import(resolve(here, "module.js"));
    import(
      join(here, "module.js")
    );
    import(path.resolve(here, "module.js"));`), [1, 2, 5]);
});

test("dynamic import guard permits file URLs, module specifiers, and inert text", () => {
  assert.deepEqual(pathImports(`import(pathToFileURL(resolve(here, "module.js")).href);
    import(pathToFileURL(join(here, "module.js")).href);
    import("../module.js");
    import(import.meta.resolve("node:fs"));
    const example = 'import(resolve(here, "module.js"))';
    // import(join(here, "module.js"))
  `), []);
});

test("web tests convert resolved dynamic import paths to file URLs (Issue #5470)", () => {
  const root = new URL("./", import.meta.url);
  const violations = readdirSync(root, { recursive: true })
    .filter(name => /\.[cm]?js$/.test(name))
    .flatMap(name => pathImports(readFileSync(new URL(name.replaceAll("\\", "/"), root), "utf8"))
      .map(line => `${name}:${line}`));
  assert.deepEqual(violations, [], "Wrap dynamic import paths in pathToFileURL(...).href");
});
