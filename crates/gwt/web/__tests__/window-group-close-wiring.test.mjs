import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname } from "node:path";
import { resolve } from "node:path";
import { test } from "node:test";
import assert from "node:assert/strict";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const indexSource = readFileSync(resolve(here, "../index.html"), "utf8");

test("titlebar close routes through group close confirmation", () => {
  assert.match(
    appSource,
    /let\s+closeWindowGroupModalState\s*=/,
    "expected local modal state for grouped window close",
  );
  assert.match(
    appSource,
    /function\s+requestCloseWindowGroup\(windowId\)/,
    "expected a titlebar close request helper",
  );
  assert.match(
    appSource,
    /windowTabsFor\(windowData\)[\s\S]{0,260}?tabs\.length\s*<\s*2[\s\S]{0,180}?kind:\s*"close_window_group"/,
    "single-window titlebar close should send close_window_group immediately",
  );
  assert.match(
    appSource,
    /closeWindowGroupModalState\s*=\s*\{[\s\S]{0,260}?tabs[\s\S]{0,260}?renderCloseWindowGroupModal\(\)/,
    "multi-tab titlebar close should open a confirmation modal",
  );
});

test("confirming grouped titlebar close sends close_window_group", () => {
  assert.match(
    indexSource,
    /id="close-window-group-modal"/,
    "index.html must provide the grouped window close modal host",
  );
  assert.match(
    appSource,
    /onConfirm:[\s\S]{0,500}?send\(\{\s*kind:\s*"close_window_group",\s*id:\s*targetId\s*\}\)/,
    "confirm action must send close_window_group for the selected group",
  );
  assert.match(
    appSource,
    /Close window/,
    "confirmation copy should name the destructive window close action",
  );
});

test("tab strip close remains a single-tab close", () => {
  assert.match(
    appSource,
    /closeButton\.addEventListener\("click"[\s\S]{0,220}?send\(\{\s*kind:\s*"close_window",\s*id:\s*tab\.id\s*\}\)/,
    "tab-strip close buttons must keep using close_window",
  );
  assert.match(
    appSource,
    /closeButton\.addEventListener\("click"[\s\S]{0,220}?event\.stopPropagation\(\)/,
    "tab-strip close must not activate the tab while closing it",
  );
});
