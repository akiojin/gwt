import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";

import { createTerminalContextMenuController } from "../terminal-context-menu.js";

test("terminal context menu pastes clipboard text through xterm paste", async () => {
  const fixture = mountFixture({
    readClipboardText: async () => "printf 'hello'\n",
  });
  let bubbledContextMenuCount = 0;
  fixture.document.body.addEventListener("contextmenu", () => {
    bubbledContextMenuCount += 1;
  });

  const event = fixture.openContextMenu({ x: 24, y: 36 });
  const menu = fixture.menu();
  assert.equal(event.defaultPrevented, true);
  assert.equal(bubbledContextMenuCount, 0);
  assert.equal(menu.hidden, false);
  assert.equal(menu.style.left, "24px");
  assert.equal(menu.style.top, "36px");
  assert.equal(fixture.pasteButton().textContent, "Paste");

  fixture.clickPaste();
  await flushPromises();

  assert.deepEqual(fixture.pastedText, ["printf 'hello'\n"]);
  assert.deepEqual(fixture.pastedImages, []);
  assert.equal(menu.hidden, true);
  assert.equal(fixture.focusCount, 1);
  fixture.dispose();
});

test("terminal context menu prefers clipboard images before text fallback", async () => {
  let readTextCount = 0;
  const fixture = mountFixture({
    readClipboardItems: async () => [
      {
        types: ["image/png"],
        getType: async (type) => ({ type }),
      },
    ],
    readClipboardText: async () => {
      readTextCount += 1;
      return "plain text";
    },
  });

  fixture.openContextMenu();
  fixture.clickPaste();
  await flushPromises();

  assert.equal(readTextCount, 0);
  assert.deepEqual(fixture.pastedText, []);
  assert.deepEqual(JSON.parse(JSON.stringify(fixture.pastedImages)), [
    {
      blob: {
        type: "image/png",
      },
      mimeType: "image/png",
      filename: null,
    },
  ]);
  assert.equal(fixture.menu().hidden, true);
  assert.equal(fixture.focusCount, 1);
  fixture.dispose();
});

test("terminal context menu falls back to text when clipboard item read fails", async () => {
  const fixture = mountFixture({
    readClipboardItems: async () => {
      throw new Error("clipboard item read unavailable");
    },
    readClipboardText: async () => "fallback text",
  });

  fixture.openContextMenu();
  fixture.clickPaste();
  await flushPromises();

  assert.deepEqual(fixture.pastedText, ["fallback text"]);
  assert.deepEqual(fixture.pastedImages, []);
  assert.equal(fixture.menu().hidden, true);
  assert.equal(fixture.focusCount, 1);
  fixture.dispose();
});

test("terminal context menu dismisses without paste on outside input and Escape", () => {
  const fixture = mountFixture({
    readClipboardText: async () => "should not paste",
  });

  fixture.openContextMenu();
  assert.equal(fixture.menu().hidden, false);
  fixture.document.body.dispatchEvent(
    pointerEvent(fixture.window, "pointerdown", { target: fixture.document.body }),
  );
  assert.equal(fixture.menu().hidden, true);

  fixture.openContextMenu();
  fixture.document.dispatchEvent(keyEvent(fixture.window, "Escape"));
  assert.equal(fixture.menu().hidden, true);

  fixture.openContextMenu();
  fixture.window.dispatchEvent(new fixture.window.Event("blur"));
  assert.equal(fixture.menu().hidden, true);

  assert.deepEqual(fixture.pastedText, []);
  assert.deepEqual(fixture.pastedImages, []);
  fixture.dispose();
});

function mountFixture(overrides = {}) {
  const { document, window } = parseHTML(
    "<!doctype html><html><body><main id=\"canvas\"><div id=\"terminal\"></div><div id=\"panel\"></div></main></body></html>",
  );
  const terminalRoot = document.getElementById("terminal");
  const pastedText = [];
  const pastedImages = [];
  let focusCount = 0;

  const controller = createTerminalContextMenuController({
    document,
    window,
    terminalRoot,
    readClipboardText: async () => "",
    readClipboardItems: async () => [],
    pasteText: (text) => pastedText.push(text),
    pasteImage: (payload) => pastedImages.push(payload),
    focusTerminal: () => {
      focusCount += 1;
    },
    ...overrides,
  });

  return {
    document,
    window,
    terminalRoot,
    pastedText,
    pastedImages,
    get focusCount() {
      return focusCount;
    },
    menu: () => document.querySelector(".terminal-context-menu"),
    pasteButton: () => document.querySelector(".terminal-context-menu__item"),
    openContextMenu({ x = 10, y = 12 } = {}) {
      const event = contextMenuEvent(window, x, y);
      terminalRoot.dispatchEvent(event);
      return event;
    },
    clickPaste() {
      this.pasteButton().dispatchEvent(
        new window.Event("click", { bubbles: true, cancelable: true }),
      );
    },
    dispose() {
      controller.dispose();
    },
  };
}

function contextMenuEvent(window, x, y) {
  const event = new window.Event("contextmenu", {
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(event, "clientX", { value: x });
  Object.defineProperty(event, "clientY", { value: y });
  return event;
}

function pointerEvent(window, type, { target }) {
  const event = new window.Event(type, {
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(event, "target", { value: target });
  return event;
}

function keyEvent(window, key) {
  const event = new window.Event("keydown", {
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(event, "key", { value: key });
  return event;
}

async function flushPromises() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}
