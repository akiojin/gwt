// SPEC-2359 W-17 (FR-398) — the Resume picker must not close optimistically.
//
// Before this phase, pick() sent `resume_workspace_agent` and immediately
// closed the modal, leaving the user staring at a blank canvas with no
// indication anything was happening (and no error surface if the socket was
// down). The picker now stays open in a pending state until the backend acks
// (`workspace_resume_agent_started`) or errors.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { createWorkspaceResumePickerController } from "../workspace-resume-picker-modal.js";
import { createLaunchPendingController } from "../launch-pending-controller.js";

function createFixture() {
  const { document } = parseHTML(`
    <div id="modal"><div id="dialog"></div></div>
  `);
  const modalEl = document.getElementById("modal");
  const dialogEl = document.getElementById("dialog");
  const createNode = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  return { document, modalEl, dialogEl, createNode };
}

function createPicker(fixture, { sent = [], launchPending } = {}) {
  return createWorkspaceResumePickerController({
    modalEl: fixture.modalEl,
    dialogEl: fixture.dialogEl,
    createNode: fixture.createNode,
    send: (message) => sent.push(message),
    getResumeBounds: () => ({ x: 0, y: 0, width: 800, height: 600 }),
    launchPending,
  });
}

const sampleAgent = {
  session_id: "work-1",
  agent_id: "codex",
  display_name: "Codex",
  branch: "feature/x",
};

test("native picker entries are labelled separately from fresh starts", () => {
  const fixture = createFixture();
  const picker = createPicker(fixture, {});

  picker.open("workspace-1");
  picker.handleAgentsList({
    agents: [{ ...sampleAgent, resume_kind: "native_picker" }],
  });

  const tag = fixture.dialogEl.querySelector(".workspace-resume-picker-row-tag");
  assert.equal(tag?.textContent, "Resume picker");
});

test("pick keeps the modal open in a pending state instead of closing", () => {
  const fixture = createFixture();
  const sent = [];
  const picker = createPicker(fixture, { sent });

  picker.open("workspace-1");
  picker.handleAgentsList({ agents: [sampleAgent] });

  const row = fixture.dialogEl.querySelector(".workspace-resume-picker-row");
  assert.ok(row, "agent row rendered");
  row.click();

  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, "resume_workspace_agent");
  assert.equal(
    fixture.modalEl.classList.contains("open"),
    true,
    "modal must stay open while the resume is in flight",
  );
  const pendingRow = fixture.dialogEl.querySelector(
    ".workspace-resume-picker-row",
  );
  assert.equal(
    pendingRow.disabled,
    true,
    "rows are disabled while the resume is in flight",
  );
  assert.match(
    fixture.dialogEl.textContent,
    /Resuming/,
    "pending state is visible to the user",
  );
});

test("a second click while pending does not send a duplicate request", () => {
  const fixture = createFixture();
  const sent = [];
  const picker = createPicker(fixture, { sent });

  picker.open("workspace-1");
  picker.handleAgentsList({ agents: [sampleAgent] });
  fixture.dialogEl.querySelector(".workspace-resume-picker-row").click();
  const row = fixture.dialogEl.querySelector(".workspace-resume-picker-row");
  row.click();

  assert.equal(sent.length, 1, "pending pick must not re-send");
});

test("handleStarted closes the modal once the backend acks", () => {
  const fixture = createFixture();
  const picker = createPicker(fixture, {});

  picker.open("workspace-1");
  picker.handleAgentsList({ agents: [sampleAgent] });
  fixture.dialogEl.querySelector(".workspace-resume-picker-row").click();

  picker.handleStarted({ session_id: "work-1" });

  assert.equal(fixture.modalEl.classList.contains("open"), false);
});

test("handleError clears pending and shows the reason in place", () => {
  const fixture = createFixture();
  const picker = createPicker(fixture, {});

  picker.open("workspace-1");
  picker.handleAgentsList({ agents: [sampleAgent] });
  fixture.dialogEl.querySelector(".workspace-resume-picker-row").click();

  picker.handleError({ session_id: "work-1", message: "Worktree missing" });

  assert.equal(fixture.modalEl.classList.contains("open"), true);
  assert.match(fixture.dialogEl.textContent, /Worktree missing/);
  const row = fixture.dialogEl.querySelector(".workspace-resume-picker-row");
  assert.equal(row.disabled, false, "rows re-enable after an error");
});

test("pick consults the shared launch-pending controller as a global guard", () => {
  const fixture = createFixture();
  const sent = [];
  const launchPending = createLaunchPendingController({
    setTimeoutFn: () => 1,
    clearTimeoutFn: () => {},
  });
  const picker = createPicker(fixture, { sent, launchPending });

  // Another surface already started this Work's resume.
  launchPending.begin("session:work-1", "Resume");

  picker.open("workspace-1");
  picker.handleAgentsList({ agents: [sampleAgent] });
  fixture.dialogEl.querySelector(".workspace-resume-picker-row").click();

  assert.equal(
    sent.length,
    0,
    "picker must not double-send a Work that is already resuming elsewhere",
  );
});
