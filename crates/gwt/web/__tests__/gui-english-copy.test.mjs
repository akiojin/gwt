import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'acorn';
import { parseHTML } from 'linkedom';

const webRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const rustRoot = resolve(webRoot, '../src');
const cjk = /[\u3040-\u30ff\u4e00-\u9fff]/u;

function filesUnder(root) {
  return readdirSync(root, { withFileTypes: true }).flatMap(entry => {
    if (['__tests__', 'vendor', 'tests'].includes(entry.name)) return [];
    const path = join(root, entry.name);
    return entry.isDirectory() ? filesUnder(path) : [path];
  });
}

function javascriptCopy(source, path) {
  const violations = [];
  const visit = (node, parent, grandparent) => {
    if (!node || typeof node !== 'object') return;
    const value = node.type === 'Literal' ? node.value
      : node.type === 'TemplateElement' ? node.value.cooked ?? node.value.raw : null;
    // The language picker displays a language's own name, not translated chrome.
    const endonym = path === 'settings-surface.js' && value === '日本語'
      && parent?.type === 'Property' && parent.key.name === 'text'
      && grandparent?.type === 'ObjectExpression'
      && grandparent.properties.some(property => property.key?.name === 'value' && property.value.value === 'ja');
    if (typeof value === 'string' && cjk.test(value) && !endonym) {
      violations.push(`${path}:${node.loc.start.line}: ${JSON.stringify(value)}`);
    }
    for (const [key, child] of Object.entries(node)) {
      if (key !== 'loc') {
        if (Array.isArray(child)) child.forEach(item => visit(item, node, parent));
        else if (child && typeof child === 'object') visit(child, node, parent);
      }
    }
  };
  visit(parse(source, { ecmaVersion: 'latest', sourceType: 'module', locations: true }));
  return violations;
}

test('GUI JavaScript and HTML fixed copy stays English', () => {
  const violations = [];
  for (const file of filesUnder(webRoot).filter(path => /\.(js|html)$/.test(path))) {
    const path = relative(webRoot, file);
    const source = readFileSync(file, 'utf8');
    if (file.endsWith('.js')) violations.push(...javascriptCopy(source, path));
    else {
      const { document } = parseHTML(source);
      const visit = node => {
        if (node.nodeType === 8) return; // Authored HTML comments are not UI copy.
        if (node.nodeName === 'SCRIPT') {
          violations.push(...javascriptCopy(node.textContent, `${path} inline script`));
          return;
        }
        for (const attribute of node.attributes || []) {
          if (cjk.test(attribute.value)) violations.push(`${path}: ${attribute.name}=${attribute.value}`);
        }
        if (node.nodeType === 3 && cjk.test(node.textContent)) violations.push(`${path}: ${node.textContent.trim()}`);
        for (const child of node.childNodes) visit(child);
      };
      visit(document);
    }
  }
  assert.deepEqual(violations, [], 'Fixed GUI copy must be English; PM/Issue/Board content remains authored data.');
});

function rustCopy(source, path) {
  // Lex only strings/comments/braces. This gate is deliberately not a Rust parser.
  const tokens = [...source.matchAll(/\/\/[^\n]*|\/\*[\s\S]*?\*\/|\br(#+)?"[\s\S]*?"\1|"(?:\\[\s\S]|[^"\\])*"|'(?:\\.|[^'\\])'|[{}]/g)];
  const testModules = [...source.matchAll(/#\[cfg\(test\)\](?:\s*#\[[^\]]*\])*\s*mod\s+\w+\s*\{/g)].map(match => match.index + match[0].length - 1);
  const violations = [];
  let testDepth = 0;
  for (const token of tokens) {
    const value = token[0];
    if (testModules.includes(token.index)) testDepth = 1;
    else if (testDepth) {
      if (value === '{') testDepth++;
      if (value === '}') testDepth--;
    } else if (!value.startsWith('//') && !value.startsWith('/*') && cjk.test(value)) {
      const line = source.slice(0, token.index).split('\n').length;
      violations.push(`${path}:${line}: ${value}`);
    }
  }
  return violations;
}

test('Rust GUI event producers and launch wizard fixed copy stays English', () => {
  const violations = [];
  for (const file of filesUnder(rustRoot).filter(path => path.endsWith('.rs') && !/(?:\btests|_tests|test_support)\.rs$/.test(path))) {
    const path = relative(rustRoot, file);
    const source = readFileSync(file, 'utf8');
    // CLI/agent guidance and native provider-reset consent are distinct from WebView copy.
    if (path.startsWith('cli/') || path.startsWith('bin/')) continue;
    if (!path.startsWith('app_runtime/') && !path.startsWith('launch_wizard/')
      && !['pm_current.rs', 'protocol.rs'].includes(path) && !/\bBackendEvent\s*(?:::|\{)/.test(source)) continue;
    violations.push(...rustCopy(source, path));
  }
  assert.deepEqual(violations, [], 'Fixed WebView event/error copy must be English.');
});
