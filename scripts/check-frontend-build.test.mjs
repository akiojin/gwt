// SPEC #6943 AC-2: scripts/check-frontend-build.sh detects a stale committed
// React bundle and accepts a fresh one. Requires crates/gwt/web-react/node_modules
// (run `npm ci --prefix crates/gwt/web-react --ignore-scripts` first).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repo = fileURLToPath(new URL("..", import.meta.url));
const script = join(repo, "scripts/check-frontend-build.sh");

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "gwt-frontend-build-"));
  const pkg = join(root, "crates/gwt/web-react");
  mkdirSync(pkg, { recursive: true });
  for (const entry of ["package.json", "package-lock.json", "build.mjs", "src"]) {
    cpSync(join(repo, "crates/gwt/web-react", entry), join(pkg, entry), { recursive: true });
  }
  symlinkSync(join(repo, "crates/gwt/web-react/node_modules"), join(pkg, "node_modules"), "dir");
  mkdirSync(join(root, "crates/gwt/web/react"), { recursive: true });
  cpSync(join(repo, "crates/gwt/web/react/islands.js"), join(root, "crates/gwt/web/react/islands.js"));
  return root;
}

function check(root) {
  return spawnSync("bash", [script], {
    encoding: "utf8",
    env: { ...process.env, GWT_FRONTEND_BUILD_ROOT: root, GWT_FRONTEND_SKIP_NPM_CI: "1" },
  });
}

test("committed bundle that matches the sources passes", () => {
  const root = fixture();
  try {
    const result = check(root);
    assert.equal(result.status, 0, result.stderr);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("source change without a regenerated bundle fails with a recovery hint", () => {
  const root = fixture();
  try {
    const surface = join(root, "crates/gwt/web-react/src/surfaces/WorkspaceStateNotice.jsx");
    writeFileSync(surface, readFileSync(surface, "utf8").replace(">Retry<", ">Retry now<"));
    const result = check(root);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /islands\.js is stale/);
    assert.match(result.stderr, /Recovery: .*build\.mjs/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("the committed bundle is checked out with LF on every platform", () => {
  const result = spawnSync("git", ["check-attr", "eol", "crates/gwt/web/react/islands.js"], { cwd: repo, encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /eol: lf/);
});
