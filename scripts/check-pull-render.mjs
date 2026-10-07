// AC-12: receiver roots may update models, but must not acquire new DOM paths.
// This is a scoped syntax guard, not a whole-program/type analysis. Only local
// named helpers and one-hop named factory return aliases are resolved.
// Other external/dynamic calls produce findings instead
// of being silently trusted; their internals are outside this guard's scope.
// Known data API spellings are trusted. Event/subscription handlers are not
// executed here; synchronous and scheduled callbacks are followed explicitly.
import { parse } from "acorn";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const rootPattern = /^(receive|apply.*ReceiveEvent|applyIssueMonitorStatus|applyIssueMonitorInbox|applyProviderUsageUi)$/;
const functionPattern = /^(FunctionDeclaration|FunctionExpression|ArrowFunctionExpression)$/;
const domWrites = new Set(["innerHTML", "outerHTML", "textContent", "innerText", "className", "value", "checked", "disabled", "hidden", "scrollTop", "scrollLeft"]);
const domCalls = new Set(["append", "appendChild", "prepend", "replaceChildren", "replaceChild", "remove", "removeChild", "insertBefore", "insertAdjacentHTML", "setAttribute", "removeAttribute", "createElement", "createTextNode", "focus", "blur"]);
const dataMethods = new Set(("get set has delete clear push pop shift unshift splice slice map filter find findIndex some every reduce sort concat join includes indexOf trim toLowerCase toUpperCase toString replace split match test startsWith endsWith substring at flat flatMap getAttribute querySelector querySelectorAll").split(" "));
const callbacks = new Set(("forEach map filter find findIndex some every reduce sort flatMap then catch finally").split(" "));
const schedules = new Set(["setTimeout", "setInterval", "queueMicrotask", "requestAnimationFrame"]);
const builtins = new Set(["String", "Number", "Boolean", "parseInt", "parseFloat", "isNaN", "isFinite", "Set", "Map", "WeakSet", "WeakMap", "Error", "TypeError", "AggregateError", "Date", "URL", "RegExp"]);
const pureNamespaces = /^(Math|Number|String|Array|JSON|Date|Intl|console|performance)\./;
const pureObject = /^Object\.(keys|values|entries|fromEntries|freeze|is|hasOwn|create|getPrototypeOf)$/;
const repoRoot = dirname(dirname(realpathSync(fileURLToPath(import.meta.url))));
const baselineFile = "scripts/pull-render-baseline.json";
const childNodes = node => Object.values(node).flatMap(value => Array.isArray(value) ? value : [value]).filter(value => value && typeof value.type === "string");
const property = node => node?.type === "MemberExpression" ? (node.computed ? node.property.value : node.property.name) : null;
const memberPath = node => node?.type === "Identifier" ? node.name : node?.type === "MemberExpression" ? memberPath(node.object) + "." + property(node) : "";
const fingerprint = node => createHash("sha256").update(JSON.stringify(node, (key, value) => ["start", "end", "loc", "raw"].includes(key) ? undefined : value)).digest("hex");

function factoryHandlers(source, name) {
  if (!source) return new Map();
  const ast = parse(source, { ecmaVersion: "latest", sourceType: "module" });
  const factory = ast.body.map(node => node.declaration || node).find(node => node.type === "FunctionDeclaration" && node.id.name === name);
  const returned = factory?.body.body.find(node => node.type === "ReturnStatement" && node.argument?.type === "ObjectExpression");
  const names = new Set(factory?.body.body.filter(node => node.type === "FunctionDeclaration").map(node => node.id.name));
  return new Map((returned?.argument.properties || []).filter(node => !node.computed && node.value?.type === "Identifier" && names.has(node.value.name))
    .map(node => [node.key.name || node.key.value, node.value.name]));
}

export function scanModule(source, file, { modules, rootName } = {}) {
  const ast = parse(source, { ecmaVersion: "latest", sourceType: "module", locations: true });
  const scopes = new WeakMap(), names = new WeakMap(), roots = [], findings = new Map();
  const scope = parent => ({ parent, bindings: new Map(), models: new Set() });
  const factoryCache = new Map(), handlerCache = new Map();
  function index(node, current) {
    if (node.type === "FunctionDeclaration") current.bindings.set(node.id.name, node);
    if (functionPattern.test(node.type)) {
      names.set(node, node.id?.name || names.get(node) || "<callback>");
      current = scope(current);
      for (const param of node.params) if (param.type === "Identifier") current.bindings.set(param.name, null);
      if (rootName ? names.get(node) === rootName : rootPattern.test(names.get(node))) roots.push(node);
    } else if (node.type === "BlockStatement") current = scope(current);
    scopes.set(node, current);
    if (node.type === "ImportDeclaration") {
      for (const specifier of node.specifiers) if (specifier.type === "ImportSpecifier") current.bindings.set(specifier.local.name,
        { imported: specifier.imported.name, module: join(dirname(file), node.source.value.replace(/^\//, "")) });
    }
    if (node.type === "VariableDeclarator" && node.id.type === "Identifier") {
      const fn = functionPattern.test(node.init?.type || "") ? node.init : null;
      current.bindings.set(node.id.name, fn);
      if (fn) names.set(fn, node.id.name);
      const factory = node.init?.type === "CallExpression" && resolve(node, node.init.callee.name);
      if (factory.imported === "createUiStateStore" && /(^|\/)ui-state-store\.js$/.test(factory.module)) current.models.add(node.id.name);
    }
    if (node.type === "VariableDeclarator" && node.id.type === "ObjectPattern" && node.init?.type === "CallExpression") {
      const factory = resolve(node, node.init.callee.name);
      const source = modules?.get(factory.module), cacheKey = factory.module + ":" + factory.imported;
      if (!factoryCache.has(cacheKey)) factoryCache.set(cacheKey, factoryHandlers(source, factory.imported));
      for (const property of node.id.properties) if (!property.computed && property.value?.type === "Identifier") {
        const name = factoryCache.get(cacheKey).get(property.key.name || property.key.value);
        current.bindings.set(property.value.name, name ? { handler: { name, source, file: factory.module } } : null);
      }
    }
    for (const child of childNodes(node)) index(child, current);
  }
  index(ast, scope(null));
  function resolve(node, name) {
    for (let current = scopes.get(node); current; current = current.parent) {
      if (current.bindings.has(name)) {
        const binding = current.bindings.get(name);
        return { fn: binding?.type ? binding : null, handler: binding?.handler, imported: binding?.imported,
          module: binding?.module, model: current.models.has(name) };
      }
    }
    return {};
  }
  for (const root of roots) {
    // One reachable helper per root/case prevents recursion/path explosion.
    const visited = new Set(), active = new Set();
    function add(kind, node, path, context) {
      const hash = fingerprint(node), key = JSON.stringify([file, names.get(root), context, kind, hash]);
      findings.set(key, { key, file, root: names.get(root), path, context, hash, kind, line: node.loc.start.line });
    }
    function follow(fn, path, context) {
      const key = fn.start + ":" + context;
      if (visited.has(key) || active.has(fn)) return;
      visited.add(key);
      active.add(fn);
      try { visit(fn.body, path, context); } finally { active.delete(fn); }
    }
    function visit(node, path, context) {
      if (functionPattern.test(node.type)) return;
      if (node.type === "SwitchStatement") {
        visit(node.discriminant, path, context);
        let labels = [];
        for (const arm of node.cases) {
          labels.push(arm.test?.value ?? "default");
          if (!arm.consequent.length) continue;
          for (const statement of arm.consequent) visit(statement, path, context + "/" + labels.join("|"));
          labels = [];
        }
        return;
      }
      if (["AssignmentExpression", "UpdateExpression"].includes(node.type)) {
        const target = node.left || node.argument;
        if (domWrites.has(property(target)) || /\.(style|dataset)(\.|$)/.test(memberPath(target))) add("dom-write", node, path, context);
      }
      if (node.type === "CallExpression" || node.type === "NewExpression") {
        const callee = node.callee.type === "ChainExpression" ? node.callee.expression : node.callee;
        const method = property(callee), spelling = memberPath(callee);
        const binding = callee.type === "Identifier" ? resolve(node, callee.name) : {};
        const local = binding.fn;
        const model = callee.object?.type === "Identifier" && resolve(node, callee.object.name).model;
        if (domCalls.has(method) || /\.(style|classList)\./.test(spelling)) add("dom-call", node, path, context);
        else if (local) follow(local, path + ">" + names.get(local), context);
        else if (binding.handler) {
          const handler = binding.handler, key = handler.file + ":" + handler.name;
          // Deliberately one hop: imported helpers inside the returned handler
          // remain unresolved, while its local closure/model bindings are read.
          if (!handlerCache.has(key)) handlerCache.set(key, scanModule(handler.source, handler.file, { rootName: handler.name }));
          for (const finding of handlerCache.get(key)) {
            const key = JSON.stringify([finding.file, names.get(root), context + finding.context, finding.kind, finding.hash]);
            findings.set(key, { ...finding, key, root: names.get(root), context: context + finding.context,
              path: path + ">" + handler.name + ">" + finding.path });
          }
        }
        else if (!(model && ["read", "update", "subscribe", "dispose"].includes(method))
          && !dataMethods.has(method) && !callbacks.has(method) && method !== "addEventListener"
          && !pureNamespaces.test(spelling) && !pureObject.test(spelling)
          && !builtins.has(callee.name) && !schedules.has(callee.name)
          && !["clearTimeout", "clearInterval", "cancelAnimationFrame"].includes(callee.name)) add("unresolved-call", node, path, context);
        const executeCallbacks = callbacks.has(method) || schedules.has(callee.name) || (model && method === "update");
        for (const [position, argument] of node.arguments.entries()) {
          if (executeCallbacks || (model && method === "subscribe" && position === 0)) {
            const fn = functionPattern.test(argument.type) ? argument : argument.type === "Identifier" ? resolve(node, argument.name).fn : null;
            if (fn) follow(fn, path + ">" + (method || callee.name) + ":callback", context);
          }
        }
      }
      for (const child of childNodes(node)) visit(child, path, context);
    }
    follow(root, names.get(root), "");
  }
  return [...findings.values()].sort((a, b) => a.key.localeCompare(b.key));
}

const receiverKey = finding => JSON.stringify([finding.file, finding.root, finding.context]);
const receiverKeys = findings => [...new Set(findings.map(receiverKey))].sort();

export function assertBaseline(findings, baseline, base, baseFindings) {
  const live = new Set(receiverKeys(findings));
  const allowed = new Set(baseline), ceiling = new Set(base);
  const existing = new Set(baseFindings.map(finding => finding.key));
  const fresh = findings.filter(finding => !allowed.has(receiverKey(finding)) || !existing.has(finding.key));
  const stale = [...allowed].filter(key => !live.has(key));
  const growth = [...allowed].filter(key => !ceiling.has(key));
  const errors = [];
  if (fresh.length) errors.push("new finding:\n" + fresh.map(finding => finding.key + " via " + finding.path).join("\n"));
  if (stale.length) errors.push("stale baseline; remove these rows:\n" + stale.join("\n"));
  if (growth.length) errors.push("baseline cannot grow beyond Git base:\n" + growth.join("\n"));
  if (errors.length) throw new Error(errors.join("\n"));
}

const git = (...args) => execFileSync("git", args, { cwd: repoRoot, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], maxBuffer: 32 * 1024 * 1024 }).trim();
function repositoryFindings(base) {
  const directory = "crates/gwt/web/";
  const files = base ? git("ls-tree", "-r", "--name-only", base, "--", directory).split("\n").filter(file => /^crates\/gwt\/web\/[^/]+\.js$/.test(file))
    : readdirSync(join(repoRoot, directory)).filter(file => file.endsWith(".js")).map(file => directory + file);
  const modules = new Map(files.map(file => [file, base ? git("show", base + ":" + file) : readFileSync(join(repoRoot, file), "utf8")]));
  return files.flatMap(file => scanModule(modules.get(file), file, { modules }));
}

export function checkRepository({ writeBaseline = false, base = process.env.GWT_PULL_RENDER_BASE_SHA } = {}) {
  // Local checks compare against integrated history, so unrelated remote
  // advances cannot change the verdict for the same HEAD. CI's explicit base
  // remains authoritative. Resolve either choice once before reading any tree.
  const comparison = base || git("merge-base", "HEAD", "origin/develop");
  base = git("rev-parse", "--verify", comparison + "^{commit}");
  // Detailed fingerprints are computed from Git source, never persisted. Only
  // the shrinking receiver/case list is committed; first introduction is
  // bounded by actual base findings. Missing refs/read failures fail closed.
  const baseFindings = repositoryFindings(base);
  const hasBaseline = git("ls-tree", "--name-only", base, "--", baselineFile) === baselineFile;
  const ceiling = hasBaseline ? JSON.parse(git("show", base + ":" + baselineFile)) : receiverKeys(baseFindings);
  const findings = repositoryFindings();
  const baseline = writeBaseline ? receiverKeys(findings)
    : JSON.parse(readFileSync(join(repoRoot, baselineFile), "utf8"));
  assertBaseline(findings, baseline, ceiling, baseFindings);
  if (writeBaseline) writeFileSync(join(repoRoot, baselineFile), JSON.stringify(baseline, null, 2) + "\n");
  return { findings: findings.length, base };
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  console.log(JSON.stringify(checkRepository({ writeBaseline: process.argv.includes("--write-baseline") })));
}
