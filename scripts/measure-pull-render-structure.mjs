#!/usr/bin/env node
// AC-6/11 source audit only. This tool never starts the AC-17 performance harness.
import { parse, version as acornVersion } from 'acorn';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, realpathSync } from 'node:fs';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { rootPattern, scanModule } from './check-pull-render.mjs';

const repositoryRoot = dirname(dirname(realpathSync(fileURLToPath(import.meta.url))));
const beforeSha = '046f94d660ba3fa823f244dc1ac92774b0215fbf';
const historicalSha = '20ab332647ec659176ef2bdb431bc539a569011f';
const appFile = 'crates/gwt/web/app.js';
const pmFile = 'crates/gwt/web/pm-chat.js';
const patterns = { appendChild: 'appendChild', textContent_assignment: 'textContent *=', subscribe: 'subscribe\\(', fetch: 'fetch\\(' };
const receiverKinds = new Set(['pm_conversation', 'pm_reports', 'terminal_output', 'terminal_snapshot']);
const domCalls = new Set('append appendChild prepend replaceChildren replaceChild remove removeChild insertBefore insertAdjacentHTML setAttribute removeAttribute createElement createTextNode focus blur'.split(' '));
const domWrites = new Set('innerHTML outerHTML textContent innerText className value checked disabled hidden scrollTop scrollLeft'.split(' '));
const children = node => Object.values(node).flatMap(value => Array.isArray(value) ? value : [value])
  .filter(value => value && typeof value.type === 'string');
const property = node => node?.type === 'MemberExpression' ? (node.computed ? node.property.value : node.property.name) : null;
const memberPath = node => node?.type === 'Identifier' ? node.name : node?.type === 'MemberExpression' ? `${memberPath(node.object)}.${property(node)}` : '';
const lexical = source => Object.fromEntries(Object.entries(patterns).map(([name, pattern]) => [name, [...source.matchAll(new RegExp(pattern, 'g'))].length]));
const git = (args, repository) => execFileSync('git', args,
  { cwd: repository, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 32 * 1024 * 1024 });

export function readSourceTree(revision, repository = repositoryRoot) {
  const sha = git(['rev-parse', '--verify', '--end-of-options', `${revision}^{commit}`], repository).trim();
  const files = git(['ls-tree', '-r', '--name-only', sha, '--', 'crates/gwt/web/'], repository).trim().split('\n')
    .filter(file => /^crates\/gwt\/web\/[^/]+\.js$/.test(file));
  return { sha, modules: new Map(files.map(file => [file, git(['show', `${sha}:${file}`], repository)])) };
}

function walk(node, visit, parent = null) {
  visit(node, parent);
  for (const child of children(node)) walk(child, visit, node);
}

function findNode(node, predicate, label) {
  let found;
  walk(node, candidate => { if (!found && predicate(candidate)) found = candidate; });
  if (!found) throw new Error(`Missing audit scope: ${label}`);
  return found;
}

function domScope(file, scope, node) {
  const construction_sites = [], dom_operation_sites = [], xterm_entry_sites = [];
  const site = (kind, node) => ({ kind, line: node.loc.start.line, column: node.loc.start.column + 1 });
  walk(node, current => {
    const method = property(current.callee);
    if (current.type === 'CallExpression' && (domCalls.has(method) || /\.(style|classList)\./.test(memberPath(current.callee)))) {
      dom_operation_sites.push(site(method, current));
      if (['createElement', 'createTextNode', 'insertAdjacentHTML'].includes(method)) construction_sites.push(site(method, current));
    }
    if (['AssignmentExpression', 'UpdateExpression'].includes(current.type)) {
      const target = current.left || current.argument;
      if (domWrites.has(property(target)) || /\.(style|dataset)(\.|$)/.test(memberPath(target))) {
        dom_operation_sites.push(site(`${memberPath(target)}${current.operator}`, current));
        if (['innerHTML', 'outerHTML'].includes(property(target))) construction_sites.push(site(`${property(target)}=`, current));
      }
    }
    if (current.type === 'NewExpression' && current.callee.name === 'Terminal') xterm_entry_sites.push(site('new Terminal', current));
    if (current.type === 'CallExpression' && memberPath(current.callee) === 'terminal.open') xterm_entry_sites.push(site('terminal.open', current));
  });
  return { file, scope, line: node.loc.start.line, construction_sites, dom_operation_sites, xterm_entry_sites };
}

export function measureModules(modules) {
  const lexical_files = Object.fromEntries([...modules].map(([file, source]) => [file, lexical(source)]));
  const lexical_totals = Object.fromEntries(Object.keys(patterns).map(name => [name,
    Object.values(lexical_files).reduce((total, counts) => total + counts[name], 0)]));
  const asts = new Map([appFile, pmFile, 'crates/gwt/web/ui-content.js'].map(file => {
    if (!modules.has(file)) throw new Error(`Missing audit module: ${file}`);
    return [file, parse(modules.get(file), { ecmaVersion: 'latest', sourceType: 'module', locations: true })];
  }));
  const named = (file, name) => findNode(asts.get(file), node => node.type === 'FunctionDeclaration' && node.id.name === name, `${file}:${name}`);
  const dom_scopes = [
    domScope(pmFile, 'createPmChat', named(pmFile, 'createPmChat')),
    domScope('crates/gwt/web/ui-content.js', 'renderUiContent (shared renderer inventory)', named('crates/gwt/web/ui-content.js', 'renderUiContent')),
    ...['syncPmChat', 'createTerminalRuntime', 'reparentTerminalRuntime', 'writeOutput', 'replaceTerminalSnapshot']
      .map(name => domScope(appFile, name, named(appFile, name))),
  ];
  const mount = named(appFile, 'mountWindowBody');
  const reset = mount.body.body.find(node => node.type === 'ExpressionStatement'
    && node.expression.type === 'AssignmentExpression' && property(node.expression.left) === 'innerHTML');
  if (!reset) throw new Error('Missing audit scope: mountWindowBody:reset');
  const terminal = findNode(mount, node => node.type === 'IfStatement' && node.test.type === 'BinaryExpression'
    && node.test.operator === '===' && node.test.left.name === 'surface' && node.test.right.value === 'terminal', 'mountWindowBody:terminal');
  dom_scopes.push(domScope(appFile, 'mountWindowBody:reset', reset), domScope(appFile, 'mountWindowBody:terminal', terminal.consequent));

  const source = modules.get(appFile);
  const receive = named(appFile, 'receive');
  const arms = [];
  walk(receive, node => { if (node.type === 'SwitchCase' && receiverKinds.has(node.test?.value)) arms.push(node); });
  const event_kinds = arms.map(arm => arm.test.value);
  if (!event_kinds.includes('terminal_output') || !event_kinds.includes('terminal_snapshot')
    || !event_kinds.some(kind => kind === 'pm_reports' || kind === 'pm_conversation')) throw new Error('Missing Reports/RAW receiver cases');
  const relevant = findings => findings.filter(finding => finding.root === 'receive'
    && finding.context.split(/[\/|]/).some(part => receiverKinds.has(part)));
  const findings = relevant(scanModule(source, appFile, { modules }));
  const bypass_probes = arms.map(arm => {
    const offset = source.indexOf(':', arm.test.end) + 1;
    const mutated = source.slice(0, offset) + ' document.body.textContent = event.kind;' + source.slice(offset);
    const detected = relevant(scanModule(mutated, appFile, { modules })).filter(finding => finding.kind === 'dom-write'
      && finding.context.split(/[\/|]/).includes(arm.test.value));
    return { event_kind: arm.test.value, detected: detected.length > 0, findings: detected };
  });
  const all_receiver_audit = { scanned_modules: modules.size, roots: [], findings: [] };
  for (const [file, source] of modules) {
    const ast = asts.get(file) || parse(source, { ecmaVersion: 'latest', sourceType: 'module', locations: true });
    walk(ast, (node, parent) => {
      if (!/^(FunctionDeclaration|FunctionExpression|ArrowFunctionExpression)$/.test(node.type)) return;
      const name = node.id?.name || (parent?.type === 'VariableDeclarator' && parent.id.type === 'Identifier' ? parent.id.name : '<callback>');
      if (!rootPattern.test(name)) return;
      const root = { file, root: name, line: node.loc.start.line, switch_labels: [], if_statements: 0 };
      walk(node.body, child => {
        if (child.type === 'SwitchCase' && child.test?.type === 'Literal') {
          const value = typeof child.test.value === 'bigint' ? child.test.value.toString() : child.test.value;
          root.switch_labels.push({ value, line: child.loc.start.line });
        }
        if (child.type === 'IfStatement') root.if_statements += 1;
      });
      all_receiver_audit.roots.push(root);
    });
    all_receiver_audit.findings.push(...scanModule(source, file, { modules }));
  }
  return { files: modules.size, lexical_totals, lexical_files, dom_scopes,
    receiver_audit: { event_kinds, findings, bypass_probes, probes_modify_committed_source: false }, all_receiver_audit };
}

export function measureStructure({ after = 'HEAD' } = {}) {
  // Resolve each revision once, and read all measured sources from that tree.
  const afterTree = readSourceTree(after);
  const beforeTree = readSourceTree(beforeSha);
  const historicalTree = readSourceTree(historicalSha);
  const recorded = { files: 66, appendChild: 1098, textContent_assignment: 285, subscribe: 3, fetch: 1 };
  const recounted = { files: historicalTree.modules.size, ...lexical([...historicalTree.modules.values()].join('\n')) };
  const hash = file => createHash('sha256').update(readFileSync(new URL(file, import.meta.url))).digest('hex');
  return {
    schema: 'gwt-pull-render-structure/v1',
    source: { kind: 'committed Git trees', working_tree_included: false,
      working_tree_dirty: git(['status', '--porcelain', '--untracked-files=normal'], repositoryRoot).trim().length > 0 },
    method: { files: 'crates/gwt/web/[^/]+.js (top-level only)', lexical_patterns: patterns,
      lexical_counting: 'Equivalent to grep -rhoE <pattern> web/*.js | wc -l; includes comments and strings.',
      dom_scopes: 'Named function syntax, including nested callbacks; terminal mount branch and common reset are separate. Shared renderer inventory does not imply receiver reachability.',
      construction_sites: 'createElement/createTextNode/insertAdjacentHTML calls and innerHTML/outerHTML assignments; one location per syntactic site, not runtime node instances.',
      dom_operation_sites: 'DOM property writes and method spellings recognized by the existing guard; inventories may include non-DOM objects with the same spelling.',
      guard: 'scripts/check-pull-render.mjs:scanModule', guard_sha256: hash('./check-pull-render.mjs'),
      tool_sha256: hash('./measure-pull-render-structure.mjs'), acorn_version: acornVersion,
      receiver_roots: rootPattern.source,
      root_inventory: 'All modules; named functions and variable-bound function expressions use the guard naming convention (an explicit function name takes precedence). Switch literal labels and if statements include nested callbacks, may overlap across roots, and are syntax inventories, not unique event counts or proof of reachability. BigInt literal values are decimal strings to keep the report JSON serializable without losing precision.',
      guard_limitations: 'Scoped syntax guard, not whole-program/type analysis. Follows local named helpers and up to two hops of named imports/factory return aliases; known data API spellings are trusted. Other unresolved calls remain findings. Model notifications enter subscribed renderers by design; zero findings means no bypass found within the named-root scope. WebSocket entrypoint routing must also be inspected.',
      xterm: 'new Terminal/terminal.open entry sites are listed separately; vendor renderer internals are outside the original top-level scope.',
      receiver_history: 'pm_conversation was replaced by durable pm_reports; the common audit includes both spellings and the two RAW events. Before findings may be unresolved calls, not direct DOM-write findings.',
    },
    before: { sha: beforeTree.sha, ...measureModules(beforeTree.modules) },
    after: { sha: afterTree.sha, ...measureModules(afterTree.modules) },
    historical_baseline: { sha: historicalTree.sha, recorded, recounted,
      status: Object.keys(recorded).every(key => recorded[key] === recounted[key]) ? 'reproduced' : 'not_reproduced',
      explanation: 'The Issue opening cites this running-build SHA but records different checkout totals. The original measured checkout is unproven; preserve those historical claims without treating them as this fixed migration baseline.' },
    performance_samples_collected: 0,
  };
}

if (process.argv[1] && realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))) {
  try {
    const { values } = parseArgs({ options: { after: { type: 'string', default: 'HEAD' } } });
    console.log(JSON.stringify(measureStructure({ after: values.after }), null, 2));
  } catch (error) {
    console.error(`Structure audit failed: ${error.message}`);
    process.exitCode = 1;
  }
}
