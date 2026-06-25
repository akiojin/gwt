import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { parseHTML } from "linkedom";
import { createImprovementInboxSurface } from "../improvement-inbox-surface.js";

function createFixture() {
  const { document } = parseHTML("<!doctype html><html><body></body></html>");
  const sent = [];
  const surface = createImprovementInboxSurface({
    createNode(tag, className, text) {
      const element = document.createElement(tag);
      if (className) element.className = className;
      if (text != null) element.textContent = text;
      return element;
    },
    send(message) {
      sent.push(message);
    },
  });
  return { document, body: document.body, sent, surface };
}

const sampleCandidates = [
  {
    id: "impr-pending",
    state: "pending",
    confidence: "high",
    target_artifact: "skill",
    classification: "gwt-caused",
    summary: "Skill failed to update after agent failure",
    dedupe_key: "skill:gwt-discussion:self-improvement",
    occurrences: 2,
    issue_preview: {
      repository: "akiojin/gwt",
      title: "fix(gwt): Skill failed to update after agent failure",
      body: [
        "## Problem",
        "",
        "Skill failed to update after agent failure",
        "",
        "Context:",
        "",
        "Stop hook allowed completion without capture.",
        "",
        "## Expected behavior",
        "",
        "gwt should handle `skill` self-improvement failures with enough public-safe context.",
        "",
        "## Observed evidence",
        "",
        "High-confidence contract violation with redacted evidence.",
        "",
        "## Impact",
        "",
        "Repeated agent failures stay local instead of becoming trackable upstream work.",
        "",
        "## Suggested verification",
        "",
        "- Confirm whether the gwt-owned skill behavior still violates the expected contract.",
        "- Add or update a regression test that fails before the fix.",
        "",
        "## Source candidate",
        "",
        "- Candidate ID: impr-pending",
        "- Target artifact: skill",
        "",
        "## Privacy",
        "",
        "- Public body generated from sanitized candidate fields only.",
      ].join("\n"),
    },
  },
  {
    id: "impr-promoted",
    state: "promoted",
    confidence: "high",
    target_artifact: "verification",
    classification: "gwt-caused",
    summary: "Verification skip regression",
    linked_issue: {
      number: 3164,
      url: "https://github.com/akiojin/gwt/issues/3164",
      repository: "akiojin/gwt",
    },
    occurrences: 1,
  },
  {
    id: "impr-dismissed",
    state: "dismissed",
    confidence: "medium",
    target_artifact: "unknown",
    classification: "ambiguous",
    summary: "False positive",
    dismissed_reason: "Not a gwt problem",
    occurrences: 1,
  },
];

test("Improvement Inbox renders candidate states and issue links", () => {
  const fixture = createFixture();
  fixture.surface.mount(fixture.body, {
    id: "improvements",
    improvement_candidates: sampleCandidates,
  });

  assert.ok(fixture.body.querySelector(".improvement-inbox-root"));
  assert.deepEqual(
    Array.from(fixture.body.querySelectorAll("[data-improvement-state]"), (row) => [
      row.dataset.improvementId,
      row.dataset.improvementState,
    ]),
    [
      ["impr-pending", "pending"],
      ["impr-promoted", "promoted"],
      ["impr-dismissed", "dismissed"],
    ],
  );
  assert.match(fixture.body.textContent, /Skill failed to update/);
  assert.match(fixture.body.textContent, /Verification skip regression/);
  assert.match(fixture.body.textContent, /Target:/);
  assert.match(fixture.body.textContent, /Cause:/);
  assert.match(fixture.body.textContent, /#3164/);
  assert.match(fixture.body.textContent, /Not a gwt problem/);
});

test("Improvement Inbox separates review candidates from processed history tabs", () => {
  const fixture = createFixture();
  fixture.surface.mount(fixture.body, {
    id: "improvements",
    improvement_candidates: sampleCandidates,
  });

  const reviewTab = fixture.body.querySelector("[data-improvement-tab='needs-review']");
  const historyTab = fixture.body.querySelector("[data-improvement-tab='history']");
  assert.ok(reviewTab, "Needs Review tab should exist");
  assert.ok(historyTab, "History tab should exist");
  assert.equal(reviewTab.getAttribute("aria-selected"), "true");
  assert.equal(historyTab.getAttribute("aria-selected"), "false");

  const review = fixture.body.querySelector("[data-improvement-panel='needs-review']");
  const history = fixture.body.querySelector("[data-improvement-panel='history']");
  assert.ok(review, "Needs Review panel should exist");
  assert.ok(history, "History panel should exist");
  assert.equal(review.hidden, false, "Needs Review should be the default active panel");
  assert.equal(history.hidden, true, "History should not be mixed into the default review panel");
  assert.match(review.textContent, /Needs Review/);
  assert.doesNotMatch(review.textContent, /Verification skip regression/);
  assert.deepEqual(
    Array.from(review.querySelectorAll(".improvement-inbox-row[data-improvement-id]"), (row) =>
      row.dataset.improvementId,
    ),
    ["impr-pending"],
  );
  assert.deepEqual(
    Array.from(
      history.querySelectorAll(".improvement-inbox-row[data-improvement-id]"),
      (row) => [row.dataset.improvementId, row.dataset.improvementState],
    ),
    [
      ["impr-promoted", "promoted"],
      ["impr-dismissed", "dismissed"],
    ],
  );
  historyTab.click();

  assert.equal(reviewTab.getAttribute("aria-selected"), "false");
  assert.equal(historyTab.getAttribute("aria-selected"), "true");
  assert.equal(review.hidden, true);
  assert.equal(history.hidden, false);
  assert.match(history.textContent, /History/);
  assert.match(history.textContent, /Processed/);
  assert.match(history.textContent, /Created upstream Issue #3164/);
  assert.match(history.textContent, /Rejected with reason/);
  for (const processed of history.querySelectorAll(".improvement-inbox-row[data-improvement-id]")) {
    const actions = Array.from(processed.querySelectorAll("[data-action]"), (button) =>
      button.textContent.trim(),
    );
    assert.ok(!actions.includes("Approve"), "processed rows must not expose Approve");
    assert.ok(!actions.includes("Reject"), "processed rows must not expose Reject");
  }
});

test("Improvement Inbox CSS hides inactive tab panels despite section display rules", () => {
  const css = readFileSync(
    new URL("../styles/app.css", import.meta.url),
    "utf8",
  );
  const sectionDisplayRule = css.indexOf(".improvement-inbox-section {");
  const hiddenPanelRule = css.indexOf("[data-improvement-panel][hidden]");

  assert.ok(sectionDisplayRule >= 0, "section display rule should be present");
  assert.ok(hiddenPanelRule >= 0, "inactive Improvement Inbox panels need a hidden CSS rule");
  assert.ok(
    hiddenPanelRule > sectionDisplayRule,
    "hidden panel rule should come after .improvement-inbox-section display rule",
  );
  assert.match(
    css.slice(hiddenPanelRule, hiddenPanelRule + 160),
    /display:\s*none\s*!important/,
    "hidden panel rule must override display:grid in browsers",
  );
});

test("Improvement Inbox keeps processed candidates in history when nothing needs review", () => {
  const fixture = createFixture();
  fixture.surface.mount(fixture.body, {
    id: "improvements",
    improvement_candidates: sampleCandidates.slice(1),
  });

  const review = fixture.body.querySelector("[data-improvement-panel='needs-review']");
  const history = fixture.body.querySelector("[data-improvement-panel='history']");
  assert.match(review.textContent, /No candidates need review/);
  fixture.body.querySelector("[data-improvement-tab='history']").click();
  assert.equal(history.hidden, false);
  assert.deepEqual(
    Array.from(history.querySelectorAll(".improvement-inbox-row[data-improvement-id]"), (row) =>
      row.dataset.improvementId,
    ),
    ["impr-promoted", "impr-dismissed"],
  );
});

test("Improvement Inbox pending rows expose explicit approval controls", () => {
  const fixture = createFixture();
  fixture.surface.mount(fixture.body, {
    id: "improvements",
    improvement_candidates: sampleCandidates,
  });

  const pending = fixture.body.querySelector("[data-improvement-id='impr-pending']");
  const actions = Array.from(pending.querySelectorAll(".improvement-inbox-action"), (button) =>
    button.textContent.trim(),
  );

  assert.deepEqual(actions, ["Approve", "Reject", "Details"]);
});

test("Improvement Inbox Approve opens an in-app public Issue confirmation", () => {
  const fixture = createFixture();
  fixture.surface.mount(fixture.body, {
    id: "improvements",
    improvement_candidates: sampleCandidates,
  });

  fixture.body
    .querySelector("[data-action='approve-improvement'][data-improvement-id='impr-pending']")
    .click();

  const modal = fixture.body.querySelector("[data-improvement-modal='approve']");
  assert.ok(modal, "Approve should open an in-app confirmation modal");
  assert.match(modal.textContent, /Create public Issue/);
  assert.match(modal.textContent, /akiojin\/gwt/);
  assert.match(modal.textContent, /Issue Preview/);
  assert.match(modal.textContent, /fix\(gwt\): Skill failed to update/);
  assert.match(modal.textContent, /## Problem/);
  assert.match(modal.textContent, /## Expected behavior/);
  assert.match(modal.textContent, /## Observed evidence/);
  assert.match(modal.textContent, /## Impact/);
  assert.match(modal.textContent, /## Suggested verification/);
  assert.match(modal.textContent, /## Source candidate/);
  assert.match(modal.textContent, /Public body generated from sanitized candidate fields only/);
  assert.match(modal.textContent, /Skill failed to update after agent failure/);
  assert.deepEqual(fixture.sent, []);

  modal.querySelector("[data-action='confirm-approve-improvement']").click();

  assert.deepEqual(fixture.sent, [
    { kind: "improvement_promote_issue", id: "impr-pending" },
  ]);
});

test("Improvement Inbox Approve cancel does not send a public Issue action", () => {
  const fixture = createFixture();
  fixture.surface.mount(fixture.body, {
    id: "improvements",
    improvement_candidates: sampleCandidates,
  });

  fixture.body
    .querySelector("[data-action='approve-improvement'][data-improvement-id='impr-pending']")
    .click();
  fixture.body.querySelector("[data-action='cancel-improvement-modal']").click();

  assert.deepEqual(fixture.sent, []);
});

test("Improvement Inbox Reject sends an optional dismiss reason", () => {
  const fixture = createFixture();
  fixture.surface.mount(fixture.body, {
    id: "improvements",
    improvement_candidates: sampleCandidates,
  });

  fixture.body
    .querySelector("[data-action='reject-improvement'][data-improvement-id='impr-pending']")
    .click();
  const modal = fixture.body.querySelector("[data-improvement-modal='reject']");
  assert.ok(modal, "Reject should open a reason modal");
  assert.match(modal.textContent, /Reject candidate/);
  const textarea = modal.querySelector("[data-improvement-reject-reason]");
  textarea.value = "Already covered by SPEC #3164";
  modal.querySelector("[data-action='confirm-reject-improvement']").click();

  assert.deepEqual(fixture.sent, [
    {
      kind: "improvement_dismiss",
      id: "impr-pending",
      reason: "Already covered by SPEC #3164",
    },
  ]);

  const review = fixture.body.querySelector("[data-improvement-panel='needs-review']");
  assert.ok(
    !review.querySelector("[data-improvement-id='impr-pending']"),
    "rejected candidate should leave Needs Review immediately",
  );
  assert.match(review.textContent, /No candidates need review/);

  fixture.body.querySelector("[data-improvement-tab='history']").click();
  const rejected = fixture.body.querySelector(
    "[data-improvement-panel='history'] [data-improvement-id='impr-pending']",
  );
  assert.ok(rejected, "rejected candidate should be visible in History immediately");
  assert.equal(rejected.dataset.improvementState, "dismissed");
  assert.match(rejected.textContent, /Rejected/);
  assert.match(rejected.textContent, /Already covered by SPEC #3164/);
  const actions = Array.from(rejected.querySelectorAll("[data-action]"), (button) =>
    button.textContent.trim(),
  );
  assert.deepEqual(actions, ["Details"]);
});

test("Improvement Inbox Details modal shows the public Issue preview and approval choices", () => {
  const fixture = createFixture();
  fixture.surface.mount(fixture.body, {
    id: "improvements",
    improvement_candidates: sampleCandidates,
  });

  fixture.body
    .querySelector("[data-action='details-improvement'][data-improvement-id='impr-pending']")
    .click();

  const modal = fixture.body.querySelector("[data-improvement-modal='details']");
  assert.ok(modal, "Details should open a candidate modal");
  assert.match(modal.textContent, /Skill failed to update after agent failure/);
  assert.match(modal.textContent, /Issue Preview/);
  assert.match(modal.textContent, /Repository: akiojin\/gwt/);
  assert.match(modal.textContent, /Title: fix\(gwt\): Skill failed to update/);
  assert.match(modal.textContent, /## Problem/);
  assert.match(modal.textContent, /## Expected behavior/);
  assert.match(modal.textContent, /## Observed evidence/);
  assert.match(modal.textContent, /## Impact/);
  assert.match(modal.textContent, /## Suggested verification/);
  assert.match(modal.textContent, /## Source candidate/);
  assert.match(modal.textContent, /## Privacy/);
  assert.ok(modal.querySelector("[data-action='approve-improvement']"));
  assert.ok(modal.querySelector("[data-action='reject-improvement']"));
});

test("Improvement Inbox processed Details explain history without approval actions", () => {
  const fixture = createFixture();
  fixture.surface.mount(fixture.body, {
    id: "improvements",
    improvement_candidates: sampleCandidates,
  });

  fixture.body
    .querySelector(
      "[data-action='details-improvement'][data-improvement-id='impr-promoted']",
    )
    .click();

  const modal = fixture.body.querySelector("[data-improvement-modal='details']");
  assert.ok(modal, "Processed Details should open a candidate modal");
  assert.match(modal.textContent, /State: Approved/);
  assert.match(modal.textContent, /Linked Issue: #3164/);
  assert.match(modal.textContent, /Issue preview is unavailable for this candidate/);
  assert.doesNotMatch(modal.textContent, /before approval/);
  assert.equal(modal.querySelector("[data-action='approve-improvement']"), null);
  assert.equal(modal.querySelector("[data-action='reject-improvement']"), null);
});

test("Improvement Inbox open linked Issue action remains available", () => {
  const fixture = createFixture();
  fixture.surface.mount(fixture.body, {
    id: "improvements",
    improvement_candidates: sampleCandidates,
  });

  fixture.body
    .querySelector("[data-action='open-improvement-issue'][data-issue-number='3164']")
    .click();

  assert.deepEqual(fixture.sent, [
    { kind: "open_server_url", url: "https://github.com/akiojin/gwt/issues/3164" },
  ]);
});
