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
    const child = spawn(profdata, ["show", file], { stdio: ["ignore", "ignore", "pipe"] });
    let stderr = "";
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", (error) => resolve({ ok: false, detail: error.message }));
    child.on("close", (code) => resolve({ ok: code === 0, detail: stderr.trim() }));
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

  const limit = Math.max(1, Math.min(8, Math.floor(os.availableParallelism() / 2)));
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

  for (const entry of entries.filter((candidate) => candidate.kind === "unreadable")) {
    const fullSize = Math.max(
      0,
      ...entries
        .filter((other) => other.kind === "healthy" && other.module === entry.module)
        .map((other) => other.size),
    );
    entry.kind = fullSize > entry.size ? "truncated" : "corrupt";
    if (entry.kind === "truncated") {
      entry.detail = `${entry.size} of ${fullSize} bytes; ${entry.detail}`;
    }
  }

  return {
    profiles: entries.map(({ name, size, kind, detail }) => ({ name, size, kind, detail })),
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
  const sysroot = spawnSync("rustc", ["--print", "sysroot"], { encoding: "utf8" }).stdout?.trim();
  const exe = process.platform === "win32" ? "llvm-profdata.exe" : "llvm-profdata";
  return path.join(sysroot ?? "", "lib", "rustlib", hostTriple() ?? "", "bin", exe);
}

function resolveProfileDir(cargo) {
  if (process.env.CARGO_LLVM_COV_TARGET_DIR) {
    return process.env.CARGO_LLVM_COV_TARGET_DIR;
  }
  const metadata = spawnSync(cargo, ["metadata", "--format-version", "1", "--no-deps"], {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  if (metadata.status !== 0) {
    throw new Error(`cargo metadata failed: ${metadata.stderr}`);
  }
  return path.join(JSON.parse(metadata.stdout).target_directory, "llvm-cov-target");
}

// `cargo llvm-cov report` takes the run's package and feature scope but not
// workspace-wide selection, which it already implies.
function reportScope(args) {
  const scope = [];
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (arg === "--") {
      break;
    }
    if (arg === "--workspace" || arg === "--all" || arg.startsWith("--exclude=")) {
      continue;
    }
    if (arg === "--exclude") {
      i += 1;
      continue;
    }
    scope.push(arg);
  }
  return scope;
}

function cargoLlvmCov(cargo, args) {
  const result = spawnSync(cargo, ["llvm-cov", ...args], { stdio: "inherit" });
  if (result.error) {
    throw result.error;
  }
  return result.status ?? 1;
}

function describe(profiles) {
  return profiles.map((entry) => `  ${entry.kind}: ${entry.name} (${entry.detail})`).join("\n");
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
  // A summary left from an earlier run must never reach the threshold check.
  fs.rmSync(output, { force: true });
  fs.rmSync(healthFile, { force: true });

  const cargo = process.env.CARGO || "cargo";
  const profileDir = resolveProfileDir(cargo);
  const cleanExit = cargoLlvmCov(cargo, ["clean", "--workspace", "--profraw-only"]);
  if (cleanExit !== 0) {
    console.error(`coverage-summary: FAIL [report-failure] could not clear old profiles`);
    return REPORT_FAILURE_EXIT;
  }

  const testExit = cargoLlvmCov(cargo, ["--no-report", ...runArgs]);
  if (testExit !== 0) {
    console.error(
      `coverage-summary: FAIL [test-failure] cargo llvm-cov exited ${testExit} while running tests; nothing was aggregated`,
    );
    return testExit;
  }

  const { profiles } = await inspectProfiles(profileDir, resolveProfdata());
  const damaged = profiles.filter((entry) => entry.kind !== "healthy");
  const truncatedOnly =
    damaged.length > 0 && damaged.every((entry) => entry.kind === "truncated");
  const recoverable = truncatedOnly && profiles.some((entry) => entry.kind === "healthy");
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
    "--output-path",
    output,
  ];
  const reportExit = cargoLlvmCov(cargo, reportArgs);
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
    console.log(`coverage-summary: PASS ${profiles.length} raw profile(s) aggregated into ${output}`);
  }
  return 0;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = await main();
}
