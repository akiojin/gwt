#!/usr/bin/env node

import { createHash } from "node:crypto";
import { createWriteStream } from "node:fs";
import { mkdir, realpath, writeFile } from "node:fs/promises";
import { basename, dirname, join } from "node:path";
import { once } from "node:events";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const checkHome = await requiredDirectory("GWT_PLAYWRIGHT_CHECK_HOME");
const projectRoot = await requiredDirectory("GWT_PLAYWRIGHT_PROJECT_ROOT");
const issueNumber = positiveInteger("GWT_PLAYWRIGHT_ISSUE_NUMBER", 3777);
const controlIssueNumber = positiveInteger("GWT_PLAYWRIGHT_CONTROL_ISSUE_NUMBER", 3772);
const stressWorkId =
  process.env.GWT_PLAYWRIGHT_STRESS_WORK_ID?.trim() ||
  "work-issue-3777-stress-0000";
const { stdout: originOutput } = await execFileAsync("git", [
  "-C",
  projectRoot,
  "remote",
  "get-url",
  "origin",
]);
const repoHash = createHash("sha256")
  .update(normalizeOrigin(originOutput.trim()))
  .digest("hex")
  .slice(0, 16);
const fixtureDirectory = join(checkHome, ".gwt", "issue-3777-fixtures");
const boardPath = join(
  checkHome,
  ".gwt",
  "projects",
  repoHash,
  "coordination",
  "events",
  "0000000000000001.jsonl",
);
const workspacePath = join(
  checkHome,
  ".gwt",
  "projects",
  repoHash,
  "workspace.json",
);
const pmPreferencesPath = join(
  checkHome,
  ".gwt",
  "projects",
  repoHash,
  "project-state",
  "pm.json",
);
const workItemsPath = join(fixtureDirectory, "works.fixture.json");
const hookProfilePath = join(checkHome, ".gwt", "issue-3777-hook-profile.jsonl");
await mkdir(dirname(boardPath), { recursive: true });
await mkdir(fixtureDirectory, { recursive: true });
await writeBoardFixture(boardPath);
await writeWorkItemsFixture(workItemsPath, stressWorkId);
await writeFile(
  workspacePath,
  `${JSON.stringify({
    next_z_index: 2,
    viewport: { x: 0, y: 0, zoom: 1 },
    windows: [
      {
        geometry: { x: 96, y: 96, width: 880, height: 520 },
        geometry_revision: 0,
        id: "work-issue-3777-setup",
        is_pm: false,
        lane_kind: "unknown",
        persist: true,
        preset: "work",
        status: "running",
        tab_group_active: false,
        title: "Workspace",
        z_index: 1,
      },
    ],
  }, null, 2)}\n`,
  "utf8",
);
await mkdir(dirname(pmPreferencesPath), { recursive: true });
await writeFile(
  pmPreferencesPath,
  `${JSON.stringify({
    registration: null,
    settings: { auto_start: false, loop_interval_secs: 60 },
  }, null, 2)}\n`,
  "utf8",
);
await writeFile(hookProfilePath, "", "utf8");

const cacheRoot = join(checkHome, ".gwt", "cache", "issues", repoHash);
await writeIssueFixture(
  cacheRoot,
  issueNumber,
  "perf(ui): prompt and RuntimeHook responsiveness",
);
await writeIssueFixture(
  cacheRoot,
  controlIssueNumber,
  "Issue #3777 browser-check control fixture",
);
process.stdout.write(
  [
    `GWT_PLAYWRIGHT_BOARD_FIXTURE_PATH=${boardPath}`,
    `GWT_PLAYWRIGHT_WORK_ITEMS_FIXTURE_PATH=${workItemsPath}`,
    `GWT_PLAYWRIGHT_HOOK_PROFILE_PATH=${hookProfilePath}`,
    `GWT_PLAYWRIGHT_ISSUE_META_PATH=${join(cacheRoot, String(issueNumber), "meta.json")}`,
    `GWT_PLAYWRIGHT_CONTROL_ISSUE_META_PATH=${join(cacheRoot, String(controlIssueNumber), "meta.json")}`,
    `GWT_PLAYWRIGHT_STRESS_WORK_ID=${stressWorkId}`,
    `# repo=${basename(projectRoot)} hash=${repoHash}`,
  ].join("\n") + "\n",
);

async function requiredDirectory(name) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required`);
  return realpath(value);
}

function positiveInteger(name, fallback) {
  const value = Number(process.env[name] || fallback);
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`${name} must be a positive integer`);
  }
  return value;
}

function normalizeOrigin(origin) {
  return origin
    .replace(/^git@([^:]+):/, "$1/")
    .replace(/^[a-z]+:\/\//i, "")
    .replace(/\.git$/i, "")
    .replace(/\/$/, "")
    .toLowerCase();
}

async function writeBoardFixture(path) {
  const stream = createWriteStream(path, { encoding: "utf8" });
  const body = "bounded-board-history ".repeat(180);
  let bytes = 0;
  for (let index = 0; bytes < 8 * 1024 * 1024; index += 1) {
    const line = `${JSON.stringify({
      type: "message_appended",
      entry: {
        id: `issue-3777-board-${String(index).padStart(8, "0")}`,
        author_kind: "agent",
        author: "fixture",
        kind: "status",
        body,
        state: "working",
        parent_id: null,
        created_at: "2026-08-29T00:00:00Z",
        updated_at: "2026-08-29T00:00:00Z",
        related_topics: [],
        related_owners: [],
        origin_branch: null,
        origin_session_id: null,
        origin_agent_id: null,
        target_owners: [],
      },
    })}\n`;
    bytes += Buffer.byteLength(line);
    if (!stream.write(line)) await once(stream, "drain");
  }
  stream.end();
  await once(stream, "finish");
}

async function writeWorkItemsFixture(path, firstWorkId) {
  const stream = createWriteStream(path, { encoding: "utf8" });
  stream.write('{"updated_at":"2026-08-29T00:00:00Z","work_items":[');
  const duplicateSummary = "repository-scale duplicate provenance ".repeat(5);
  for (let index = 0; index < 256; index += 1) {
    const id = index === 0 ? firstWorkId : `work-issue-3777-stress-${String(index).padStart(4, "0")}`;
    // Many small provenance records make serde perform realistic repository-
    // scale object decoding while keeping the UI projection itself compact.
    // A single giant display summary would instead benchmark a 160MiB browser
    // payload and obscure the RuntimeHook / background-ingest boundary.
    const duplicateEventContainers = Object.fromEntries(
      Array.from({ length: 2_048 }, (_, duplicateIndex) => {
        const suffix = String(duplicateIndex).padStart(5, "0");
        return [
          `duplicate-${suffix}`,
          [{
            id: `event-${index}-${suffix}`,
            work_item_id: id,
            kind: "update",
            summary: duplicateSummary,
            updated_at: "2026-08-29T00:00:00Z",
          }],
        ];
      }),
    );
    const row = JSON.stringify({
      id,
      title: `Issue #3777 stress Work ${index}`,
      intent: "Exercise repository-scale Active Work projection preparation",
      summary: `Repository-scale Work ${index}`,
      status_category: "active",
      owner: "Issue #3777",
      created_at: "2026-08-29T00:00:00Z",
      updated_at: "2026-08-29T00:00:00Z",
      duplicate_event_containers: duplicateEventContainers,
    });
    if (index > 0) stream.write(",");
    if (!stream.write(row)) await once(stream, "drain");
  }
  stream.end("]}\n");
  await once(stream, "finish");
}

async function writeIssueFixture(cacheRoot, number, title) {
  const directory = join(cacheRoot, String(number));
  await mkdir(directory, { recursive: true });
  await writeFile(
    join(directory, "meta.json"),
    `${JSON.stringify({
      comment_ids: [],
      labels: ["bug"],
      number,
      state: "open",
      title,
      updated_at: "2026-08-29T00:00:00Z",
    }, null, 2)}\n`,
    "utf8",
  );
  await writeFile(
    join(directory, "body.md"),
    `# ${title}\n\nDeterministic Issue #${number} browser-check fixture.\n`,
    "utf8",
  );
  await writeFile(join(directory, "linked_prs.json"), "[]\n", "utf8");
}
