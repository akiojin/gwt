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
  const read = (endpoint, extra = []) => {
    const result = spawn("gh", ["api", "--method", "GET", `repos/${repository}/${endpoint}`, ...extra],
      { encoding: "utf8", maxBuffer: 128 * 1024 * 1024 });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(result.stderr?.trim() || `gh exited ${result.status}`);
    return JSON.parse(result.stdout);
  };
  const pages = read("issues?state=closed&per_page=100", ["--paginate", "--slurp"]);
  if (!Array.isArray(pages) || !pages.every(Array.isArray)) {
    throw new Error("Expected page arrays from gh api --paginate --slurp");
  }
  return pages.flat().flatMap((issue) => {
    if (issue.pull_request || issue.state !== "closed" || issue.state_reason !== "completed") return [];
    const unchecked = uncheckedAcceptance(acceptanceSource(issue, read));
    return unchecked.length ? [{ number: issue.number, url: issue.html_url, unchecked }] : [];
  });
}

// Follow the SPEC routing index, never stale/unreferenced artifact comments.
function acceptanceSource(issue, read) {
  const body = issue.body ?? "";
  const isSpec = issue.labels?.some((label) =>
    (typeof label === "string" ? label : label.name)?.toLowerCase() === "gwt-spec");
  if (!isSpec) return body;
  const index = body.match(/<!--\s*sections:\s*([\s\S]*?)-->/)?.[1];
  const route = index?.match(/^[ \t]*spec[ \t]*=(.*)$/m)?.[1].trim();
  if (route === undefined || route === "body") return body;
  const chunks = route.split(",").map((entry) => {
    const id = entry.trim().match(/^comment:(\d+)$/)?.[1];
    if (!id) throw new Error(`Issue #${issue.number}: invalid SPEC comment route`);
    const comment = read(`issues/comments/${id}`);
    if (!issue.url || comment.issue_url !== issue.url) {
      throw new Error(`Issue #${issue.number}: SPEC comment belongs to another Issue (comment:${id})`);
    }
    const section = comment.body?.match(/<!--\s*artifact:spec\s+BEGIN(?:\s+part=(\d+)\/(\d+))?\s*-->([\s\S]*?)<!--\s*artifact:spec\s+END(?:\s+part=(\d+)\/(\d+))?\s*-->/);
    if (!section) throw new Error(`Issue #${issue.number}: missing SPEC section in comment:${id}`);
    if (section[1] !== section[4] || section[2] !== section[5]) {
      throw new Error(`Issue #${issue.number}: inconsistent SPEC part markers`);
    }
    return { part: Number(section[1] ?? 0), total: Number(section[2] ?? 0), text: section[3] };
  });
  // Legacy unmarked multi-comment sections preserve routing order.
  if (chunks.some((chunk) => chunk.part || chunk.total)) {
    chunks.sort((a, b) => a.part - b.part);
    if (chunks.some((chunk, i) => chunk.total !== chunks.length || chunk.part !== i + 1)) {
      throw new Error(`Issue #${issue.number}: incomplete SPEC parts`);
    }
  }
  return `${body}\n\n${chunks.map((chunk) => chunk.text).join("\n")}`;
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
