#!/usr/bin/env node

// Run the coverage tests and aggregate their summary, keeping a damaged raw
// profile apart from a failing test (Issue #4628).
//
//   node scripts/coverage-summary.mjs --output-path <summary.json> -- <cargo llvm-cov args>
// Add --nextest before -- to run the same inventory in isolated parallel processes.
// Export a shard with --nextest --partition slice:N/M --export-dir DIR.
// Aggregate downloaded workspace-tests-N directories with --report-from ROOT
// --shards M --output-path summary.json [--lcov-output-path lcov.info].
//
// `cargo llvm-cov` fails the whole run when one raw profile cannot be merged,
// which reads exactly like a test regression. This script runs the tests with
// `--no-report`, reads every raw profile with the same llvm-profdata the merge
// uses, and only then aggregates:
//
//   exit <cargo's>  FAIL [test-failure]    the tests failed; nothing aggregated
//   exit 3          FAIL [profile-damage]  tests passed; a profile is malformed or
//                                          unreadable and not provably truncated
//   exit 4          FAIL [report-failure]  tests passed; the aggregation failed
//   exit 0          RECOVERED [profile-truncated]  tests passed; truncated profiles
//                   were left out by ONE `report --failure-mode all` aggregation
//   exit 0          PASS
//
// A recovery is written to `<summary.json>.profiles.json` and never hides the
// damaged files. Leaving a profile out can only lower coverage, so the
// threshold check that follows (check-coverage-threshold.mjs) is unchanged.
//
// `CARGO`, `LLVM_PROFDATA`, and `CARGO_LLVM_COV_TARGET_DIR` are honoured the
// same way cargo-llvm-cov honours them.

import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

// __llvm_profile_raw_magic ("lprofr" + 0x81 + 0xff), 64- and 32-bit variants,
// in either byte order.
const RAW_MAGICS = [
  Buffer.from([0x81, 0x72, 0x66, 0x6f, 0x72, 0x70, 0x6c, 0xff]),
  Buffer.from([0x81, 0x52, 0x66, 0x6f, 0x72, 0x70, 0x6c, 0xff]),
].flatMap((magic) => [magic, Buffer.from(magic).reverse()]);

const PROFILE_DAMAGE_EXIT = 3;
const REPORT_FAILURE_EXIT = 4;

function hasRawMagic(file) {
  const header = Buffer.alloc(8);
  const fd = fs.openSync(file, "r");
  try {
    if (fs.readSync(fd, header, 0, 8, 0) < 8) {
      return false;
    }
  } finally {
    fs.closeSync(fd);
  }
  return RAW_MAGICS.some((magic) => magic.equals(header));
}

// `%Nm` expands to `<module signature>_<pool slot>`. Ignore the slot for
// both pooled names and cargo-llvm-cov's default `<name>-%p-%Nm.profraw`.
function moduleOf(name) {
  const stem = name.replace(/\.profraw$/, "");
  return stem.slice(stem.lastIndexOf("-") + 1).replace(/_\d+$/, "");
}

function readsCleanly(profdata, file) {
  return new Promise((resolve) => {
    const child = spawn(profdata, ["show", file], {
      stdio: ["ignore", "ignore", "pipe"],
    });
    let stderr = "";
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", (error) => resolve({ ok: false, detail: error.message }));
    child.on("close", (code) =>
      resolve({ ok: code === 0, detail: stderr.trim() }),
    );
  });
}

/**
 * Read every raw profile in `dir` and classify it.
 *
 * - `healthy`: llvm-profdata reads it.
 * - `truncated`: unreadable, and either empty or shorter than a readable
 *   profile of the same module. Every process of one binary writes the same
 *   fixed-size sections, so a shorter unreadable file is an interrupted write.
 * - `malformed`: not a raw profile header at all.
 * - `corrupt`: unreadable with a valid header and no proof of truncation.
 */
export async function inspectProfiles(dir, profdata) {
  const names = fs.existsSync(dir)
    ? fs
        .readdirSync(dir)
        .filter((name) => name.endsWith(".profraw"))
        .sort()
    : [];
  const entries = names.map((name) => {
    const file = path.join(dir, name);
    return { name, file, size: fs.statSync(file).size, module: moduleOf(name) };
  });

  const limit = Math.max(
    1,
    Math.min(8, Math.floor(os.availableParallelism() / 2)),
  );
  let next = 0;
  await Promise.all(
    Array.from({ length: limit }, async () => {
      while (next < entries.length) {
        const entry = entries[next++];
        if (entry.size === 0) {
          entry.kind = "truncated";
          entry.detail = "empty raw profile";
        } else if (!hasRawMagic(entry.file)) {
          entry.kind = "malformed";
          entry.detail = "no raw profile magic";
        } else {
          const read = await readsCleanly(profdata, entry.file);
          entry.kind = read.ok ? "healthy" : "unreadable";
          entry.detail = read.detail;
        }
      }
    }),
  );

  for (const entry of entries.filter(
    (candidate) => candidate.kind === "unreadable",
  )) {
    const fullSize = Math.max(
      0,
      ...entries
        .filter(
          (other) => other.kind === "healthy" && other.module === entry.module,
        )
        .map((other) => other.size),
    );
    entry.kind = fullSize > entry.size ? "truncated" : "corrupt";
    if (entry.kind === "truncated") {
      entry.detail = `${entry.size} of ${fullSize} bytes; ${entry.detail}`;
    }
  }

  return {
    profiles: entries.map(({ name, size, kind, detail }) => ({
      name,
      size,
      kind,
      detail,
    })),
  };
}

function hostTriple() {
  const version = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
  return /^host: (.+)$/m.exec(version.stdout ?? "")?.[1];
}

function resolveProfdata() {
  if (process.env.LLVM_PROFDATA) {
    return process.env.LLVM_PROFDATA;
  }
  const sysroot = spawnSync("rustc", ["--print", "sysroot"], {
    encoding: "utf8",
  }).stdout?.trim();
  const exe =
    process.platform === "win32" ? "llvm-profdata.exe" : "llvm-profdata";
  return path.join(
    sysroot ?? "",
    "lib",
    "rustlib",
    hostTriple() ?? "",
    "bin",
    exe,
  );
}

function resolveProfileDir(cargo) {
  if (process.env.CARGO_LLVM_COV_TARGET_DIR) {
    return process.env.CARGO_LLVM_COV_TARGET_DIR;
  }
  const metadata = spawnSync(
    cargo,
    ["metadata", "--format-version", "1", "--no-deps"],
    {
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    },
  );
  if (metadata.status !== 0) {
    throw new Error(`cargo metadata failed: ${metadata.stderr}`);
  }
  return path.join(
    JSON.parse(metadata.stdout).target_directory,
    "llvm-cov-target",
  );
}

// `cargo llvm-cov report` must see the run's package scope and build profile,
// but refuses the test run's other selectors (`--workspace`, feature and
// target flags: "invalid option '--all-features' for subcommand 'report'").
const REPORT_FLAGS = new Set(["-r", "--release"]);
const REPORT_OPTIONS = new Set([
  "-p",
  "--package",
  "--profile",
  "--manifest-path",
  "--target",
]);

function reportScope(args) {
  const scope = [];
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (arg === "--") {
      break;
    }
    const option = arg.split("=", 1)[0];
    if (
      REPORT_FLAGS.has(arg) ||
      (arg.includes("=") && REPORT_OPTIONS.has(option))
    ) {
      scope.push(arg);
    } else if (REPORT_OPTIONS.has(arg) && i + 1 < args.length) {
      scope.push(arg, args[i + 1]);
      i += 1;
    }
  }
  return scope;
}

function cargoLlvmCov(cargo, args, env = process.env) {
  const result = spawnSync(cargo, ["llvm-cov", ...args], {
    stdio: "inherit",
    env,
  });
  if (result.error) {
    throw result.error;
  }
  return result.status ?? 1;
}

function coverageEnvironment(cargo, runArgs, profileDir) {
  const args = runArgs.slice(
    0,
    runArgs.includes("--") ? runArgs.indexOf("--") : runArgs.length,
  );
  // show-env rejects test selectors such as --all-features. Only flags that
  // change instrumentation belong here; build selection stays on cargo test.
  const envFlags = new Set([
    "--coverage-target-only",
    "--remap-path-prefix",
    "--include-ffi",
    "--no-cfg-coverage",
    "--no-cfg-coverage-nightly",
    "--no-rustc-wrapper",
  ]);
  const envArgs = [];
  for (let i = 0; i < args.length; i += 1) {
    if (envFlags.has(args[i]) || /^(--target|--manifest-path)=/.test(args[i])) {
      envArgs.push(args[i]);
    } else if (args[i] === "--target" || args[i] === "--manifest-path") {
      envArgs.push(args[i], args[++i]);
    }
  }
  const environment = spawnSync(
    cargo,
    ["llvm-cov", "show-env", "--sh", ...envArgs],
    {
      encoding: "utf8",
      env: { ...process.env, CARGO_TARGET_DIR: profileDir },
      stdio: ["ignore", "pipe", "inherit"],
    },
  );
  if (environment.status !== 0)
    throw new Error("could not read the coverage build environment");
  const env = { ...process.env, CARGO_TARGET_DIR: profileDir };
  // Decode cargo-llvm-cov's POSIX assignments as data, never execute them.
  for (const line of environment.stdout.split(/\r?\n/)) {
    if (!line) continue;
    const match = /^export ([A-Z_][A-Z_0-9]*)=(.*)$/.exec(line);
    if (!match)
      throw new Error(`invalid coverage environment assignment: ${line}`);
    const [, key, value] = match;
    env[key] = value.startsWith("'")
      ? value.slice(1, -1).replaceAll("'\\''", "'")
      : value;
  }
  return env;
}

function currentObjects(cargo, runArgs, profileDir, env) {
  const args = runArgs.slice(
    0,
    runArgs.includes("--") ? runArgs.indexOf("--") : runArgs.length,
  );
  env ??= coverageEnvironment(cargo, runArgs, profileDir);
  // These switches configure llvm-cov rather than cargo test. The remaining
  // package, feature, profile and target selectors must match the test run.
  const llvmFlags = new Set([
    "--no-clean",
    "--no-report",
    "--no-cfg-coverage",
    "--no-cfg-coverage-nightly",
    "--no-rustc-wrapper",
    "--remap-path-prefix",
    "--coverage-target-only",
    "--include-ffi",
  ]);
  const buildArgs = args.filter((arg) => !llvmFlags.has(arg));
  if (
    !buildArgs.some((arg) =>
      /^--(lib|bins?|examples?|tests?|bench(es)?|all-targets|doc)(=|$)/.test(
        arg,
      ),
    )
  ) {
    // Match cargo-llvm-cov's default selection; plain cargo test also builds
    // examples, which may not belong to the coverage test run.
    buildArgs.push("--tests");
  }
  const build = spawnSync(
    cargo,
    ["test", "--no-run", "--message-format=json", ...buildArgs],
    {
      encoding: "utf8",
      env,
      maxBuffer: 64 * 1024 * 1024,
      stdio: ["ignore", "pipe", "inherit"],
    },
  );
  if (build.status !== 0)
    throw new Error("could not enumerate the current coverage build artifacts");
  const objects = new Set();
  for (const line of build.stdout.split(/\r?\n/)) {
    if (!line) continue;
    const artifact = JSON.parse(line);
    if (artifact.reason === "compiler-artifact" && artifact.executable) {
      objects.add(path.resolve(artifact.executable));
    }
  }
  if (objects.size === 0)
    throw new Error("the current coverage build has no executable artifacts");
  return [...objects].sort();
}

// cargo-llvm-cov report discovers objects by walking target, including old
// hashes. Give it a temporary target containing ONLY this build's artifacts.
// Hardlinks retain the original binaries/profiles and avoid copying gigabytes.
export function stageReportArtifacts(profileDir, objects) {
  const root = path.resolve(profileDir);
  const reportDir = fs.mkdtempSync(path.join(root, "current-report-"));
  try {
    for (const object of objects) {
      const relative = path.relative(root, object);
      if (
        !relative ||
        relative === ".." ||
        relative.startsWith(`..${path.sep}`) ||
        path.isAbsolute(relative)
      ) {
        throw new Error(
          `current coverage artifact is outside the coverage target: ${object}`,
        );
      }
      const destination = path.join(reportDir, relative);
      fs.mkdirSync(path.dirname(destination), { recursive: true });
      fs.linkSync(object, destination);
    }
    for (const name of fs.readdirSync(root)) {
      if (name.endsWith(".profraw")) {
        fs.linkSync(path.join(root, name), path.join(reportDir, name));
      } else if (name.endsWith(".profdata")) {
        // llvm-profdata may overwrite its output; never write through a link
        // to the predecessor's merged profile.
        fs.copyFileSync(path.join(root, name), path.join(reportDir, name));
      }
    }
    return reportDir;
  } catch (error) {
    fs.rmSync(reportDir, { recursive: true, force: true });
    throw error;
  }
}

function describe(profiles) {
  return profiles
    .map((entry) => `  ${entry.kind}: ${entry.name} (${entry.detail})`)
    .join("\n");
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

async function sha256(file) {
  const hash = createHash("sha256");
  for await (const chunk of fs.createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

function commandOutput(command, args) {
  const result = spawnSync(command, args, { encoding: "utf8" });
  if (result.status !== 0) throw new Error(`${command} ${args.join(" ")} failed`);
  return result.stdout.trim();
}

function sourceIdentity() {
  const source = commandOutput("git", ["rev-parse", "HEAD"]);
  if (!/^[0-9a-f]{40}$/.test(source)) throw new Error("invalid source commit");
  return { source_sha: source, toolchain: commandOutput("rustc", ["-vV"]) };
}

function containedPath(root, relative) {
  if (
    typeof relative !== "string" || !relative ||
    path.isAbsolute(relative) || relative.includes("\\") ||
    relative.split("/").some((part) => !part || part === "." || part === "..")
  ) throw new Error(`invalid artifact path: ${relative}`);
  const resolved = path.resolve(root, relative);
  if (!resolved.startsWith(`${path.resolve(root)}${path.sep}`)) {
    throw new Error(`artifact is outside its root: ${relative}`);
  }
  return resolved;
}

async function checkedProfiles(profileDir) {
  const { profiles } = await inspectProfiles(profileDir, resolveProfdata());
  const damaged = profiles.filter((entry) => entry.kind !== "healthy");
  const recoverable = damaged.length > 0 &&
    damaged.every((entry) => entry.kind === "truncated") &&
    profiles.some((entry) => entry.kind === "healthy");
  if (!profiles.length || (damaged.length && !recoverable)) {
    console.error(`coverage-summary: FAIL [profile-damage] tests passed; no healthy profiles or unrecoverable damage:\n${describe(damaged)}`);
    return undefined;
  }
  return { profiles, damaged, recoverable };
}

function mergeProfiles(files, output, recoverable = false) {
  const result = spawnSync(resolveProfdata(), [
    "merge", "-sparse", ...(recoverable ? ["--failure-mode=all"] : []),
    ...files, "-o", output,
  ], { stdio: "inherit" });
  if (result.status !== 0) throw new Error("could not merge coverage profiles");
}

async function exportShard(cargo, profileDir, runArgs, partition, destination) {
  const match = /^slice:([1-9]\d*)\/([1-9]\d*)$/.exec(partition ?? "");
  if (!match || Number(match[1]) > Number(match[2])) {
    throw new Error("--export-dir requires --nextest --partition slice:N/M");
  }
  const shard = Number(match[1]), count = Number(match[2]);
  // A failed retry must not leave a predecessor's success manifest uploadable.
  fs.rmSync(path.join(destination, "manifest.json"), { force: true });
  fs.mkdirSync(destination, { recursive: true });
  if (cargoLlvmCov(cargo, ["clean", "--workspace", "--profraw-only"]) !== 0) {
    throw new Error("could not clear old profiles");
  }
  const env = coverageEnvironment(cargo, runArgs, profileDir);
  const objects = currentObjects(cargo, runArgs, profileDir, env);
  const inventory = spawnSync(cargo, [
    "nextest", "list", ...runArgs, "--message-format", "json",
  ], { encoding: "utf8", env, maxBuffer: 64 * 1024 * 1024 });
  if (inventory.status !== 0) throw new Error("could not inventory coverage tests");
  writeJson(path.join(destination, "workspace-test-evidence/inventory.json"), JSON.parse(inventory.stdout));
  // Nextest's default store is workspace-relative even when llvm-cov redirects
  // Cargo's build artifacts into a separate instrumented target directory.
  const workspace = JSON.parse(commandOutput(cargo, ["metadata", "--format-version", "1", "--no-deps"]));
  const sourceJunit = path.join(workspace.workspace_root, "target/nextest", process.env.NEXTEST_PROFILE || "default", "junit.xml");
  fs.rmSync(sourceJunit, { force: true });
  const testExit = cargoLlvmCov(cargo, [
    "nextest", "--no-report", "--retries", "0", "--partition", partition, ...runArgs,
  ]);
  if (testExit !== 0) {
    console.error(`coverage-summary: FAIL [test-failure] shard exited ${testExit}; nothing was exported`);
    return testExit;
  }
  const junit = path.join(destination, "nextest/ci/junit.xml");
  fs.mkdirSync(path.dirname(junit), { recursive: true });
  fs.copyFileSync(sourceJunit, junit);
  const health = await checkedProfiles(profileDir);
  if (!health) return PROFILE_DAMAGE_EXIT;
  const profile = path.join(destination, "profiles.profdata");
  mergeProfiles(health.profiles.map((entry) => path.join(profileDir, entry.name)), profile, health.recoverable);
  writeJson(path.join(destination, "profiles.json"), {
    classification: health.recoverable ? "profile-truncated" : "healthy",
    recovered: health.recoverable, profiles: health.profiles.length, damaged: health.damaged,
  });
  const identities = [];
  for (const object of objects) {
    const relative = path.relative(path.resolve(profileDir), object).split(path.sep).join("/");
    containedPath(profileDir, relative);
    identities.push({ path: relative, sha256: await sha256(object) });
    if (shard === 1) {
      const target = containedPath(path.join(destination, "objects"), relative);
      fs.mkdirSync(path.dirname(target), { recursive: true });
      fs.copyFileSync(object, target);
    }
  }
  writeJson(path.join(destination, "manifest.json"), {
    schema_version: 1, success: true, shard, count, ...sourceIdentity(),
    run_args: runArgs, nextest_profile: process.env.NEXTEST_PROFILE || "default",
    profile_directory: path.resolve(profileDir),
    build_environment: Object.fromEntries(Object.entries(env).filter(([key]) =>
      /^(RUSTFLAGS|RUSTDOCFLAGS|CARGO_ENCODED_RUSTFLAGS|CARGO_BUILD_TARGET|CARGO_INCREMENTAL|CARGO_PROFILE_.+)$/.test(key)).sort()),
    objects: identities, profile_sha256: await sha256(profile),
  });
  console.log(`coverage-summary: ${health.recoverable ? "RECOVERED [profile-truncated]" : "PASS"} shard ${shard}/${count} exported`);
  return 0;
}

async function reportShards(cargo, root, count, output, lcovOutput) {
  if (!Number.isSafeInteger(count) || count < 1) throw new Error("invalid --shards count");
  const expected = Array.from({ length: count }, (_, index) => `workspace-tests-${index + 1}`);
  const found = fs.readdirSync(root).filter((name) => name.startsWith("workspace-tests-")).sort();
  if (JSON.stringify(found) !== JSON.stringify([...expected].sort())) {
    throw new Error("missing or unexpected coverage shard artifacts");
  }
  const identity = sourceIdentity();
  let baseline;
  const profiles = [];
  const recoveries = [];
  for (let index = 0; index < count; index += 1) {
    const dir = path.join(root, expected[index]);
    const manifest = JSON.parse(fs.readFileSync(path.join(dir, "manifest.json"), "utf8"));
    if (manifest.schema_version !== 1 || manifest.success !== true ||
      manifest.shard !== index + 1 || manifest.count !== count ||
      manifest.source_sha !== identity.source_sha || manifest.toolchain !== identity.toolchain ||
      !Array.isArray(manifest.objects) || !manifest.objects.length ||
      !Array.isArray(manifest.run_args) || !manifest.run_args.every((arg) => typeof arg === "string") ||
      typeof manifest.nextest_profile !== "string" || typeof manifest.profile_directory !== "string" ||
      !manifest.build_environment || typeof manifest.build_environment !== "object") {
      throw new Error(`invalid or mismatched coverage manifest: ${expected[index]}`);
    }
    const common = JSON.stringify({ source_sha: manifest.source_sha, toolchain: manifest.toolchain,
      run_args: manifest.run_args, nextest_profile: manifest.nextest_profile,
      profile_directory: manifest.profile_directory,
      build_environment: manifest.build_environment, objects: manifest.objects });
    if (baseline && baseline !== common) throw new Error("coverage shard build identities differ");
    baseline = common;
    for (const file of ["workspace-test-evidence/inventory.json", "nextest/ci/junit.xml"]) {
      if (!fs.statSync(path.join(dir, file)).isFile()) throw new Error(`missing coverage evidence: ${file}`);
    }
    const health = JSON.parse(fs.readFileSync(path.join(dir, "profiles.json"), "utf8"));
    if (health.recovered === true) recoveries.push({ ...health, shard: index + 1 });
    const profile = path.join(dir, "profiles.profdata");
    if (await sha256(profile) !== manifest.profile_sha256) throw new Error("coverage profile hash differs");
    profiles.push(profile);
  }
  const manifest = JSON.parse(baseline);
  const objectRoot = path.join(root, expected[0], "objects");
  const workspace = JSON.parse(commandOutput(cargo, ["metadata", "--format-version", "1", "--no-deps"]));
  // Keep the single current object inventory separate from caches and raw files.
  const target = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-coverage-report-"));
  let reportDir;
  try {
    const objects = [];
    for (const object of manifest.objects) {
      const source = containedPath(objectRoot, object.path);
      if (await sha256(source) !== object.sha256) throw new Error("coverage object hash differs");
      const destination = containedPath(target, object.path);
      fs.mkdirSync(path.dirname(destination), { recursive: true });
      fs.copyFileSync(source, destination);
      // Actions artifacts restore files as 0644; cargo-llvm-cov discovers Unix
      // objects by their execute bit. Restore it only after checking identity.
      fs.chmodSync(destination, 0o755);
      objects.push(destination);
    }
    mergeProfiles(profiles, path.join(target, `${path.basename(workspace.workspace_root)}.profdata`));
    reportDir = stageReportArtifacts(target, objects);
    const env = { ...process.env, CARGO_LLVM_COV_TARGET_DIR: reportDir, CARGO_LLVM_COV_BUILD_DIR: reportDir };
    for (const [file, format] of [[output, "--json"], [lcovOutput, "--lcov"]]) {
      if (!file) continue;
      fs.mkdirSync(path.dirname(file), { recursive: true });
      const result = cargoLlvmCov(cargo, ["report", ...reportScope(manifest.run_args), format,
        ...(format === "--json" ? ["--summary-only"] : []),
        "--ignore-filename-regex", `^${manifest.profile_directory.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}[/\\\\]`,
        "--output-path", file], env);
      if (result !== 0) throw new Error(`coverage ${format} report failed`);
    }
    writeJson(`${output}.objects.json`, { objects: manifest.objects });
    if (recoveries.length) {
      writeJson(`${output}.profiles.json`, {
        classification: "profile-truncated", recovered: true, shards: recoveries,
      });
    }
  } finally {
    if (reportDir) fs.rmSync(reportDir, { recursive: true, force: true });
    fs.rmSync(target, { recursive: true, force: true });
  }
  console.log(`coverage-summary: ${recoveries.length ? "RECOVERED [profile-truncated]" : "PASS"} ${count} coverage shards aggregated into ${output}`);
  return 0;
}

async function main() {
  const argv = process.argv.slice(2);
  const split = argv.indexOf("--");
  const own = split === -1 ? argv : argv.slice(0, split);
  const runArgs = split === -1 ? [] : argv.slice(split + 1);
  const outputIndex = own.indexOf("--output-path");
  const output = outputIndex === -1 ? undefined : own[outputIndex + 1];
  const value = (flag) => own.includes(flag) ? own[own.indexOf(flag) + 1] : undefined;
  const exportDir = value("--export-dir");
  const reportRoot = value("--report-from");
  const lcovOutput = value("--lcov-output-path");
  if (exportDir || reportRoot) {
    for (const file of [output, output && `${output}.profiles.json`, output && `${output}.objects.json`, lcovOutput]) {
      if (file) fs.rmSync(file, { force: true });
    }
    try {
      if (exportDir && reportRoot) throw new Error("export and report modes are mutually exclusive");
      const cargo = process.env.CARGO || "cargo";
      if (exportDir) {
        if (!own.includes("--nextest")) throw new Error("export requires --nextest");
        return await exportShard(cargo, resolveProfileDir(cargo), runArgs, value("--partition"), exportDir);
      }
      if (!output || runArgs.length) throw new Error("report mode requires --output-path and no test arguments");
      return await reportShards(cargo, reportRoot, Number(value("--shards")), output, lcovOutput);
    } catch (error) {
      if (output) fs.rmSync(output, { force: true });
      if (lcovOutput) fs.rmSync(lcovOutput, { force: true });
      console.error(`coverage-summary: FAIL [report-failure] ${error.message}`);
      return REPORT_FAILURE_EXIT;
    }
  }
  if (!output) {
    console.error(
      "Usage: node scripts/coverage-summary.mjs --output-path <summary.json> -- <cargo llvm-cov args>",
    );
    return 2;
  }
  const healthFile = `${output}.profiles.json`;
  const objectsFile = `${output}.objects.json`;
  // A summary left from an earlier run must never reach the threshold check.
  fs.rmSync(output, { force: true });
  fs.rmSync(healthFile, { force: true });
  fs.rmSync(objectsFile, { force: true });

  const cargo = process.env.CARGO || "cargo";
  const profileDir = resolveProfileDir(cargo);
  const cleanExit = cargoLlvmCov(cargo, [
    "clean",
    "--workspace",
    "--profraw-only",
  ]);
  if (cleanExit !== 0) {
    console.error(
      `coverage-summary: FAIL [report-failure] could not clear old profiles`,
    );
    return REPORT_FAILURE_EXIT;
  }

  const testExit = cargoLlvmCov(cargo, [
    ...(own.includes("--nextest") ? ["nextest"] : []),
    "--no-report",
    ...(own.includes("--nextest") ? ["--retries", "0"] : []),
    ...runArgs,
  ]);
  if (testExit !== 0) {
    console.error(
      `coverage-summary: FAIL [test-failure] cargo llvm-cov exited ${testExit} while running tests; nothing was aggregated`,
    );
    return testExit;
  }

  let objects;
  try {
    objects = currentObjects(cargo, runArgs, profileDir);
  } catch (error) {
    console.error(`coverage-summary: FAIL [report-failure] ${error.message}`);
    return REPORT_FAILURE_EXIT;
  }
  const health = await checkedProfiles(profileDir);
  if (!health) return PROFILE_DAMAGE_EXIT;
  const { profiles, damaged, recoverable } = health;

  const reportArgs = [
    "report",
    ...reportScope(runArgs),
    ...(recoverable ? ["--failure-mode", "all"] : []),
    "--json",
    "--summary-only",
    "--ignore-filename-regex",
    `^${path.resolve(profileDir).replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}[/\\\\]`,
    "--output-path",
    output,
  ];
  let reportExit;
  let reportDir;
  try {
    reportDir = stageReportArtifacts(profileDir, objects);
    reportExit = cargoLlvmCov(cargo, reportArgs, {
      ...process.env,
      CARGO_LLVM_COV_TARGET_DIR: reportDir,
      CARGO_LLVM_COV_BUILD_DIR: reportDir,
    });
  } catch (error) {
    console.error(`coverage-summary: FAIL [report-failure] ${error.message}`);
    fs.rmSync(output, { force: true });
    return REPORT_FAILURE_EXIT;
  } finally {
    if (reportDir) fs.rmSync(reportDir, { recursive: true, force: true });
  }
  if (recoverable) {
    fs.writeFileSync(
      healthFile,
      `${JSON.stringify(
        {
          classification: "profile-truncated",
          recovered: reportExit === 0,
          aggregation: `cargo llvm-cov ${reportArgs.join(" ")}`,
          profiles: profiles.length,
          damaged,
        },
        null,
        2,
      )}\n`,
    );
  }
  if (reportExit !== 0) {
    fs.rmSync(output, { force: true });
    console.error(
      `coverage-summary: FAIL [${recoverable ? "profile-damage" : "report-failure"}] tests passed; aggregation exited ${reportExit}`,
    );
    return recoverable ? PROFILE_DAMAGE_EXIT : REPORT_FAILURE_EXIT;
  }
  if (recoverable) {
    console.log(
      `coverage-summary: RECOVERED [profile-truncated] tests passed; ${damaged.length} truncated raw profile(s) were left out by one --failure-mode all aggregation (coverage can only be under-counted; recorded in ${healthFile}):\n${describe(damaged)}`,
    );
  } else {
    console.log(
      `coverage-summary: PASS ${profiles.length} raw profile(s) aggregated into ${output}`,
    );
  }
  fs.writeFileSync(objectsFile, `${JSON.stringify({ objects }, null, 2)}\n`);
  return 0;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  process.exitCode = await main();
}
