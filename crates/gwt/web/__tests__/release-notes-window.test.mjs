// SPEC #2780 — release-notes-window.js DOM unit tests.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { createReleaseNotesWindow } from "../release-notes-window.js";

function makeFixture({ entries = sampleEntries(), focusVersion = null } = {}) {
  const { document } = parseHTML(
    '<!doctype html><html><body><button id="trigger">Version</button></body></html>',
  );
  const sent = [];
  const send = (msg) => sent.push(msg);
  const controller = createReleaseNotesWindow({
    document,
    send,
    generateId: () => "rn-test-1",
  });
  return { document, controller, sent, entries, focusVersion };
}

function sampleEntries() {
  return [
    {
      version: "9.38.0",
      date: "2026-05-19",
      sections: [
        {
          heading: "Bug Fixes",
          items: [
            "**gui:** Keep tao::Window alive for the event_loop lifetime",
            "**installer:** Pin gwt.exe Name",
          ],
        },
        {
          heading: "Features",
          items: ["**serve:** Open default browser unless --no-open is passed"],
        },
      ],
    },
    {
      version: "9.37.0",
      date: "2026-05-19",
      sections: [
        { heading: "Bug Fixes", items: ["Stabilize terminal resize"] },
      ],
    },
    {
      version: "9.36.0",
      date: "2026-05-18",
      sections: [
        {
          heading: "Features",
          items: ["**file-tree:** Add file content read domain"],
        },
      ],
    },
  ];
}

test("open() sends open_release_notes with id and focus_version", () => {
  const { controller, sent } = makeFixture();
  controller.open("9.37.0");
  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, "open_release_notes");
  assert.equal(sent[0].id, "rn-test-1");
  assert.equal(sent[0].focus_version, "9.37.0");
});

test("open() with null focus_version sends null, not undefined", () => {
  const { controller, sent } = makeFixture();
  controller.open(null);
  assert.equal(sent[0].focus_version, null);
});

test("handlePayload mounts the window and renders sidebar entries", () => {
  const { document, controller, entries } = makeFixture();
  controller.handlePayload({ id: "rn-test-1", entries, focus_version: null });

  const root = document.getElementById("release-notes-window");
  assert.ok(root, "window root must be present in the DOM");
  assert.equal(root.getAttribute("role"), "dialog");

  const sidebarItems = root.querySelectorAll(".release-notes-sidebar-item");
  assert.equal(sidebarItems.length, entries.length);
  assert.equal(sidebarItems[0].dataset.version, "9.38.0");
  assert.equal(sidebarItems[1].dataset.version, "9.37.0");
});

test("handlePayload mounts an app-global floating window chrome", () => {
  const { document, controller, entries } = makeFixture();
  controller.handlePayload({ id: "rn-test-1", entries, focus_version: null });

  const root = document.getElementById("release-notes-window");
  assert.ok(root.classList.contains("op-global-window"));
  assert.ok(root.classList.contains("release-notes-window"));
  assert.equal(root.getAttribute("data-surface"), "release-notes");
  assert.ok(root.querySelector(".op-global-window__titlebar"));
  assert.ok(root.querySelector(".op-global-window__title"));
  assert.ok(root.querySelector(".op-global-window__actions"));
  assert.ok(root.querySelector(".op-global-window__body"));
});

test("handlePayload defaults selection to the first entry when no focus", () => {
  const { document, controller, entries } = makeFixture();
  controller.handlePayload({ id: "rn-test-1", entries, focus_version: null });
  assert.equal(controller.selectedVersion(), "9.38.0");
  const selected = document.querySelector(
    ".release-notes-sidebar-item.is-selected",
  );
  assert.ok(selected);
  assert.equal(selected.dataset.version, "9.38.0");
});

test("handlePayload honours backend focus_version", () => {
  const { controller, entries } = makeFixture();
  controller.handlePayload({
    id: "rn-test-1",
    entries,
    focus_version: "9.37.0",
  });
  assert.equal(controller.selectedVersion(), "9.37.0");
});

test("handlePayload renders heading and **bold** as DOM, not literal markup", () => {
  const { document, controller, entries } = makeFixture();
  controller.handlePayload({ id: "rn-test-1", entries, focus_version: null });

  const content = document.querySelector(".release-notes-content");
  const headings = content.querySelectorAll("h3");
  const headingTexts = Array.from(headings).map((h) => h.textContent);
  assert.deepEqual(headingTexts, ["Bug Fixes", "Features"]);

  const firstLi = content.querySelector("li");
  // No literal '**' should leak through to text.
  assert.ok(
    !firstLi.textContent.includes("**"),
    `bold markers must be consumed, got: ${firstLi.textContent}`,
  );
  const strong = firstLi.querySelector("strong");
  assert.ok(strong, "first item should contain a <strong> element");
  assert.equal(strong.textContent, "gui:");
});

test("does not interpret HTML-like text inside items", () => {
  const { document, controller } = makeFixture();
  const entries = [
    {
      version: "1.0.0",
      date: "2025-01-01",
      sections: [
        {
          heading: "Notes",
          items: ["payload <script>alert(1)</script> end"],
        },
      ],
    },
  ];
  controller.handlePayload({ id: "rn-test-1", entries, focus_version: null });
  const content = document.querySelector(".release-notes-content");
  const li = content.querySelector("li");
  assert.equal(li.querySelectorAll("script").length, 0);
  assert.ok(li.textContent.includes("<script>"));
});

test("sidebar click selects a different version", () => {
  const { document, controller, entries } = makeFixture();
  controller.handlePayload({ id: "rn-test-1", entries, focus_version: null });
  const second = document.querySelectorAll(
    ".release-notes-sidebar-item",
  )[1];
  second.click();
  assert.equal(controller.selectedVersion(), "9.37.0");
  const content = document.querySelector(".release-notes-content h2");
  assert.equal(content.textContent, "v9.37.0");
});

test("close() removes the window from the DOM and isOpen reflects that", () => {
  const { document, controller, entries } = makeFixture();
  controller.handlePayload({ id: "rn-test-1", entries, focus_version: null });
  assert.equal(controller.isOpen(), true);
  controller.close();
  assert.equal(controller.isOpen(), false);
  assert.equal(document.getElementById("release-notes-window"), null);
});

test("Escape closes Release Notes and restores focus to the trigger", () => {
  const { document, controller, entries } = makeFixture();
  const trigger = document.getElementById("trigger");
  let activeElement = trigger;
  Object.defineProperty(document, "activeElement", {
    configurable: true,
    get: () => activeElement,
  });
  trigger.focus = () => {
    activeElement = trigger;
  };
  trigger.focus();
  controller.open("9.38.0");
  controller.handlePayload({ id: "rn-test-1", entries, focus_version: null });
  assert.equal(controller.isOpen(), true);

  const event = new document.defaultView.Event("keydown", { bubbles: true });
  Object.defineProperty(event, "key", { value: "Escape" });
  document.dispatchEvent(event);

  assert.equal(controller.isOpen(), false);
  assert.equal(document.activeElement, trigger);
});

test("Release Notes open request is app-global and does not require project state", () => {
  const { controller, sent } = makeFixture();
  controller.open(null);

  assert.deepEqual(Object.keys(sent[0]).sort(), [
    "focus_version",
    "id",
    "kind",
  ]);
  assert.equal(sent[0].kind, "open_release_notes");
});

test("handleError shows the empty state with the CHANGELOG URL", () => {
  const { document, controller } = makeFixture();
  controller.handleError("Release notes could not be loaded.");
  const empty = document.querySelector(".release-notes-empty");
  assert.ok(empty);
  const link = empty.querySelector("a");
  assert.ok(link);
  assert.ok(link.href.includes("akiojin/gwt"));
  assert.ok(link.href.includes("CHANGELOG.md"));
});

test("handlePayload with empty entries falls back to the empty state", () => {
  const { document, controller } = makeFixture();
  controller.handlePayload({ id: "rn-test-1", entries: [], focus_version: null });
  const empty = document.querySelector(".release-notes-empty");
  assert.ok(empty);
});
