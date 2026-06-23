// SPEC-2359 US-83 — Workspace "Open a branch…" picker modal tests.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { createWorkspaceBranchPickerController } from "../workspace-branch-picker-modal.js";

function createFixture() {
  const { document } = parseHTML(`<div id="modal"><div id="dialog"></div></div>`);
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

function createPicker(fixture, sent) {
  return createWorkspaceBranchPickerController({
    modalEl: fixture.modalEl,
    dialogEl: fixture.dialogEl,
    createNode: fixture.createNode,
    send: (message) => sent.push(message),
  });
}

test("branch rows show the bare name and a remote hint", () => {
  const fixture = createFixture();
  const picker = createPicker(fixture, []);
  picker.open("tab-1::work-1");
  picker.handleBranchList({ id: "tab-1::work-1", branches: ["origin/feature-foo"] });

  const name = fixture.dialogEl.querySelector(".workspace-branch-picker-row-name");
  assert.equal(name?.textContent, "feature-foo", "origin/ prefix is stripped for display");
  const tag = fixture.dialogEl.querySelector(".workspace-branch-picker-row-tag");
  assert.equal(tag?.textContent, "remote");
});

test("picking a branch sends open_launch_wizard with the raw ref and closes", () => {
  const fixture = createFixture();
  const sent = [];
  const picker = createPicker(fixture, sent);
  picker.open("tab-1::work-1");
  picker.handleBranchList({ id: "tab-1::work-1", branches: ["origin/feature-foo"] });

  const row = fixture.dialogEl.querySelector(".workspace-branch-picker-row");
  assert.ok(row, "branch row rendered");
  row.click();

  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, "open_launch_wizard");
  assert.equal(sent[0].id, "tab-1::work-1");
  assert.equal(
    sent[0].branch_name,
    "origin/feature-foo",
    "the raw ref is sent; the backend normalizes origin/ for the launch",
  );
  assert.equal(picker.isOpen(), false, "picker closes after handing off to the wizard");
});

test("an empty branch list shows the empty state", () => {
  const fixture = createFixture();
  const picker = createPicker(fixture, []);
  picker.open("tab-1::work-1");
  picker.handleBranchList({ id: "tab-1::work-1", branches: [] });

  const empty = fixture.dialogEl.querySelector(".workspace-branch-picker-empty");
  assert.match(empty?.textContent ?? "", /No remote branches/);
});

test("a response for a different window is ignored", () => {
  const fixture = createFixture();
  const picker = createPicker(fixture, []);
  picker.open("tab-1::work-1");
  picker.handleBranchList({ id: "tab-1::work-OTHER", branches: ["origin/x"] });

  const empty = fixture.dialogEl.querySelector(".workspace-branch-picker-empty");
  assert.match(empty?.textContent ?? "", /Loading/, "stays loading for the unmatched window");
});
