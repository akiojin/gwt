// Regressions for scripts/coverage-summary.mjs (Issue #4628).
//
// `cargo` and `llvm-profdata` are replaced by small fakes so the cases run in
// milliseconds: the fake profdata reads a profile iff it ends with `#OK`, and
// the fake cargo copies fixture profiles into the coverage target directory,
// logs every call, and fails a strict merge the way llvm-profdata does.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { inspectProfiles } from "./coverage-summary.mjs";

const scriptsDir = path.dirname(fileURLToPath(import.meta.url));
const SCRIPT = path.join(scriptsDir, "coverage-summary.mjs");
const THRESHOLD = path.join(scriptsDir, "check-coverage-threshold.mjs");
const MAGIC = Buffer.from([0x81, 0x72, 0x66, 0x6f, 0x72, 0x70, 0x6c, 0xff]);

function profile(bodyBytes, { readable = true, magic = MAGIC } = {}) {
  const tail = Buffer.from(readable ? "#OK" : "");
  return Buffer.concat([magic, Buffer.alloc(bodyBytes, 1), tail]);
}

function writeExecutable(file, source) {
  fs.writeFileSync(file, `#!/usr/bin/env node\n${source}`);
  fs.chmodSync(file, 0o755);
}

function sandbox(fixtures, { testExit = 0, covered = 95 } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-coverage-summary-"));
  const fixtureDir = path.join(root, "fixtures");
  const targetDir = path.join(root, "llvm-cov-target");
  fs.mkdirSync(fixtureDir);
  fs.mkdirSync(targetDir);
  for (const [name, bytes] of Object.entries(fixtures)) {
    fs.writeFileSync(path.join(fixtureDir, name), bytes);
  }
  const profdata = path.join(root, "llvm-profdata");
  writeExecutable(
    profdata,
    `const fs = require("node:fs");
const file = process.argv[process.argv.length - 1];
const bytes = fs.readFileSync(file);
if (bytes.length === 0) { console.error("empty raw profile file"); process.exit(1); }
if (!bytes.subarray(-3).equals(Buffer.from("#OK"))) {
  console.error("invalid instrumentation profile data (file header is corrupt)");
  process.exit(1);
}`,
  );
  const cargo = path.join(root, "cargo");
  writeExecutable(
    cargo,
    `const fs = require("node:fs");
const path = require("node:path");
const args = process.argv.slice(2);
const target = process.env.CARGO_LLVM_COV_TARGET_DIR;
fs.appendFileSync(${JSON.stringify(path.join(root, "calls.jsonl"))}, JSON.stringify(args) + "\\n");
const profiles = () => fs.readdirSync(target).filter((n) => n.endsWith(".profraw"));
if (args[1] === "clean") {
  for (const name of profiles()) fs.rmSync(path.join(target, name));
  process.exit(0);
}
if (args.includes("--no-report")) {
  for (const name of fs.readdirSync(${JSON.stringify(fixtureDir)})) {
    fs.copyFileSync(path.join(${JSON.stringify(fixtureDir)}, name), path.join(target, name));
  }
  process.exit(${testExit});
}
if (args[1] === "report") {
  const lenient = args.join(" ").includes("--failure-mode all");
  const unreadable = profiles().filter((name) => {
    const bytes = fs.readFileSync(path.join(target, name));
    return !bytes.subarray(-3).equals(Buffer.from("#OK"));
  });
  if (unreadable.length > 0 && !lenient) {
    console.error("error: failed to merge profile data");
    process.exit(1);
  }
  const out = args[args.indexOf("--output-path") + 1];
  const summary = { data: [{ files: [{ filename: "crates/gwt/src/lib.rs",
    summary: { lines: { covered: ${covered}, count: 100 } } }] }] };
  fs.writeFileSync(out, JSON.stringify(summary));
  process.exit(0);
}
console.error("unexpected cargo call: " + args.join(" "));
process.exit(2);`,
  );
  const output = path.join(root, "coverage-summary.json");
  return { root, targetDir, profdata, cargo, output };
}

function run(box, extraArgs = ["--workspace", "--all-features"]) {
  const result = spawnSync(
    process.execPath,
    [SCRIPT, "--output-path", box.output, "--", ...extraArgs],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        CARGO: box.cargo,
        LLVM_PROFDATA: box.profdata,
        CARGO_LLVM_COV_TARGET_DIR: box.targetDir,
      },
    },
  );
  const calls = fs.existsSync(path.join(box.root, "calls.jsonl"))
    ? fs
        .readFileSync(path.join(box.root, "calls.jsonl"), "utf8")
        .trim()
        .split("\n")
        .map((line) => JSON.parse(line))
    : [];
  return { ...result, calls, text: `${result.stdout}${result.stderr}` };
}

const reportCalls = (calls) => calls.filter((args) => args[1] === "report");

test("inspectProfiles classifies healthy, truncated, empty, malformed, and corrupt profiles", async () => {
  const box = sandbox({});
  const write = (name, bytes) => fs.writeFileSync(path.join(box.targetDir, name), bytes);
  write("gwt-100-111_0.profraw", profile(4096));
  write("gwt-101-111_0.profraw", profile(1024, { readable: false }));
  write("gwt-102-111_0.profraw", Buffer.alloc(0));
  write("gwt-103-222_0.profraw", profile(4096, { magic: Buffer.from("notaprof") }));
  write("gwt-104-333_0.profraw", profile(4096, { readable: false }));

  const report = await inspectProfiles(box.targetDir, box.profdata);
  const kinds = Object.fromEntries(report.profiles.map((entry) => [entry.name, entry.kind]));

  assert.deepEqual(kinds, {
    "gwt-100-111_0.profraw": "healthy",
    "gwt-101-111_0.profraw": "truncated",
    "gwt-102-111_0.profraw": "truncated",
    "gwt-103-222_0.profraw": "malformed",
    // Unreadable with a valid header, but no readable profile of the same
    // module proves the full length: not claimed as truncation.
    "gwt-104-333_0.profraw": "corrupt",
  });
});

test("(a) healthy profiles aggregate with the strict merge", () => {
  const box = sandbox({ "gwt-1-111_0.profraw": profile(4096) });
  const result = run(box);

  assert.equal(result.status, 0, result.text);
  const reports = reportCalls(result.calls);
  assert.equal(reports.length, 1);
  assert.ok(!reports[0].includes("--failure-mode"), reports[0].join(" "));
  assert.ok(!reports[0].includes("--workspace"), "report does not accept --workspace");
  assert.ok(reports[0].includes("--all-features"), "report keeps the run's feature scope");
  assert.ok(fs.existsSync(box.output));
  assert.match(result.text, /coverage-summary: PASS/);
});

test("(b) a truncated profile is reported apart from tests and re-aggregated once", () => {
  const box = sandbox({
    "gwt-1-111_0.profraw": profile(4096),
    "gwt-2-111_0.profraw": profile(1024, { readable: false }),
  });
  const result = run(box, ["-p", "gwt-core", "-p", "gwt", "--all-features"]);

  assert.equal(result.status, 0, result.text);
  const reports = reportCalls(result.calls);
  assert.equal(reports.length, 1, "exactly one aggregation, the conservative one");
  assert.deepEqual(reports[0].slice(reports[0].indexOf("--failure-mode"), reports[0].indexOf("--failure-mode") + 2), ["--failure-mode", "all"]);
  assert.ok(reports[0].includes("gwt-core"), "report keeps the run's package scope");
  assert.match(result.text, /RECOVERED \[profile-truncated\]/);
  assert.match(result.text, /tests passed/);
  assert.match(result.text, /gwt-2-111_0\.profraw/);
  const health = JSON.parse(fs.readFileSync(`${box.output}.profiles.json`, "utf8"));
  assert.equal(health.classification, "profile-truncated");
  assert.equal(health.recovered, true);
  assert.deepEqual(health.damaged.map((entry) => entry.name), ["gwt-2-111_0.profraw"]);
  assert.ok(
    fs.existsSync(path.join(box.targetDir, "gwt-2-111_0.profraw")),
    "the damaged profile is kept as evidence",
  );
});

test("(c) a malformed profile fails as profile damage without aggregating", () => {
  const box = sandbox({
    "gwt-1-111_0.profraw": profile(4096),
    "gwt-2-222_0.profraw": profile(4096, { magic: Buffer.from("notaprof") }),
  });
  const result = run(box);

  assert.equal(result.status, 3, result.text);
  assert.equal(reportCalls(result.calls).length, 0);
  assert.match(result.text, /FAIL \[profile-damage\]/);
  assert.match(result.text, /tests passed/);
  assert.ok(!fs.existsSync(box.output));
});

test("a test failure is reported as a test failure and never aggregated", () => {
  const box = sandbox({ "gwt-1-111_0.profraw": profile(4096) }, { testExit: 101 });
  const result = run(box);

  assert.equal(result.status, 101, result.text);
  assert.equal(reportCalls(result.calls).length, 0);
  assert.match(result.text, /FAIL \[test-failure\]/);
});

test("a stale summary never survives a failed run", () => {
  const box = sandbox({ "gwt-1-111_0.profraw": profile(4096) }, { testExit: 101 });
  fs.writeFileSync(box.output, "{}");
  run(box);
  assert.ok(!fs.existsSync(box.output), "an old summary would let the threshold pass");
});

test("the threshold still fails after a conservative re-aggregation below it", () => {
  const box = sandbox(
    {
      "gwt-1-111_0.profraw": profile(4096),
      "gwt-2-111_0.profraw": profile(1024, { readable: false }),
    },
    { covered: 85 },
  );
  const result = run(box);
  assert.equal(result.status, 0, result.text);
  assert.match(result.text, /RECOVERED/);

  const threshold = spawnSync(process.execPath, [THRESHOLD, box.output, "90"], {
    encoding: "utf8",
  });
  assert.equal(threshold.status, 1, `${threshold.stdout}${threshold.stderr}`);
  assert.match(threshold.stderr, /Coverage threshold not met/);
});
