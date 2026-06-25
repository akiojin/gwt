import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));

test("Usage hover shows account label while status strip stays compact", async () => {
  const { createProviderUsageSurface } = await import(
    resolve(here, "../provider-usage-surface.js")
  );
  const { applyProviderUsage } = await importOperatorShell();
  const { document, window } = parseHTML(
    "<html><body><div id='op-strip-usage'></div><div id='usage-anchor'></div></body></html>",
  );
  const previousDocument = globalThis.document;
  const previousWindow = globalThis.window;
  const previousCustomEvent = globalThis.CustomEvent;
  const previousRequestAnimationFrame = globalThis.requestAnimationFrame;
  globalThis.document = document;
  globalThis.window = window;
  globalThis.CustomEvent = window.CustomEvent;
  globalThis.requestAnimationFrame = (cb) => cb();
  window.innerWidth = 1200;
  window.innerHeight = 800;

  try {
    const snapshot = {
      accounts: [
        {
          provider: "codex",
          account_label: "codex@example.com",
          plan: "pro",
          windows: [{ kind: "weekly", used_percent: 12, resets_at: null }],
          state: { kind: "ok" },
        },
      ],
      consumption: [],
      sessions: [],
    };
    const surface = createProviderUsageSurface({
      send: () => {},
      renderWorkspaceWindows: () => {},
    });
    surface.applyProviderUsageUi(snapshot);
    applyProviderUsage(document, snapshot);

    const strip = document.getElementById("op-strip-usage");
    assert.match(strip.textContent, /USAGE/);
    assert.doesNotMatch(strip.textContent, /codex@example\.com/);

    const anchor = document.getElementById("usage-anchor");
    anchor.getBoundingClientRect = () => ({ left: 24, top: 640 });
    window.__gwtShowUsageHover(anchor);

    assert.match(document.body.textContent, /Account:\s*codex@example\.com/);
  } finally {
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
    globalThis.CustomEvent = previousCustomEvent;
    globalThis.requestAnimationFrame = previousRequestAnimationFrame;
  }
});

async function importOperatorShell() {
  const modulePath = resolve(here, "../operator-shell.js");
  const source = readFileSync(modulePath, "utf8")
    .replace('from "/theme-manager.js"', `from "${pathToFileURL(resolve(here, "../theme-manager.js")).href}"`)
    .replace('from "/hotkey.js"', `from "${pathToFileURL(resolve(here, "../hotkey.js")).href}"`)
    .replace('from "/theme-toggle.js"', `from "${pathToFileURL(resolve(here, "../theme-toggle.js")).href}"`);
  const tmpDir = resolve(here, "../../../../.tmp-tests");
  mkdirSync(tmpDir, { recursive: true });
  const tmpModule = resolve(tmpDir, "provider-usage-operator-shell-import.mjs");
  writeFileSync(tmpModule, source);
  return import(pathToFileURL(tmpModule).href);
}
