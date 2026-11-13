// SPEC #6943 AC-9: exercise the dependency gate CLI with local lockfiles and
// a deterministic npm process. The real lockfile/registry is checked separately.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repo = fileURLToPath(new URL("..", import.meta.url));
const script = join(repo, "scripts/check-frontend-deps.mjs");
const cleanAudit = { metadata: { vulnerabilities: { high: 0, critical: 0, moderate: 1 } } };

function fixture(t, packages, { exceptions = [], audit = cleanAudit, status = 0 } = {}) {
  const root = mkdtempSync(join(tmpdir(), "gwt-frontend-deps-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const bin = join(root, "bin"), argsFile = join(root, "audit-args.json");
  mkdirSync(bin);
  writeFileSync(join(root, "package-lock.json"), JSON.stringify({ lockfileVersion: 3, packages: { "": {}, ...packages } }));
  writeFileSync(join(root, "exceptions.json"), JSON.stringify(exceptions));
  const npm = join(bin, "npm");
  writeFileSync(npm, `#!/usr/bin/env node
require("node:fs").writeFileSync(${JSON.stringify(argsFile)}, JSON.stringify(process.argv.slice(2)));
process.stdout.write(${JSON.stringify(typeof audit === "string" ? audit : JSON.stringify(audit))});
process.exit(${status});
`);
  chmodSync(npm, 0o755);
  return {
    root, argsFile,
    run: () => spawnSync(process.execPath, [script, "--package-dir", root, "--exceptions", join(root, "exceptions.json")], {
      encoding: "utf8", env: { ...process.env, PATH: `${bin}:${process.env.PATH}` },
    }),
  };
}

test("all approved licenses pass, including optional/dev packages, and npm audits production at high severity", t => {
  const licenses = ["MIT", "ISC", "BSD-2-Clause", "BSD-3-Clause", "Apache-2.0", "0BSD"];
  const f = fixture(t, Object.fromEntries(licenses.map((license, i) => [
    `node_modules/allowed-${i}`, { version: "1.0.0", license, dev: i % 2 === 0, optional: true },
  ])));
  const result = f.run();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /6 locked packages/);
  assert.deepEqual(JSON.parse(readFileSync(f.argsFile, "utf8")), ["audit", "--omit=dev", "--audit-level=high", "--json"]);
});

test("GPL and unknown dev/optional licenses fail with package, license and recovery diagnostics", t => {
  const f = fixture(t, {
    "node_modules/blocked": { version: "1.0.0", license: "GPL-3.0-only", dev: true },
    "node_modules/unknown": { version: "2.0.0", optional: true },
  });
  const result = f.run();
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /blocked@1\.0\.0.*GPL-3\.0-only/);
  assert.match(result.stderr, /unknown@2\.0\.0.*UNKNOWN/);
  assert.match(result.stderr, /Recovery:.*(replace|update)/);
  assert.match(result.stderr, /frontend-license-exceptions\.json/);
});

test("a license exception requires an exact package/version/license and a review reason", t => {
  const exception = { package: "@fixture/blocked", version: "1.0.0", license: "GPL-3.0-only", reason: "Reviewed fixture permission" };
  const f = fixture(t, { "node_modules/@fixture/blocked": { version: "1.0.0", license: exception.license } }, { exceptions: [exception] });
  assert.equal(f.run().status, 0);
  writeFileSync(join(f.root, "exceptions.json"), JSON.stringify([{ ...exception, version: "2.0.0" }]));
  assert.notEqual(f.run().status, 0, "an exception for another version must not apply");
  writeFileSync(join(f.root, "exceptions.json"), JSON.stringify([{ ...exception, reason: " " }]));
  assert.match(f.run().stderr, /exception.*reason/);
});

test("high/critical advisories fail with npm package/advisory details and a recovery command", t => {
  const f = fixture(t, {}, { status: 1, audit: {
    metadata: { vulnerabilities: { high: 1, critical: 1 } },
    vulnerabilities: { affected: { severity: "critical", via: [{ source: 12345, url: "https://example.invalid/advisories/12345" }] } },
  } });
  const result = f.run();
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /affected/);
  assert.match(result.stderr, /12345/);
  assert.match(result.stderr, /Recovery:.*npm audit/);
});

test("registry errors and malformed audit responses fail as unknown rather than passing", t => {
  for (const [audit, status] of [[{ error: { code: "ENOTFOUND", summary: "registry unavailable" } }, 1], ["not-json", 0], [{}, 0]]) {
    const result = fixture(t, {}, { audit, status }).run();
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /outcome unknown/);
    assert.match(result.stderr, /Recovery:/);
  }
});

test("the existing required Cargo Deny job runs dependency tests and the real gate with Node 22", () => {
  const workflow = readFileSync(join(repo, ".github/workflows/lint.yml"), "utf8");
  const audit = workflow.slice(workflow.indexOf("  audit:"), workflow.indexOf("  lint:"));
  assert.match(audit, /node-version: 22/);
  assert.match(audit, /node --test scripts\/check-frontend-deps\.test\.mjs/);
  assert.match(audit, /run: node scripts\/check-frontend-deps\.mjs/);
  assert.doesNotMatch(audit, /continue-on-error/);
});
