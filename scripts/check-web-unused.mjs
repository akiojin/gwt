// Import-graph guard for the shipped browser modules. Test-only exports remain
// valid test seams, but importing a retired module in a test cannot keep it live.
// Static namespace reads/destructuring consume their named exports; unknown
// namespace uses remain conservative. Vendor assets and the generated protocol
// enum are outside this hand-written API check.
import { parse } from "acorn";
import { readFileSync, readdirSync } from "node:fs";
import { posix } from "node:path";

export function readWebModules(root) {
  return new Map(readdirSync(root, { recursive: true })
    .map(name => name.replaceAll("\\", "/"))
    .filter(name => /^[^/]+\.js$/.test(name) || /^__tests__\/.*\.mjs$/.test(name))
    .map(name => [name, readFileSync(new URL(name, root), "utf8")]));
}

export function auditModules(modules, entries) {
  const records = new Map();
  for (const [file, source] of modules) {
    const exports = new Set(), imports = [], readTargets = new Set(), identifiers = new Set();
    const namespaces = new Map();
    const ast = parse(source, { ecmaVersion: "latest", sourceType: "module" });
    function addImport(specifier, names) {
      if (!specifier || (!specifier.startsWith(".") && !specifier.startsWith("/"))) return;
      const target = posix.normalize(specifier.startsWith("/") ? specifier.slice(1)
        : posix.join(posix.dirname(file), specifier));
      if (modules.has(target)) imports.push({ target, names });
    }
    function bindings(node) {
      if (!node) return [];
      if (node.type === "Identifier") return [node.name];
      if (node.type === "ObjectPattern") return node.properties.flatMap(p => bindings(p.value ?? p.argument));
      if (node.type === "ArrayPattern") return node.elements.flatMap(bindings);
      return bindings(node.left ?? node.argument);
    }
    function propertyName(node) {
      return node.computed ? (["string", "number"].includes(typeof node.property.value) ? String(node.property.value) : "*")
        : node.property.name;
    }
    function patternNames(node) {
      return node.type === "ObjectPattern" ? node.properties.map(p => p.type === "RestElement"
        ? "*" : p.computed ? (typeof p.key.value === "string" ? p.key.value : "*")
          : p.key.name ?? p.key.value) : ["*"];
    }
    function visit(node, parent, grandparent) {
      if (!node || typeof node !== "object") return;
      // Several DOM tests rewrite absolute browser imports before importing a
      // data URL or temporary module. Recognize their explicit source-file path
      // and referenced bindings without making those modules browser entries.
      if (node.type === "Identifier") identifiers.add(node.name);
      if (node.type === "Literal" && typeof node.value === "string" && /^\.\.?\/.*\.js$/.test(node.value)) {
        readTargets.add(posix.normalize(posix.join(posix.dirname(file), node.value)));
      }
      if (node.type === "ImportDeclaration") {
        addImport(node.source.value, node.specifiers.filter(s => s.type !== "ImportNamespaceSpecifier")
          .map(s => s.type === "ImportDefaultSpecifier" ? "default" : s.imported.name ?? s.imported.value));
        for (const s of node.specifiers.filter(s => s.type === "ImportNamespaceSpecifier")) {
          const names = new Set();
          namespaces.set(s.local.name, names);
          addImport(node.source.value, names);
        }
      }
      if (node.type === "ImportExpression") {
        const value = parent?.type === "AwaitExpression" ? parent : node;
        const context = value === parent ? grandparent : parent;
        const names = new Set();
        if (context?.type === "VariableDeclarator" && context.init === value) {
          if (context.id.type === "Identifier") namespaces.set(context.id.name, names);
          else patternNames(context.id).forEach(name => names.add(name));
        } else if (context?.type === "MemberExpression" && context.object === value) {
          names.add(propertyName(context));
        } else names.add("*");
        addImport(node.source.value, names);
      }
      if (node.type === "ExportDefaultDeclaration") exports.add("default");
      if (node.type === "ExportAllDeclaration") addImport(node.source.value, ["*"]);
      if (node.type === "ExportNamedDeclaration") {
        if (node.declaration?.id) exports.add(node.declaration.id.name);
        for (const d of node.declaration?.declarations ?? []) bindings(d.id).forEach(name => exports.add(name));
        for (const s of node.specifiers) exports.add(s.exported.name ?? s.exported.value);
        if (node.source) addImport(node.source.value, node.specifiers.map(s => s.local.name ?? s.local.value));
      }
      for (const child of Object.values(node)) {
        if (Array.isArray(child)) child.forEach(value => visit(value, node, parent));
        else if (child && typeof child === "object") visit(child, node, parent);
      }
    }
    visit(ast);
    function namespaceUses(node, parent) {
      if (!node || typeof node !== "object") return;
      const names = node.type === "Identifier" ? namespaces.get(node.name) : null;
      if (names) {
        if (parent?.type === "MemberExpression" && parent.object === node) names.add(propertyName(parent));
        else if (parent?.type === "VariableDeclarator" && parent.init === node) {
          patternNames(parent.id).forEach(name => names.add(name));
        } else if (parent?.type !== "ImportNamespaceSpecifier"
          && !(parent?.type === "VariableDeclarator" && parent.id === node)
          && !(parent?.type === "MemberExpression" && parent.property === node && !parent.computed)
          && !(parent?.type === "Property" && parent.key === node && !parent.computed && !parent.shorthand)) names.add("*");
      }
      for (const child of Object.values(node)) {
        if (Array.isArray(child)) child.forEach(value => namespaceUses(value, node));
        else if (child && typeof child === "object") namespaceUses(child, node);
      }
    }
    namespaceUses(ast);
    records.set(file, { exports, imports, readTargets, identifiers });
  }
  const live = new Set();
  function visit(file) {
    if (live.has(file)) return;
    const record = records.get(file);
    if (!record) throw new Error(`Missing browser entry: ${file}`);
    live.add(file);
    record.imports.forEach(({ target }) => visit(target));
  }
  entries.forEach(visit);
  const consumed = new Map();
  for (const [file, { imports, readTargets, identifiers }] of records) {
    if (!live.has(file) && !file.startsWith("__tests__/")) continue;
    for (const { target, names } of imports) {
      const uses = consumed.get(target) ?? new Set();
      names.forEach(name => uses.add(name));
      consumed.set(target, uses);
    }
    if (file.startsWith("__tests__/")) for (const target of readTargets) {
      const uses = consumed.get(target) ?? new Set();
      for (const name of records.get(target)?.exports ?? []) if (identifiers.has(name)) uses.add(name);
      consumed.set(target, uses);
    }
  }
  const production = [...records.keys()].filter(file => !file.startsWith("__tests__/"));
  return {
    unusedModules: production.filter(file => !live.has(file)).sort(),
    unusedExports: production.filter(file => live.has(file) && file !== "protocol-enums.js")
      .flatMap(file => [...records.get(file).exports]
        .filter(name => !consumed.get(file)?.has(name) && !consumed.get(file)?.has("*"))
        .map(name => `${file}:${name}`)).sort(),
  };
}
