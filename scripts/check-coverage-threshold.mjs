#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const [summaryPath, thresholdArg] = process.argv.slice(2);

if (!summaryPath || !thresholdArg) {
  console.error(
    "Usage: node scripts/check-coverage-threshold.mjs <summary-json> <threshold>",
  );
  process.exit(2);
}

const threshold = Number(thresholdArg);
if (!Number.isFinite(threshold)) {
  console.error(`Invalid threshold: ${thresholdArg}`);
  process.exit(2);
}

const report = JSON.parse(fs.readFileSync(summaryPath, "utf8"));
const files = report.data?.flatMap((entry) => entry.files ?? []) ?? [];
const ignoredFilePatterns = [/(^|[\\/])gwt[\\/]src[\\/]main\.rs$/i];

let coveredLines = 0;
let totalLines = 0;
const excludedFiles = [];

for (const file of files) {
  const filename = file.filename ?? "";
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
console.log(
  `Filtered line coverage: ${percent.toFixed(2)}% (${coveredLines}/${totalLines})`,
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
