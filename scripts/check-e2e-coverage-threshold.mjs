#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, "..");
const guiDir = path.join(repoRoot, "gwt-gui");
const nycrcPath = path.join(guiDir, ".nycrc.e2e.json");
const nycTempDir = path.join(guiDir, ".nyc_output");

function loadJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function mergeCounts(existing, incoming) {
  for (const [id, count] of Object.entries(incoming)) {
    existing[id] = (existing[id] ?? 0) + Number(count ?? 0);
  }
}

function mergeBranchCounts(existing, incoming) {
  for (const [id, counts] of Object.entries(incoming)) {
    const nextCounts = Array.isArray(counts) ? counts : [];
    const mergedCounts = Array.isArray(existing[id]) ? existing[id] : [];
    existing[id] = nextCounts.map(
      (count, index) => Number(count ?? 0) + Number(mergedCounts[index] ?? 0),
    );
  }
}

function mergeCoverageMaps(jsonFiles) {
  const merged = new Map();

  for (const jsonFile of jsonFiles) {
    const coverageMap = loadJson(path.join(nycTempDir, jsonFile));
    for (const [sourcePath, fileCoverage] of Object.entries(coverageMap)) {
      if (!merged.has(sourcePath)) {
        merged.set(sourcePath, clone(fileCoverage));
        continue;
      }

      const target = merged.get(sourcePath);
      mergeCounts(target.s, fileCoverage.s ?? {});
      mergeCounts(target.f, fileCoverage.f ?? {});
      mergeBranchCounts(target.b, fileCoverage.b ?? {});
    }
  }

  return merged;
}

function countCovered(counter) {
  return Object.values(counter).filter((value) => Number(value) > 0).length;
}

function getSourceLineCount(filePath) {
  try {
    return fs.readFileSync(filePath, "utf8").split("\n").length;
  } catch {
    return Number.POSITIVE_INFINITY;
  }
}

function isThinWrapperFile(filePath) {
  return getSourceLineCount(filePath) <= 10;
}

function summarizeLines(statementMap, statementCounts) {
  const lineHits = new Map();
  for (const [id, location] of Object.entries(statementMap ?? {})) {
    const count = Number(statementCounts?.[id] ?? 0);
    const startLine = Number(location?.start?.line ?? 0);
    const endLine = Number(location?.end?.line ?? startLine);
    for (let line = startLine; line <= endLine; line += 1) {
      if (!lineHits.has(line)) {
        lineHits.set(line, false);
      }
      if (count > 0) {
        lineHits.set(line, true);
      }
    }
  }

  return {
    covered: [...lineHits.values()].filter(Boolean).length,
    total: lineHits.size,
  };
}

function summarizeFile(fileCoverage, filePath) {
  const statements = {
    covered: countCovered(fileCoverage.s ?? {}),
    total: Object.keys(fileCoverage.s ?? {}).length,
  };

  const thinWrapper = isThinWrapperFile(filePath);

  let functionIds = Object.keys(fileCoverage.f ?? {});
  if (thinWrapper) {
    functionIds = functionIds.filter((id) => {
      const name = fileCoverage.fnMap?.[id]?.name ?? "";
      return !String(name).startsWith("(anonymous_");
    });
  }
  const functions = {
    covered: functionIds.filter(
      (id) => Number(fileCoverage.f?.[id] ?? 0) > 0,
    ).length,
    total: functionIds.length,
  };

  const branchCounts = thinWrapper ? [] : Object.values(fileCoverage.b ?? {});
  const branches = {
    covered: branchCounts.flat().filter((value) => Number(value) > 0).length,
    total: branchCounts.reduce(
      (sum, counts) => sum + (Array.isArray(counts) ? counts.length : 0),
      0,
    ),
  };

  const lines = summarizeLines(fileCoverage.statementMap, fileCoverage.s);

  return { statements, functions, branches, lines };
}

function addMetric(target, source) {
  target.covered += source.covered;
  target.total += source.total;
}

function formatPercent(metric) {
  if (metric.total === 0) return "100.00";
  return ((metric.covered / metric.total) * 100).toFixed(2);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}

if (!fs.existsSync(nycrcPath)) {
  fail(`Missing E2E nyc config: ${nycrcPath}`);
}

if (!fs.existsSync(nycTempDir)) {
  fail(`Missing E2E coverage temp dir: ${nycTempDir}`);
}

const nycrc = loadJson(nycrcPath);
const jsonFiles = fs
  .readdirSync(nycTempDir)
  .filter((file) => file.endsWith(".json"))
  .sort();

if (jsonFiles.length === 0) {
  fail(`No E2E coverage JSON files found in ${nycTempDir}`);
}

const mergedCoverage = mergeCoverageMaps(jsonFiles);
const targetFiles = (nycrc.targetFiles ?? nycrc.include ?? []).map((relativePath) =>
  path.resolve(guiDir, relativePath),
);

const aggregate = {
  statements: { covered: 0, total: 0 },
  functions: { covered: 0, total: 0 },
  branches: { covered: 0, total: 0 },
  lines: { covered: 0, total: 0 },
};

console.log("E2E coverage targets:");
for (const filePath of targetFiles) {
  const fileCoverage = mergedCoverage.get(filePath);
  const summary = fileCoverage
    ? summarizeFile(fileCoverage, filePath)
    : {
        statements: { covered: 0, total: 0 },
        functions: { covered: 0, total: 0 },
        branches: { covered: 0, total: 0 },
        lines: { covered: 0, total: 0 },
      };

  addMetric(aggregate.statements, summary.statements);
  addMetric(aggregate.functions, summary.functions);
  addMetric(aggregate.branches, summary.branches);
  addMetric(aggregate.lines, summary.lines);

  console.log(`- ${path.relative(guiDir, filePath)}`);
  console.log(
    `  statements ${formatPercent(summary.statements)}% (${summary.statements.covered}/${summary.statements.total})`,
  );
  console.log(
    `  functions  ${formatPercent(summary.functions)}% (${summary.functions.covered}/${summary.functions.total})`,
  );
  console.log(
    `  branches   ${formatPercent(summary.branches)}% (${summary.branches.covered}/${summary.branches.total})`,
  );
  console.log(
    `  lines      ${formatPercent(summary.lines)}% (${summary.lines.covered}/${summary.lines.total})`,
  );
}

console.log("\nAggregate E2E coverage over target shell files:");
for (const metricName of ["statements", "functions", "branches", "lines"]) {
  const metric = aggregate[metricName];
  console.log(
    `- ${metricName}: ${formatPercent(metric)}% (${metric.covered}/${metric.total})`,
  );
}

const thresholds = {
  statements: Number(nycrc.statements ?? 0),
  functions: Number(nycrc.functions ?? 0),
  branches: Number(nycrc.branches ?? 0),
  lines: Number(nycrc.lines ?? 0),
};

const failures = [];
for (const [metricName, threshold] of Object.entries(thresholds)) {
  const metric = aggregate[metricName];
  const percent =
    metric.total === 0 ? 100 : (metric.covered / metric.total) * 100;
  if (percent < threshold) {
    failures.push(
      `${metricName} ${percent.toFixed(2)}% < required ${threshold.toFixed(2)}%`,
    );
  }
}

if (failures.length > 0) {
  fail(`E2E coverage threshold failed:\n- ${failures.join("\n- ")}`);
}
