// SPEC-3165 — Issue Monitor surface controls and queue status.

import assert from "node:assert/strict";
import { test } from "node:test";
import { parseHTML } from "linkedom";

import { createIssueMonitorSurface } from "../issue-monitor-surface.js";

function makeFixture() {
  const { document } = parseHTML("<!doctype html><html><head></head><body></body></html>");
  const sent = [];
  const surface = createIssueMonitorSurface({
    document,
    send: (event) => sent.push(event),
  });
  surface.mount(document.body);
  return { document, sent, surface };
}

function issue(number, state = "queued", labels = []) {
  return {
    issue: {
      number,
      title: `Issue ${number}`,
      labels,
      state: "open",
      body: `Body for issue ${number}`,
      url: `https://github.com/example/repo/issues/${number}`,
    },
    state,
    claim_id: null,
    blocked_by_owner: null,
    claim_expires_at: null,
    launched_window_id: null,
    error_message: null,
    launch_plan: {
      branch_name: labels.includes("gwt-spec") ? `feature/spec-${number}` : `work/issue-${number}`,
      linked_issue_kind: labels.includes("gwt-spec") ? "spec" : "issue",
      prompt: labels.includes("gwt-spec") ? `$gwt-build-spec SPEC-${number}` : `$gwt-fix-issue #${number}`,
    },
  };
}

test("Issue Monitor exposes Start/Stop while keeping queued issue controls visible", () => {
  const { document, sent, surface } = makeFixture();
  surface.applyStatus({
    enabled: false,
    state: "disabled",
    queue_len: 2,
    active_count: 0,
    max_active_agents: 2,
    total_candidates: 2,
    launch_profile_source: "last_settings",
    launch_profile_summary: "codex / gpt-5.5 / high / host",
  });
  surface.applyInbox([issue(42, "queued", ["gwt-spec"]), issue(43, "launching")]);

  const toggle = document.querySelector(".issue-monitor-card__toggle");
  assert.equal(toggle.textContent, "Start");
  toggle.click();
  assert.equal(toggle.textContent, "Stop");
  assert.equal(document.querySelector(".issue-monitor-card__state").textContent, "Starting");
  assert.deepEqual(sent.at(-1), {
    kind: "set_issue_monitor_enabled",
    enabled: true,
  });

  const rows = [...document.querySelectorAll(".issue-monitor-card__item")];
  assert.equal(rows.length, 2);
  assert.match(
    document.querySelector(".issue-monitor-card__settings").textContent,
    /Agent settings Last settings: codex \/ gpt-5\.5 \/ high \/ host/,
  );
  assert.equal(rows[0].querySelector(".issue-monitor-card__state-badge").textContent, "Queued");
  assert.match(rows[0].textContent, /Prompt: \$gwt-build-spec SPEC-42/);
  assert.match(rows[0].textContent, /Branch: feature\/spec-42/);
  assert.equal(rows[1].querySelector(".issue-monitor-card__state-badge").textContent, "Launching");

  const up = rows[0].querySelector('[data-action="move-up"]');
  const down = rows[0].querySelector('[data-action="move-down"]');
  assert.equal(up.textContent, "↑");
  assert.equal(up.getAttribute("aria-label"), "Move up");
  assert.equal(down.textContent, "↓");
  assert.equal(down.getAttribute("aria-label"), "Move down");

  const configure = rows[0].querySelector('[data-action="configure-issue"]');
  assert.equal(configure.textContent, "Configure");
  configure.click();
  assert.deepEqual(sent.at(-1), {
    kind: "issue_monitor_configure_issue",
    issue_number: 42,
    linked_issue_kind: "spec",
  });

  const launchNow = rows[0].querySelector('[data-action="launch-now"]');
  assert.equal(launchNow.textContent, "Launch now");
  launchNow.click();
  assert.deepEqual(sent.at(-1), {
    kind: "issue_monitor_launch_now",
    issue_number: 42,
    linked_issue_kind: "spec",
  });

  surface.applyStatus({ enabled: true, state: "idle" });
  assert.equal(toggle.textContent, "Stop");
  toggle.click();
  assert.equal(toggle.textContent, "Start");
  assert.equal(document.querySelector(".issue-monitor-card__state").textContent, "Stopped");
  assert.deepEqual(sent.at(-1), {
    kind: "set_issue_monitor_enabled",
    enabled: false,
  });
});

test("Issue Monitor arrows reorder visible queued rows and Detail opens a modal", () => {
  const { document, sent, surface } = makeFixture();
  surface.applyInbox([issue(42, "queued", ["bug"]), issue(43, "queued"), issue(44, "queued")]);

  let rows = [...document.querySelectorAll(".issue-monitor-card__item")];
  rows[0].querySelector('[data-action="move-down"]').click();

  rows = [...document.querySelectorAll(".issue-monitor-card__item")];
  assert.match(rows[0].textContent, /#43 Issue 43/);
  assert.match(rows[1].textContent, /#42 Issue 42/);
  assert.deepEqual(sent.at(-1), {
    kind: "reorder_issue_monitor_issues",
    issue_numbers: [43, 42, 44],
  });

  const detailButton = rows[1].querySelector('[data-action="open-detail"]');
  assert.equal(detailButton.textContent, "Detail");
  detailButton.click();

  const modal = document.getElementById("issue-monitor-detail-modal");
  assert.ok(modal, "Detail should open a modal");
  assert.ok(modal.classList.contains("modal-backdrop"));
  assert.ok(modal.classList.contains("open"));
  assert.equal(modal.querySelector('[role="dialog"]').getAttribute("aria-modal"), "true");
  assert.ok(modal.querySelector('[role="dialog"]').classList.contains("modal-shell"));
  assert.match(modal.textContent, /#42 Issue 42/);
  assert.match(modal.textContent, /\$gwt-fix-issue #42/);
  assert.match(modal.textContent, /work\/issue-42/);
  assert.match(modal.textContent, /Body for issue 42/);
  assert.match(
    modal.textContent,
    /https:\/\/github\.com\/example\/repo\/issues\/42/,
  );

  modal.querySelector(".issue-monitor-detail-modal__close").click();
  assert.equal(document.getElementById("issue-monitor-detail-modal"), null);
});

test("Issue Monitor CSS composes Operator tokens instead of raw colors", () => {
  const { document } = makeFixture();
  const css = document.getElementById("issue-monitor-surface-style").textContent;

  assert.doesNotMatch(css, /#[0-9a-fA-F]{3,8}\b/);
  assert.doesNotMatch(css, /\brgba?\(/);
  assert.match(css, /var\(--color-surface\)/);
  assert.match(css, /var\(--color-text\)/);
  assert.match(css, /var\(--radius-/);
});

test("Issue Monitor follows the Workspace window hierarchy", () => {
  const { document, surface } = makeFixture();
  surface.applyInbox([issue(42, "queued", ["gwt-spec"]), issue(43, "launching")]);

  assert.ok(document.querySelector(".issue-monitor-card__toolbar"));
  assert.equal(document.querySelector(".issue-monitor-card__header"), null);
  assert.equal(document.querySelector(".issue-monitor-card__icon"), null);

  const rows = [...document.querySelectorAll(".issue-monitor-card__item")];
  assert.equal(rows.length, 2);
  assert.equal(rows[0].getAttribute("role"), "listitem");
  assert.equal(document.querySelector(".issue-monitor-card__inbox").getAttribute("role"), "list");
  assert.ok(rows[0].querySelector(".issue-monitor-card__status-dot"));
  assert.equal(
    rows[0].querySelector(".issue-monitor-card__issue-meta") ? "present" : "absent",
    "absent",
  );

  const detail = rows[0].querySelector('[data-action="open-detail"]');
  const configure = rows[0].querySelector('[data-action="configure-issue"]');
  const launchNow = rows[0].querySelector('[data-action="launch-now"]');
  assert.ok(detail.classList.contains("wizard-button"));
  assert.ok(detail.classList.contains("is-compact"));
  assert.ok(configure.classList.contains("wizard-button"));
  assert.ok(launchNow.classList.contains("wizard-button"));

  const moveUp = rows[0].querySelector('[data-action="move-up"]');
  const moveDown = rows[0].querySelector('[data-action="move-down"]');
  assert.ok(moveUp.classList.contains("icon-button"));
  assert.ok(moveDown.classList.contains("icon-button"));

  const css = document.getElementById("issue-monitor-surface-style").textContent;
  assert.doesNotMatch(css, /\.issue-monitor-card__button\b/);
  assert.match(css, /\.issue-monitor-card__toolbar\s*{/);
  assert.match(css, /\.issue-monitor-card__row-button\s*{[^}]*background: transparent/s);
  assert.match(css, /\.issue-monitor-card__item\s*{[^}]*border-left: 2px solid transparent/s);
});

test("Issue Monitor renders claim authentication blockage as an error", () => {
  const { document, surface } = makeFixture();
  const help =
    "GitHub authentication is required before automatic Issue Monitor launches can claim Issues. Configure it on the host terminal with: gh auth login --hostname github.com --git-protocol https --scopes repo,read:org; gh auth setup-git. Then verify: gh auth status --hostname github.com; git ls-remote origin HEAD.";
  surface.applyStatus({
    enabled: true,
    state: "error",
    queue_len: 1,
    active_count: 0,
    max_active_agents: 1,
    total_candidates: 1,
    last_error: help,
  });
  surface.applyInbox([issue(42, "queued", ["bug"])]);

  assert.equal(document.querySelector(".issue-monitor-card__state").textContent, "Error");
  assert.equal(document.querySelector(".issue-monitor-card__error").dataset.visible, "true");
  assert.match(
    document.querySelector(".issue-monitor-card__error").textContent,
    /GitHub authentication is required/,
  );
  assert.match(
    document.querySelector(".issue-monitor-card__error").textContent,
    /gh auth setup-git/,
  );
  const css = document.getElementById("issue-monitor-surface-style").textContent;
  assert.match(css, /\.issue-monitor-card__error\s*{[^}]*white-space: pre-wrap/s);
  assert.match(document.querySelector(".issue-monitor-card__detail").textContent, /Queue 1/);
  assert.match(document.querySelector(".issue-monitor-card__item").textContent, /#42 Issue 42/);
});

test("Issue Monitor shows when launch settings are required before auto-run", () => {
  const { document, surface } = makeFixture();
  surface.applyStatus({
    enabled: true,
    state: "settings_required",
    queue_len: 1,
    active_count: 0,
    max_active_agents: 5,
    total_candidates: 1,
    launch_profile_source: "default",
    launch_profile_summary: "configure before auto start",
  });
  surface.applyInbox([issue(3165, "queued", ["gwt-spec"])]);

  assert.equal(document.querySelector(".issue-monitor-card__state").textContent, "Settings required");
  assert.match(document.querySelector(".issue-monitor-card__detail").textContent, /Active 0\/5/);
  assert.match(
    document.querySelector(".issue-monitor-card__settings").textContent,
    /Agent settings Default: configure before auto start/,
  );
  assert.match(document.querySelector(".issue-monitor-card__item").textContent, /Queued/);
});

test("Issue Monitor launch failure event marks the row and header as failed", () => {
  const { document, surface } = makeFixture();
  surface.applyStatus({
    enabled: true,
    state: "launching",
    queue_len: 1,
    active_count: 1,
    active_issue_number: 42,
    max_active_agents: 1,
    total_candidates: 2,
  });
  surface.applyInbox([issue(42, "launching"), issue(43, "queued")]);

  surface.applyLaunchFailed({
    issue_number: 42,
    message: "remote branch failed",
  });

  const rows = [...document.querySelectorAll(".issue-monitor-card__item")];
  assert.equal(rows[0].querySelector(".issue-monitor-card__state-badge").textContent, "Launch failed");
  assert.match(rows[0].textContent, /Error: remote branch failed/);
  assert.equal(
    rows[0].querySelector(".issue-monitor-card__state-badge").dataset.state,
    "launch_failed",
  );
  assert.equal(document.querySelector(".issue-monitor-card__state").textContent, "Error");
  assert.match(document.querySelector(".issue-monitor-card__detail").textContent, /Active 0\/1/);
  assert.match(
    document.querySelector(".issue-monitor-card__error").textContent,
    /issue #42: remote branch failed/,
  );
});

test("Issue Monitor renders agent runtime failures in the row and detail modal", () => {
  const { document, surface } = makeFixture();
  surface.applyStatus({
    enabled: true,
    state: "error",
    queue_len: 0,
    active_count: 0,
    active_issue_number: null,
    max_active_agents: 5,
    total_candidates: 1,
    last_error: "issue #3164: Stop-block hit an error",
  });
  surface.applyInbox([
    {
      ...issue(3164, "agent_failed", ["gwt-spec"]),
      error_message: "Stop-block hit an error",
    },
  ]);

  const row = document.querySelector(".issue-monitor-card__item");
  assert.equal(row.dataset.state, "agent_failed");
  assert.equal(row.querySelector(".issue-monitor-card__state-badge").textContent, "Agent failed");
  assert.equal(row.querySelector(".issue-monitor-card__state-badge").dataset.state, "agent_failed");
  assert.match(row.textContent, /Error: Stop-block hit an error/);
  assert.match(
    document.querySelector(".issue-monitor-card__error").textContent,
    /issue #3164: Stop-block hit an error/,
  );
  assert.ok(row.querySelector('[data-action="configure-issue"]'));
  assert.ok(row.querySelector('[data-action="launch-now"]'));

  row.querySelector('[data-action="open-detail"]').click();
  const modal = document.getElementById("issue-monitor-detail-modal");
  assert.ok(modal, "Detail should open for failed rows");
  assert.match(modal.textContent, /Agent failed/);
  assert.match(modal.textContent, /Stop-block hit an error/);
  assert.match(modal.textContent, /\$gwt-build-spec SPEC-3164/);
});

test("autonomous toggle sends the control and reflects status", () => {
  // SPEC #3200 T-047/FR-024: an Autonomous toggle arms/disarms unattended mode.
  const { document, sent, surface } = makeFixture();
  const button = document.querySelector(".issue-monitor-card__autonomous");
  assert.ok(button, "autonomous toggle is rendered");
  assert.equal(button.textContent, "Autonomous: OFF");

  button.click();
  const control = sent.find((e) => e.kind === "set_issue_monitor_autonomous_mode");
  assert.ok(control, "clicking sends the autonomous-mode control");
  assert.equal(control.enabled, true);
  assert.equal(button.textContent, "Autonomous: ON", "optimistic ON");

  // Backend status echo keeps it ON and exposes the indicator.
  surface.applyStatus({ enabled: true, autonomous_mode: true });
  assert.equal(button.dataset.enabled, "true");
  assert.match(
    document.querySelector(".issue-monitor-card__detail").textContent,
    /Autonomous ON/,
  );
});

test("per-issue NeedsHuman / phase / attempts surface from autonomous_issues", () => {
  // SPEC #3200 T-090/FR-033: the autonomous lifecycle is visible per issue.
  const { document, surface } = makeFixture();
  surface.applyStatus({
    enabled: true,
    autonomous_mode: true,
    autonomous_issues: [
      { issue_number: 3164, phase: "needs_human", attempts: 3, needs_human: true },
      { issue_number: 3165, phase: "reviewing", attempts: 1, needs_human: false },
    ],
  });
  surface.applyInbox([issue(3164, "needs_human", ["auto-merge"]), issue(3165, "launched", ["auto-merge"])]);

  const rows = document.querySelectorAll(".issue-monitor-card__item");
  const meta3164 = rows[0].querySelector(".issue-monitor-card__autonomous-meta");
  assert.ok(meta3164, "issue 3164 has an autonomous meta line");
  assert.equal(meta3164.dataset.needsHuman, "true");
  assert.match(meta3164.textContent, /Needs human/);
  assert.match(meta3164.textContent, /Attempts 3/);

  const meta3165 = rows[1].querySelector(".issue-monitor-card__autonomous-meta");
  assert.equal(meta3165.dataset.needsHuman, "false");
  assert.match(meta3165.textContent, /Phase reviewing/);

  // The detail line shows the needs-human count.
  assert.match(
    document.querySelector(".issue-monitor-card__detail").textContent,
    /1 need human/,
  );
});
