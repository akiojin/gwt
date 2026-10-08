#!/usr/bin/env node

// Run the coverage tests and aggregate their summary, keeping a damaged raw
// profile apart from a failing test (Issue #4628).
//
//   node scripts/coverage-summary.mjs --output-path <summary.json> -- <cargo llvm-cov args>
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

// cargo-llvm-cov names profiles `<name>-%p-%Nm.profraw`; the `%m` token is
// the instrumented module's signature, shared by every process of one binary.
function moduleOf(name) {
  const stem = name.replace(/\.profraw$/, "");
  return stem.slice(stem.lastIndexOf("-") + 1);
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

function currentObjects(cargo, runArgs, profileDir) {
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

async function main() {
  const argv = process.argv.slice(2);
  const split = argv.indexOf("--");
  const own = split === -1 ? argv : argv.slice(0, split);
  const runArgs = split === -1 ? [] : argv.slice(split + 1);
  const outputIndex = own.indexOf("--output-path");
  const output = outputIndex === -1 ? undefined : own[outputIndex + 1];
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

  const testExit = cargoLlvmCov(cargo, ["--no-report", ...runArgs]);
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
  const { profiles } = await inspectProfiles(profileDir, resolveProfdata());
  const damaged = profiles.filter((entry) => entry.kind !== "healthy");
  const truncatedOnly =
    damaged.length > 0 && damaged.every((entry) => entry.kind === "truncated");
  const recoverable =
    truncatedOnly && profiles.some((entry) => entry.kind === "healthy");
  if (damaged.length > 0 && !recoverable) {
    console.error(
      `coverage-summary: FAIL [profile-damage] tests passed; ${damaged.length} of ${profiles.length} raw profile(s) cannot be aggregated and are not a recoverable truncation:\n${describe(damaged)}`,
    );
    return PROFILE_DAMAGE_EXIT;
  }

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
