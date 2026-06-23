import assert from "node:assert/strict";
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

test("Improvement Inbox actions emit promote dismiss and open messages", () => {
  const fixture = createFixture();
  fixture.surface.mount(fixture.body, {
    id: "improvements",
    improvement_candidates: sampleCandidates,
  });

  fixture.body
    .querySelector("[data-action='promote-improvement'][data-improvement-id='impr-pending']")
    .click();
  fixture.body
    .querySelector("[data-action='dismiss-improvement'][data-improvement-id='impr-pending']")
    .click();
  fixture.body
    .querySelector("[data-action='open-improvement-issue'][data-issue-number='3164']")
    .click();

  assert.deepEqual(fixture.sent, [
    { kind: "improvement_promote_issue", id: "impr-pending" },
    { kind: "improvement_dismiss", id: "impr-pending" },
    { kind: "open_server_url", url: "https://github.com/akiojin/gwt/issues/3164" },
  ]);
});
