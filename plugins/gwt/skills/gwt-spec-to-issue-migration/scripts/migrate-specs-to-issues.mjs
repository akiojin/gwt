#!/usr/bin/env node
// migrate-specs-to-issues.mjs
// Migrate local specs/SPEC-*/ directories and legacy body-canonical gwt-spec issues
// to artifact-first GitHub Issues with gwt-spec label.
//
// Usage:
//   node migrate-specs-to-issues.mjs [--dry-run] [--specs-dir DIR] [--label LABEL]... [--convert-existing-issues]
//
// Options:
//   --dry-run       Show what would be done without creating issues
//   --specs-dir     Path to specs/ directory (default: auto-detect from target repository)
//   --label LABEL   Additional label to apply (can be repeated; gwt-spec is always applied)
//   --convert-existing-issues  Rewrite body-canonical gwt-spec issues to artifact-first format

import { execFileSync, execSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync, readdirSync, statSync, rmSync, rmdirSync } from "node:fs";
import path from "node:path";

let DRY_RUN = false;
let SPECS_DIR = "";
let CONVERT_EXISTING_ISSUES = false;
const REPORT_FILE = "migration-report.json";
const RATE_LIMIT_BATCH = 10;
const RATE_LIMIT_SLEEP = 3;
const COMMENT_POST_SLEEP = 1;
const GH_RETRY_ATTEMPTS = 5;
const GH_RETRY_SLEEP_SECONDS = [5, 15, 30, 60];
const EXEC_MAX_BUFFER = 20 * 1024 * 1024;
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
    case "--convert-existing-issues":
      CONVERT_EXISTING_ISSUES = true;
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
      maxBuffer: EXEC_MAX_BUFFER,
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

function makeTempFilePath(prefix, ext = ".md") {
  return path.join(
    REPO_ROOT || process.cwd(),
    `.tmp-${prefix}-${process.pid}-${Date.now()}-${Math.random().toString(16).slice(2)}${ext}`
  );
}

function withTempBodyFile(prefix, body, fn) {
  const filePath = makeTempFilePath(prefix);
  writeFileSync(filePath, body);
  try {
    return fn(filePath);
  } finally {
    rmSync(filePath, { force: true });
  }
}

function gh(args, options = {}) {
  return execFileSync("gh", args, {
    encoding: "utf8",
    stdio: ["pipe", "pipe", "ignore"],
    maxBuffer: EXEC_MAX_BUFFER,
    ...options,
  });
}

function runWithRetry(label, fn) {
  let lastError;
  for (let attempt = 1; attempt <= GH_RETRY_ATTEMPTS; attempt++) {
    try {
      return fn();
    } catch (error) {
      lastError = error;
      const sleepSeconds =
        GH_RETRY_SLEEP_SECONDS[Math.min(attempt - 1, GH_RETRY_SLEEP_SECONDS.length - 1)];
      if (attempt === GH_RETRY_ATTEMPTS) break;
      console.error(`  Warning: ${label} failed (attempt ${attempt}/${GH_RETRY_ATTEMPTS}); retrying in ${sleepSeconds}s`);
      sleep(sleepSeconds);
    }
  }
  throw lastError;
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

  if (!SPECS_DIR && !CONVERT_EXISTING_ISSUES) {
    console.log("Specs directory not found under target repository. Nothing to migrate.");
    writeEmptyReportAndExit();
  }
} else if (!existsSync(SPECS_DIR) || !statSync(SPECS_DIR).isDirectory()) {
  console.error(`Error: specs directory not found: ${SPECS_DIR}`);
  process.exit(1);
}

if (!REPO_ROOT) {
  REPO_ROOT = SPECS_DIR ? path.resolve(SPECS_DIR, "..") : resolveRepoRoot();
}

console.log(`Specs directory: ${SPECS_DIR || "(not used)"}`);
console.log(`Dry run: ${DRY_RUN}`);
console.log(`Convert existing issues: ${CONVERT_EXISTING_ISSUES}`);

// Collect SPEC directories (exclude archive/)
if (SPECS_DIR) {
  for (const entry of readdirSync(SPECS_DIR)) {
    if (!entry.startsWith("SPEC-")) continue;
    const fullPath = path.join(SPECS_DIR, entry);
    if (statSync(fullPath).isDirectory()) SPEC_DIRS.push(fullPath);
  }
}

console.log(`Found ${SPEC_DIRS.length} spec directories to migrate`);

if (SPEC_DIRS.length === 0 && !CONVERT_EXISTING_ISSUES) writeEmptyReportAndExit();

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

function buildIssueBody(specId) {
  return `<!-- GWT_SPEC_ID:${specId} -->

## Artifact Index

- \`doc:spec.md\`
- \`doc:plan.md\`
- \`doc:tasks.md\`
- \`doc:research.md\`
- \`doc:data-model.md\`
- \`doc:quickstart.md\`
- \`checklist:tdd.md\`
- \`checklist:acceptance.md\`
- \`contract:*\`
- \`checklist:*\`

## Status

- Phase: Specify
- Clarification: Migrated from legacy source
- Analysis: Pending \`gwt-spec-analyze\`

## Links

- Parent: ...
- Related: ...
- PRs: ...
`;
}

function createArtifactComments(dir, issueNumber) {
  let hadError = false;
  const docArtifacts = [
    ["doc", "spec.md", readSection(path.join(dir, "spec.md"))],
    ["doc", "plan.md", readSection(path.join(dir, "plan.md"))],
    ["doc", "tasks.md", readSection(path.join(dir, "tasks.md"))],
    ["checklist", "tdd.md", readSection(path.join(dir, "tdd.md"))],
    ["doc", "research.md", readSection(path.join(dir, "research.md"))],
    ["doc", "data-model.md", readSection(path.join(dir, "data-model.md"))],
    ["doc", "quickstart.md", readSection(path.join(dir, "quickstart.md"))],
  ];

  for (const [kind, name, content] of docArtifacts) {
    if (!content || content.trim() === "_TODO_") continue;
    if (DRY_RUN) {
      console.log(`  [dry-run] Would create ${kind} artifact comment: ${name}`);
    } else {
      const commentBody = `<!-- GWT_SPEC_ARTIFACT:${kind}:${name} -->\n${kind}:${name}\n\n${content.trim()}\n`;
      try {
        runWithRetry(`create ${kind}:${name} on issue #${issueNumber}`, () =>
          withTempBodyFile(`issue-comment-${issueNumber}-${kind}-${name}`, commentBody, (bodyFile) => {
            gh(["issue", "comment", String(issueNumber), "--body-file", bodyFile], {
              stdio: ["pipe", "ignore", "ignore"],
            });
            sleep(COMMENT_POST_SLEEP);
          })
        );
      } catch {
        console.error(`  Warning: Failed to create ${kind} artifact: ${name}`);
        hadError = true;
      }
    }
  }

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
          runWithRetry(`create ${kind}:${name} on issue #${issueNumber}`, () =>
            withTempBodyFile(`issue-comment-${issueNumber}-${kind}-${name}`, commentBody, (bodyFile) => {
              gh(["issue", "comment", String(issueNumber), "--body-file", bodyFile], {
                stdio: ["pipe", "ignore", "ignore"],
              });
              sleep(COMMENT_POST_SLEEP);
            })
          );
        } catch {
          console.error(`  Warning: Failed to create ${kind} artifact: ${name}`);
          hadError = true;
        }
      }
    }
  }
  return !hadError;
}

function parseIssueSections(body) {
  const sections = {};
  const regex = /^##\s+(.+)$/gm;
  const matches = [...body.matchAll(regex)];
  for (let i = 0; i < matches.length; i++) {
    const title = matches[i][1].trim();
    const start = matches[i].index + matches[i][0].length;
    const end = i + 1 < matches.length ? matches[i + 1].index : body.length;
    sections[title] = body.slice(start, end).trim();
  }
  return sections;
}

function buildArtifactCommentBody(kind, name, content) {
  return `<!-- GWT_SPEC_ARTIFACT:${kind}:${name} -->\n${kind}:${name}\n\n${content.trim()}\n`;
}

function migrateExistingBodyCanonicalIssues() {
  console.log("");
  console.log("Scanning existing gwt-spec issues for body-canonical migration...");

  let issues = [];
  try {
    issues = JSON.parse(
      gh(["issue", "list", "--label", "gwt-spec", "--state", "all", "--limit", "200", "--json", "number,title,body"])
    );
  } catch {
    console.error("  Failed to list existing gwt-spec issues");
    return { converted: 0, failed: 0 };
  }

  const candidates = issues.filter((issue) => {
    const body = issue.body || "";
    return body.includes("## Spec") && !body.includes("## Artifact Index");
  });

  console.log(`Found ${candidates.length} body-canonical issues to migrate`);
  let converted = 0;
  let failedCount = 0;

  for (const issue of candidates) {
    console.log(`  Migrating issue #${issue.number}...`);
    const sections = parseIssueSections(issue.body || "");
    const artifacts = [
      ["doc", "spec.md", sections["Spec"] || ""],
      ["doc", "plan.md", sections["Plan"] || ""],
      ["doc", "tasks.md", sections["Tasks"] || ""],
      ["checklist", "tdd.md", sections["TDD"] || ""],
      ["doc", "research.md", sections["Research"] || ""],
      ["doc", "data-model.md", sections["Data Model"] || ""],
      ["doc", "quickstart.md", sections["Quickstart"] || ""],
      ["checklist", "acceptance.md", sections["Acceptance Checklist"] || ""],
    ].filter(([, , content]) => content && content.trim().length > 0);

    if (DRY_RUN) {
      console.log(`  [dry-run] Would migrate issue #${issue.number} to artifact-first format`);
      continue;
    }

    let ok = true;
    for (const [kind, name, content] of artifacts) {
      try {
        runWithRetry(`create ${kind}:${name} on issue #${issue.number}`, () =>
          withTempBodyFile(
            `issue-comment-${issue.number}-${kind}-${name}`,
            buildArtifactCommentBody(kind, name, content),
            (bodyFile) => {
              gh(["issue", "comment", String(issue.number), "--body-file", bodyFile], {
                stdio: ["pipe", "ignore", "ignore"],
              });
              sleep(COMMENT_POST_SLEEP);
            }
          )
        );
      } catch {
        console.error(`  Warning: failed to add ${kind}:${name} to issue #${issue.number}`);
        ok = false;
      }
    }

    try {
      runWithRetry(`rewrite body for issue #${issue.number}`, () =>
        withTempBodyFile(`issue-edit-${issue.number}`, buildIssueBody(`#${issue.number}`), (bodyFile) => {
          gh(["issue", "edit", String(issue.number), "--body-file", bodyFile], {
            stdio: ["pipe", "ignore", "ignore"],
          });
        })
      );
    } catch {
      console.error(`  Warning: failed to rewrite issue #${issue.number} body`);
      ok = false;
    }

    if (ok) {
      converted++;
    } else {
      failedCount++;
    }

    if (converted % RATE_LIMIT_BATCH === 0) {
      console.log(`  Rate limit pause: sleeping ${RATE_LIMIT_SLEEP}s...`);
      sleep(RATE_LIMIT_SLEEP);
    }
  }

  return { converted, failed: failedCount };
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

  const issueBody = buildIssueBody(specName);
  const labelArgs = ["--label", "gwt-spec"];
  for (const label of EXTRA_LABELS) {
    labelArgs.push("--label", label);
  }

  let issueUrl;
  try {
    issueUrl = runWithRetry(`create issue for ${specName}`, () =>
      withTempBodyFile(`issue-create-${specName}`, issueBody, (bodyFile) => {
        return gh(["issue", "create", "--title", title, "--body-file", bodyFile, ...labelArgs]).trim();
      })
    );
  } catch {
    console.error(`  Failed to create issue for ${specName}`);
    reportEntries.push({ oldSpecId: specName, issueNumber: 0, title, status: "failed" });
    failed++;
    continue;
  }

  let migrationOk = true;
  const issueNumber = issueUrl.split("/").pop();
  console.log(`  Created issue #${issueNumber}`);

  const updatedBody = buildIssueBody(`#${issueNumber}`);
  try {
    runWithRetry(`rewrite body for issue #${issueNumber}`, () =>
      withTempBodyFile(`issue-edit-${issueNumber}`, updatedBody, (bodyFile) => {
        gh(["issue", "edit", String(issueNumber), "--body-file", bodyFile], {
          stdio: ["pipe", "ignore", "ignore"],
        });
      })
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

if (CONVERT_EXISTING_ISSUES) {
  const converted = migrateExistingBodyCanonicalIssues();
  success += converted.converted;
  failed += converted.failed;
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
