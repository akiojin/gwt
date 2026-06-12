#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const [summaryPath, thresholdArg, ...scopeArgs] = process.argv.slice(2);

if (!summaryPath || !thresholdArg) {
  console.error(
    "Usage: node scripts/check-coverage-threshold.mjs <summary-json> <threshold> " +
      "[--scope <regex>] [--scope-exclude <regex>]",
  );
  process.exit(2);
}

// Optional scope filters so one workspace-wide summary can enforce
// different thresholds per crate group (paths are matched with `/`
// separators regardless of platform).
let scopeRe = null;
let scopeExcludeRe = null;
for (let i = 0; i < scopeArgs.length; i += 2) {
  const flag = scopeArgs[i];
  const value = scopeArgs[i + 1];
  if (!value) {
    console.error(`Missing value for ${flag}`);
    process.exit(2);
  }
  if (flag === "--scope") {
    scopeRe = new RegExp(value);
  } else if (flag === "--scope-exclude") {
    scopeExcludeRe = new RegExp(value);
  } else {
    console.error(`Unknown option: ${flag}`);
    process.exit(2);
  }
}

const threshold = Number(thresholdArg);
if (!Number.isFinite(threshold)) {
  console.error(`Invalid threshold: ${thresholdArg}`);
  process.exit(2);
}

const report = JSON.parse(fs.readFileSync(summaryPath, "utf8"));
const files = report.data?.flatMap((entry) => entry.files ?? []) ?? [];
const ignoredFilePatterns = [
  // Runtime entrypoints, process orchestration, and split CLI integration
  // adapters are covered by focused contract tests; line coverage is not a
  // useful release gate for these outer shells.
  /(^|[\\/])gwt[\\/]src[\\/]main\.rs$/i,
  /(^|[\\/])gwt[\\/]src[\\/]bin[\\/]gwtd\.rs$/i,
  /(^|[\\/])gwt[\\/]src[\\/]app_runtime\.rs$/i,
  /(^|[\\/])gwt[\\/]src[\\/]app_runtime[\\/].+\.rs$/i,
  /(^|[\\/])gwt[\\/]src[\\/]docker_launch\.rs$/i,
  /(^|[\\/])gwt[\\/]src[\\/]index_worker\.rs$/i,
  /(^|[\\/])gwt[\\/]src[\\/]issue_cache\.rs$/i,
  /(^|[\\/])gwt[\\/]src[\\/]launch_runtime\.rs$/i,
  /(^|[\\/])gwt[\\/]src[\\/]native_app\.rs$/i,
  /(^|[\\/])gwt[\\/]src[\\/]update_front_door\.rs$/i,
  /(^|[\\/])gwt[\\/]src[\\/]cli[\\/]daemon[\\/]mod\.rs$/i,
  /(^|[\\/])gwt[\\/]src[\\/]cli[\\/]index\.rs$/i,
  /(^|[\\/])gwt[\\/]src[\\/]cli[\\/]index[\\/].+\.rs$/i,
];

let coveredLines = 0;
let totalLines = 0;
const excludedFiles = [];

for (const file of files) {
  const filename = file.filename ?? "";
  const normalized = filename.replace(/\\/g, "/");
  if (scopeRe && !scopeRe.test(normalized)) {
    continue;
  }
  if (scopeExcludeRe && scopeExcludeRe.test(normalized)) {
    continue;
  }
  if (ignoredFilePatterns.some((pattern) => pattern.test(filename))) {
    excludedFiles.push(path.relative(process.cwd(), filename));
    continue;
  }

  const lines = file.summary?.lines;
  if (!lines) {
    continue;
  }

  coveredLines += lines.covered;
  totalLines += lines.count;
}

if (totalLines === 0) {
  console.error("Coverage report did not contain any line summary data.");
  process.exit(1);
}

const percent = (coveredLines / totalLines) * 100;
const scopeLabel = scopeRe
  ? ` [scope: ${scopeRe.source}]`
  : scopeExcludeRe
    ? ` [scope-exclude: ${scopeExcludeRe.source}]`
    : "";
console.log(
  `Filtered line coverage${scopeLabel}: ${percent.toFixed(2)}% (${coveredLines}/${totalLines})`,
);
if (excludedFiles.length > 0) {
  console.log(`Excluded from threshold: ${excludedFiles.join(", ")}`);
}

if (percent + Number.EPSILON < threshold) {
  console.error(
    `Coverage threshold not met: required ${threshold.toFixed(2)}%, got ${percent.toFixed(2)}%`,
  );
  process.exit(1);
}

console.log(`Coverage threshold met: ${threshold.toFixed(2)}%`);
