const { spawnSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const repoRoot = path.resolve(__dirname, '..');
const nodeModules = path.join(repoRoot, 'node_modules');
const backup = path.join(repoRoot, `node_modules.commitlintbak-${process.pid}`);

const expectedTypeEnum = [
  'feat',
  'fix',
  'docs',
  'style',
  'refactor',
  'perf',
  'test',
  'build',
  'ci',
  'chore',
  'revert',
];

const expectedRules = {
  'subject-empty': [2, 'never'],
  'header-max-length': [2, 'always', 100],
  'subject-max-length': [2, 'always', 100],
  'type-enum': [2, 'always', expectedTypeEnum],
};

const ruleMatches = (actual, expected) =>
  JSON.stringify(actual) === JSON.stringify(expected);

let moved = false;
if (fs.existsSync(nodeModules)) {
  fs.renameSync(nodeModules, backup);
  moved = true;
}

try {
  const result = spawnSync(
    'bunx',
    ['--package', '@commitlint/cli', 'commitlint', '--from', 'HEAD~1', '--to', 'HEAD'],
    { cwd: repoRoot, env: process.env, encoding: 'utf8' },
  );

  if (result.status !== 0) {
    console.error('commitlint config fallback test failed');
    if (result.stdout) {
      console.error(result.stdout);
    }
    if (result.stderr) {
      console.error(result.stderr);
    }
    process.exitCode = result.status || 1;
    return;
  }

  const config = require(path.join(repoRoot, 'commitlint.config.cjs'));
  const rules = config.rules || {};

  for (const [name, expected] of Object.entries(expectedRules)) {
    if (!ruleMatches(rules[name], expected)) {
      console.error(`commitlint config rule mismatch: ${name}`);
      console.error(`expected: ${JSON.stringify(expected)}`);
      console.error(`actual: ${JSON.stringify(rules[name])}`);
      process.exitCode = 1;
      return;
    }
  }

  console.log('commitlint config fallback: ok');
} finally {
  if (moved) {
    fs.renameSync(backup, nodeModules);
  }
}
