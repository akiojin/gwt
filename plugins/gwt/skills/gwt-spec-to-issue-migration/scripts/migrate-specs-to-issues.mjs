#!/usr/bin/env node
// migrate-specs-to-issues.mjs
// Migrate local specs/SPEC-*/ directories to GitHub Issues with gwt-spec label.
//
// Usage:
//   node migrate-specs-to-issues.mjs [--dry-run] [--specs-dir DIR] [--label LABEL]...
//
// Options:
//   --dry-run       Show what would be done without creating issues
//   --specs-dir     Path to specs/ directory (default: auto-detect from target repository)
//   --label LABEL   Additional label to apply (can be repeated; gwt-spec is always applied)

import { execSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync, readdirSync, statSync, rmSync, rmdirSync } from "node:fs";
import path from "node:path";

let DRY_RUN = false;
let SPECS_DIR = "";
const REPORT_FILE = "migration-report.json";
const RATE_LIMIT_BATCH = 10;
const RATE_LIMIT_SLEEP = 3;
const EXTRA_LABELS = [];
let REPO_ROOT = "";
const SPEC_DIRS = [];
let CLEANUP_TARGETS = [];

const LEGACY_CLEANUP_ALLOWLIST_RELATIVE = [
  ".specify",
  ".github/spec-kit",
  ".github/spec-kit.yml",
  ".github/spec-kit.yaml",
  ".github/prompts/specify.md",
  ".github/prompts/specify-system.md",
  "scripts/spec-kit.sh",
  "scripts/specify.sh",
  "templates/spec-kit",
  "templates/spec-kit.md",
  "templates/specify",
  "templates/specify.md",
];

// Parse arguments
const args = process.argv.slice(2);
for (let i = 0; i < args.length; i++) {
  switch (args[i]) {
    case "--dry-run":
      DRY_RUN = true;
      break;
    case "--specs-dir":
      SPECS_DIR = args[++i];
      break;
    case "--label":
      EXTRA_LABELS.push(args[++i]);
      break;
    default:
      console.error(`Unknown option: ${args[i]}`);
      process.exit(1);
  }
}

function resolveRepoRoot() {
  if (process.env.GWT_PROJECT_ROOT) return process.env.GWT_PROJECT_ROOT;
  try {
    return execSync("git rev-parse --show-toplevel", {
      encoding: "utf8",
      stdio: ["pipe", "pipe", "ignore"],
    }).trim();
  } catch {
    return "";
  }
}

function appendCleanupTarget(target) {
  if (!existsSync(target)) return;
  if (!CLEANUP_TARGETS.includes(target)) CLEANUP_TARGETS.push(target);
}

function collectCleanupTargets() {
  CLEANUP_TARGETS = [];
  for (const dir of SPEC_DIRS) appendCleanupTarget(dir);
  appendCleanupTarget(path.join(SPECS_DIR, "specs.md"));
  appendCleanupTarget(path.join(SPECS_DIR, "archive"));
  for (const rel of LEGACY_CLEANUP_ALLOWLIST_RELATIVE) {
    appendCleanupTarget(path.join(REPO_ROOT, rel));
  }
}

function previewCleanupTargets() {
  collectCleanupTargets();
  console.log("");
  console.log("Legacy cleanup plan:");
  if (CLEANUP_TARGETS.length === 0) {
    console.log("  (no legacy paths detected beyond migration report handling)");
  } else {
    for (const p of CLEANUP_TARGETS) console.log(`  - ${p}`);
  }
  console.log(`  - ${REPORT_FILE} (delete after successful cleanup)`);
}

function cleanupLegacySources() {
  collectCleanupTargets();
  console.log("");
  console.log("Cleaning up legacy local spec artifacts...");
  for (const p of CLEANUP_TARGETS) {
    if (!existsSync(p)) continue;
    console.log(`  Removing ${p}`);
    rmSync(p, { recursive: true, force: true });
  }

  if (existsSync(SPECS_DIR)) {
    try {
      const entries = readdirSync(SPECS_DIR);
      if (entries.length === 0) {
        console.log(`  Removing empty ${SPECS_DIR}`);
        rmdirSync(SPECS_DIR);
      }
    } catch {
      // ignore
    }
  }

  if (existsSync(REPORT_FILE)) {
    console.log(`  Removing ${REPORT_FILE}`);
    rmSync(REPORT_FILE, { force: true });
  }
}

function writeEmptyReportAndExit() {
  writeFileSync(REPORT_FILE, "[]\n");
  console.log("");
  console.log("Migration complete: 0 succeeded, 0 failed out of 0 total");
  console.log(`Report: ${REPORT_FILE}`);
  process.exit(0);
}

function sleep(seconds) {
  execSync(`node -e "setTimeout(()=>{},${seconds * 1000})"`, { stdio: "ignore" });
}

// Resolve specs directory
if (!SPECS_DIR) {
  REPO_ROOT = resolveRepoRoot();
  if (!REPO_ROOT) {
    console.error(
      "Error: target repository root not found. Set GWT_PROJECT_ROOT or run inside a git repository."
    );
    process.exit(1);
  }

  const candidate = path.join(REPO_ROOT, "specs");
  if (existsSync(candidate) && statSync(candidate).isDirectory()) {
    SPECS_DIR = candidate;
  }

  if (!SPECS_DIR) {
    console.log("Specs directory not found under target repository. Nothing to migrate.");
    writeEmptyReportAndExit();
  }
} else if (!existsSync(SPECS_DIR) || !statSync(SPECS_DIR).isDirectory()) {
  console.error(`Error: specs directory not found: ${SPECS_DIR}`);
  process.exit(1);
}

if (!REPO_ROOT) {
  REPO_ROOT = path.resolve(SPECS_DIR, "..");
}

console.log(`Specs directory: ${SPECS_DIR}`);
console.log(`Dry run: ${DRY_RUN}`);

// Collect SPEC directories (exclude archive/)
for (const entry of readdirSync(SPECS_DIR)) {
  if (!entry.startsWith("SPEC-")) continue;
  const fullPath = path.join(SPECS_DIR, entry);
  if (statSync(fullPath).isDirectory()) SPEC_DIRS.push(fullPath);
}

console.log(`Found ${SPEC_DIRS.length} spec directories to migrate`);

if (SPEC_DIRS.length === 0) writeEmptyReportAndExit();

// Initialize report
const reportEntries = [];
let count = 0;
let success = 0;
let failed = 0;

function readSection(file) {
  if (existsSync(file)) return readFileSync(file, "utf8");
  return "_TODO_";
}

function extractTitle(specFile) {
  if (!existsSync(specFile)) return "Untitled spec";
  const content = readFileSync(specFile, "utf8");
  const match = content.match(/^#+\s+(.*)/m);
  if (match) return match[1].substring(0, 200);
  return "Untitled spec";
}

function buildIssueBody(dir, specId) {
  const spec = readSection(path.join(dir, "spec.md"));
  const plan = readSection(path.join(dir, "plan.md"));
  const tasks = readSection(path.join(dir, "tasks.md"));
  const tdd = readSection(path.join(dir, "tdd.md"));
  const research = readSection(path.join(dir, "research.md"));
  const dataModel = readSection(path.join(dir, "data-model.md"));
  const quickstart = readSection(path.join(dir, "quickstart.md"));

  let contractsNote;
  const contractsDir = path.join(dir, "contracts");
  if (existsSync(contractsDir) && readdirSync(contractsDir).length > 0) {
    contractsNote = "Migrated from local files. See artifact comments below.";
  } else {
    contractsNote =
      "Artifact files under `contracts/` are managed in issue comments with `contract:<name>` entries.";
  }

  let checklistsNote;
  const checklistsDir = path.join(dir, "checklists");
  if (existsSync(checklistsDir) && readdirSync(checklistsDir).length > 0) {
    checklistsNote = "Migrated from local files. See artifact comments below.";
  } else {
    checklistsNote =
      "Artifact files under `checklists/` are managed in issue comments with `checklist:<name>` entries.";
  }

  return `<!-- GWT_SPEC_ID:${specId} -->

## Spec

${spec}

## Plan

${plan}

## Tasks

${tasks}

## TDD

${tdd}

## Research

${research}

## Data Model

${dataModel}

## Quickstart

${quickstart}

## Contracts

${contractsNote}

## Checklists

${checklistsNote}

## Acceptance Checklist

- [ ] Add acceptance checklist`;
}

function createArtifactComments(dir, issueNumber) {
  let hadError = false;
  for (const subdir of ["contracts", "checklists"]) {
    const artifactDir = path.join(dir, subdir);
    if (!existsSync(artifactDir) || !statSync(artifactDir).isDirectory()) continue;

    const kind = subdir.replace(/s$/, "");
    for (const file of readdirSync(artifactDir)) {
      const filePath = path.join(artifactDir, file);
      if (!statSync(filePath).isFile()) continue;

      const name = file;
      const content = readFileSync(filePath, "utf8");

      if (DRY_RUN) {
        console.log(`  [dry-run] Would create ${kind} artifact comment: ${name}`);
      } else {
        const commentBody = `<!-- GWT_SPEC_ARTIFACT:${kind}:${name} -->\n${kind}:${name}\n\n${content}`;
        try {
          execSync(`gh issue comment ${issueNumber} --body ${JSON.stringify(commentBody)}`, {
            stdio: ["pipe", "ignore", "ignore"],
          });
        } catch {
          console.error(`  Warning: Failed to create ${kind} artifact: ${name}`);
          hadError = true;
        }
      }
    }
  }
  return !hadError;
}

for (const dir of SPEC_DIRS) {
  const specName = path.basename(dir);
  const specFile = path.join(dir, "spec.md");

  let title = extractTitle(specFile);
  if (!title || title === "Untitled spec") title = specName;

  count++;

  if (DRY_RUN) {
    console.log(`[${count}] [dry-run] Would create issue: ${title} (from ${specName})`);
    reportEntries.push({
      oldSpecId: specName,
      issueNumber: 0,
      title,
      status: "dry-run",
    });
    continue;
  }

  console.log(`[${count}] Creating issue: ${title}`);

  const issueBody = buildIssueBody(dir, specName);
  const labelArgs = ["--label", "gwt-spec"];
  for (const label of EXTRA_LABELS) {
    labelArgs.push("--label", label);
  }

  let issueUrl;
  try {
    issueUrl = execSync(
      `gh issue create --title ${JSON.stringify(title)} --body ${JSON.stringify(issueBody)} ${labelArgs.map((a) => JSON.stringify(a)).join(" ")}`,
      { encoding: "utf8", stdio: ["pipe", "pipe", "ignore"] }
    ).trim();
  } catch {
    console.error(`  Failed to create issue for ${specName}`);
    reportEntries.push({ oldSpecId: specName, issueNumber: 0, title, status: "failed" });
    failed++;
    continue;
  }

  let migrationOk = true;
  const issueNumber = issueUrl.split("/").pop();
  console.log(`  Created issue #${issueNumber}`);

  const updatedBody = buildIssueBody(dir, `#${issueNumber}`);
  try {
    execSync(
      `gh issue edit ${issueNumber} --body ${JSON.stringify(updatedBody)}`,
      { stdio: ["pipe", "ignore", "ignore"] }
    );
  } catch {
    console.error(`  Warning: Failed to update issue body for ${specName}`);
    migrationOk = false;
  }

  if (!createArtifactComments(dir, issueNumber)) migrationOk = false;

  if (migrationOk) {
    reportEntries.push({ oldSpecId: specName, issueNumber: Number(issueNumber), title, status: "success" });
    success++;
  } else {
    reportEntries.push({ oldSpecId: specName, issueNumber: Number(issueNumber), title, status: "failed" });
    failed++;
  }

  if (count % RATE_LIMIT_BATCH === 0 && count < SPEC_DIRS.length) {
    console.log(`  Rate limit pause: sleeping ${RATE_LIMIT_SLEEP}s...`);
    sleep(RATE_LIMIT_SLEEP);
  }
}

writeFileSync(REPORT_FILE, JSON.stringify(reportEntries, null, 2) + "\n");

console.log("");
console.log(`Migration complete: ${success} succeeded, ${failed} failed out of ${count} total`);

if (DRY_RUN) {
  console.log(`Report: ${REPORT_FILE}`);
  previewCleanupTargets();
} else if (failed > 0) {
  console.log(`Report: ${REPORT_FILE}`);
  console.log("Cleanup skipped because some migrations failed.");
} else {
  cleanupLegacySources();
  console.log(`Legacy cleanup complete. Report removed: ${REPORT_FILE}`);
}
