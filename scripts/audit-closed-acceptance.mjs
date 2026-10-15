#!/usr/bin/env node
// Issue #5126: find completed Issues whose acceptance checklist is unfinished.
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const acceptanceHeadings = new Set([
  "acceptance criteria", "受け入れ基準", "受け入れ条件", "受け入れシナリオ",
]);

export function uncheckedAcceptance(body = "") {
  const unchecked = [];
  let inAcceptance = false;
  let fence;
  for (const line of body.split(/\r?\n/)) {
    const marker = line.match(/^ {0,3}(`{3,}|~{3,})/);
    if (fence) {
      if (marker && marker[1][0] === fence[0] && marker[1].length >= fence.length && line.trim() === marker[1]) fence = undefined;
      continue;
    }
    if (marker) {
      fence = marker[1];
      continue;
    }
    // Four-space/tab indentation is Markdown code, not an acceptance item.
    if (/^( {4}|\t)/.test(line)) continue;
    const heading = line.match(/^ {0,3}#+\s*(.*?)\s*#*\s*$/);
    if (heading) {
      inAcceptance = acceptanceHeadings.has(heading[1].toLowerCase());
      continue;
    }
    if (!inAcceptance) continue;
    if (/^\s*<!--.*artifact:/.test(line)) {
      inAcceptance = false;
      continue;
    }
    const item = line.match(/^\s*[-*] \[ \]\s*(.*)$/);
    if (!item) continue;
    const explicitId = item[1].match(/^(AC-[A-Za-z0-9_.-]+)\s*:/);
    unchecked.push(explicitId?.[1] ?? item[1]);
  }
  return unchecked;
}

export function auditClosedAcceptance(repository, spawn = spawnSync) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) {
    throw new Error("Usage: node scripts/audit-closed-acceptance.mjs <owner/repository>");
  }
  const result = spawn("gh", [
    "api", "--method", "GET", `repos/${repository}/issues?state=closed&per_page=100`,
    "--paginate", "--slurp",
  ], { encoding: "utf8", maxBuffer: 128 * 1024 * 1024 });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(result.stderr?.trim() || `gh exited ${result.status}`);
  const pages = JSON.parse(result.stdout);
  if (!Array.isArray(pages) || !pages.every(Array.isArray)) {
    throw new Error("Expected page arrays from gh api --paginate --slurp");
  }
  return pages.flat().flatMap((issue) => {
    if (issue.pull_request || issue.state !== "closed" || issue.state_reason !== "completed") return [];
    const unchecked = uncheckedAcceptance(issue.body ?? "");
    return unchecked.length ? [{ number: issue.number, url: issue.html_url, unchecked }] : [];
  });
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv.length !== 3) throw new Error("Usage: node scripts/audit-closed-acceptance.mjs <owner/repository>");
    console.log(JSON.stringify(auditClosedAcceptance(process.argv[2]), null, 2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
