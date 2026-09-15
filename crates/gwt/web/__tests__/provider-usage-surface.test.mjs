import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");

test("Usage severity styles use Operator tokens without raw color fallbacks", () => {
  assert.match(
    componentsCss,
    /\.op-status-strip__cell--usage\s*\{[^}]*flex:\s*0 0 23ch;[^}]*inline-size:\s*23ch;[^}]*white-space:\s*nowrap;[^}]*overflow:\s*hidden;/s,
  );
  assert.match(
    componentsCss,
    /\.op-usage-sum\s*\{[^}]*inline-size:\s*7ch;/s,
  );
  assert.match(
    componentsCss,
    /\.op-usage-more\s*\{[^}]*color:\s*var\(--color-status-strip-fg\);/s,
  );
  assert.match(
    componentsCss,
    /\.op-usage-sum\[data-severity="normal"\]\s*\{[^}]*color:\s*var\(--color-status-strip-fg\);/s,
  );
  assert.match(
    componentsCss,
    /\.op-usage-sum\[data-severity="warning"\]\s*\{[^}]*color:\s*var\(--color-state-needs-input\);/s,
  );
  assert.match(
    componentsCss,
    /\.op-usage-sum\[data-severity="danger"\]\s*\{[^}]*color:\s*var\(--color-state-blocked\);/s,
  );
  assert.match(
    componentsCss,
    /\.op-usage-bar__fill\[data-severity="warning"\]\s*\{[^}]*background:\s*var\(--color-state-needs-input\);/s,
  );
  assert.match(
    componentsCss,
    /\.op-usage-bar__fill\[data-severity="danger"\]\s*\{[^}]*background:\s*var\(--color-state-blocked\);/s,
  );
});

test("Status strip labels each provider and removes symbol-only usage shorthand", async () => {
  const { applyProviderUsage } = await importOperatorShell();
  const { document } = parseHTML(
    "<html><body><button id='op-strip-usage'></button></body></html>",
  );
  const snapshot = {
    accounts: [
      {
        provider: "codex",
        windows: [{ kind: "weekly", used_percent: 10 }],
        state: { kind: "ok" },
      },
      {
        provider: "claude_code",
        windows: [{ kind: "weekly", used_percent: 29 }],
        state: { kind: "ok" },
      },
    ],
  };

  applyProviderUsage(document, snapshot);

  const strip = document.getElementById("op-strip-usage");
  assert.equal(
    strip.querySelector('[data-provider="codex"]').textContent.trim(),
    "CX 10%",
  );
  assert.equal(
    strip.querySelector('[data-provider="claude_code"]').textContent.trim(),
    "CC 29%",
  );
  assert.doesNotMatch(strip.textContent, /[⬡◇]/);
});

test("Status strip summarizes the provider window nearest its limit", async () => {
  const { applyProviderUsage } = await importOperatorShell();
  const { document } = parseHTML(
    "<html><body><button id='op-strip-usage'></button></body></html>",
  );

  applyProviderUsage(document, {
    accounts: [
      {
        provider: "codex",
        windows: [
          { kind: "weekly", used_percent: 29 },
          { kind: "five_hour", used_percent: 96 },
        ],
        state: { kind: "ok" },
      },
    ],
  });

  assert.equal(
    document.querySelector('[data-provider="codex"]').textContent.trim(),
    "CX 96%",
  );
});

test("Status strip excludes missing usage values from its summary", async () => {
  const { applyProviderUsage } = await importOperatorShell();
  const { document } = parseHTML(
    "<html><body><button id='op-strip-usage'></button></body></html>",
  );

  applyProviderUsage(document, {
    accounts: [
      {
        provider: "codex",
        windows: [
          { kind: "weekly", used_percent: null },
          { kind: "five_hour", used_percent: "" },
          { kind: "opus_weekly", used_percent: "unknown" },
        ],
        state: { kind: "ok" },
      },
    ],
  });

  const summary = document.querySelector('[data-provider="codex"]');
  assert.equal(summary.textContent.trim(), "CX —");
  assert.equal(summary.dataset.severity, undefined);
});

test("Status strip classifies usage at the 80 and 95 percent boundaries", async () => {
  const { applyProviderUsage } = await importOperatorShell();
  const { document } = parseHTML(
    "<html><body><button id='op-strip-usage'></button></body></html>",
  );
  const cases = [
    { percent: 79, expected: "normal" },
    { percent: 80, expected: "warning" },
    { percent: 94, expected: "warning" },
    { percent: 95, expected: "danger" },
  ];

  for (const { percent, expected } of cases) {
    applyProviderUsage(document, {
      accounts: [
        {
          provider: "codex",
          windows: [{ kind: "weekly", used_percent: percent }],
          state: { kind: "ok" },
        },
      ],
    });
    assert.equal(
      document.querySelector('[data-provider="codex"]').dataset.severity,
      expected,
      `${percent}% should be ${expected}`,
    );
  }

  applyProviderUsage(document, {
    accounts: [
      {
        provider: "codex",
        windows: [{ kind: "weekly", used_percent: 1 }],
        limit_reached: true,
        state: { kind: "ok" },
      },
    ],
  });
  assert.equal(
    document.querySelector('[data-provider="codex"]').dataset.severity,
    "danger",
  );
  assert.equal(
    document.getElementById("op-strip-usage").getAttribute("aria-label"),
    "Provider usage: Codex 1% limit reached",
  );
});

test("Usage popover bars use the same 80 and 95 percent severity boundaries", async () => {
  const { createProviderUsageSurface } = await import(
    resolve(here, "../provider-usage-surface.js")
  );
  const { document, window } = parseHTML(
    "<html><body><div id='usage-anchor'></div></body></html>",
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
    const surface = createProviderUsageSurface({
      send: () => {},
      renderWorkspaceWindows: () => {},
    });
    surface.applyProviderUsageUi({
      accounts: [
        {
          provider: "codex",
          windows: [
            { kind: "five_hour", used_percent: 79 },
            { kind: "five_hour_rounded", used_percent: 79.6 },
            { kind: "weekly", used_percent: 80 },
            { kind: "opus_weekly", used_percent: 94 },
            { kind: "opus_weekly_rounded", used_percent: 94.6 },
            { kind: "sonnet_weekly", used_percent: 95 },
          ],
          state: { kind: "ok" },
        },
      ],
      consumption: [],
      sessions: [],
    });
    const anchor = document.getElementById("usage-anchor");
    anchor.getBoundingClientRect = () => ({ left: 24, top: 640 });
    window.__gwtShowUsageHover(anchor);

    assert.deepEqual(
      [...document.querySelectorAll(".op-usage-bar__fill")].map(
        (fill) => fill.dataset.severity,
      ),
      ["normal", "normal", "warning", "warning", "warning", "danger"],
    );

    surface.applyProviderUsageUi({
      accounts: [
        {
          provider: "codex",
          windows: [{ kind: "weekly", used_percent: 1 }],
          limit_reached: true,
          state: { kind: "ok" },
        },
      ],
      consumption: [],
      sessions: [],
    });
    assert.equal(
      document.querySelector(".op-usage-bar__fill").dataset.severity,
      "danger",
    );
    assert.match(document.body.textContent, /Limit reached/);
  } finally {
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
    globalThis.CustomEvent = previousCustomEvent;
    globalThis.requestAnimationFrame = previousRequestAnimationFrame;
  }
});

test("Empty usage snapshot closes the popover and clears trigger state", async () => {
  const { createProviderUsageSurface } = await import(
    resolve(here, "../provider-usage-surface.js")
  );
  const { applyProviderUsage } = await importOperatorShell();
  const { document, window } = parseHTML(
    "<html><body><button id='op-strip-usage'></button></body></html>",
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
    const surface = createProviderUsageSurface({
      send: () => {},
      renderWorkspaceWindows: () => {},
    });
    window.__operatorShell = {
      applyProviderUsage: (snapshot) => applyProviderUsage(document, snapshot),
    };
    const populated = {
      accounts: [
        {
          provider: "codex",
          windows: [{ kind: "weekly", used_percent: 100 }],
          limit_reached: true,
          state: { kind: "ok" },
        },
      ],
      consumption: [],
      sessions: [],
    };
    surface.applyProviderUsageUi(populated);
    const strip = document.getElementById("op-strip-usage");
    strip.getBoundingClientRect = () => ({ left: 24, top: 640 });
    window.__gwtShowUsageHover(strip);
    assert.equal(strip.getAttribute("aria-expanded"), "true");

    surface.applyProviderUsageUi({ accounts: [], consumption: [], sessions: [] });

    assert.equal(strip.hidden, true);
    assert.equal(strip.getAttribute("aria-expanded"), "false");
    assert.equal(strip.getAttribute("aria-label"), "Provider usage and limits");
    assert.equal(strip.title, "Provider usage & limits");
    assert.equal(strip.dataset.limit, undefined);
    assert.equal(document.getElementById("provider-usage-popover").hidden, true);

    surface.applyProviderUsageUi(populated);
    assert.equal(strip.hidden, false);
    assert.equal(strip.getAttribute("aria-expanded"), "false");
  } finally {
    delete window.__operatorShell;
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
    globalThis.CustomEvent = previousCustomEvent;
    globalThis.requestAnimationFrame = previousRequestAnimationFrame;
  }
});

test("Usage popover keeps keyboard focus within the trigger-popover boundary", async () => {
  const { createProviderUsageSurface } = await import(
    resolve(here, "../provider-usage-surface.js")
  );
  const { applyProviderUsage } = await importOperatorShell();
  const { document, window } = parseHTML(
    "<html><body><button id='op-strip-usage'></button></body></html>",
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
          provider: "claude_code",
          windows: [],
          state: { kind: "disabled" },
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
    strip.getBoundingClientRect = () => ({ left: 24, top: 640 });
    strip.dispatchEvent(new window.Event("focus"));
    const popover = document.getElementById("provider-usage-popover");
    const action = popover.querySelector(".op-usage-card__reason--action");
    assert.equal(popover.getAttribute("tabindex"), "0");

    strip.onblur({ relatedTarget: action });
    action.dispatchEvent(new window.Event("focusin", { bubbles: true }));
    await new Promise((resolveWait) => setTimeout(resolveWait, 220));

    assert.equal(popover.hidden, false);
    assert.equal(strip.getAttribute("aria-expanded"), "true");

    const escapeEvent = new window.Event("keydown", { bubbles: true });
    escapeEvent.key = "Escape";
    popover.dispatchEvent(escapeEvent);
    assert.equal(popover.hidden, true);
    assert.equal(strip.getAttribute("aria-expanded"), "false");

    window.__gwtShowUsageHover(strip);
    const refreshedAction = popover.querySelector(".op-usage-card__reason--action");
    let popoverFocusRestored = false;
    Object.defineProperty(document, "activeElement", {
      configurable: true,
      value: refreshedAction,
    });
    popover.focus = () => {
      popoverFocusRestored = true;
    };
    surface.applyProviderUsageUi(snapshot);
    assert.equal(popoverFocusRestored, true);
  } finally {
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
    globalThis.CustomEvent = previousCustomEvent;
    globalThis.requestAnimationFrame = previousRequestAnimationFrame;
  }
});

test("Status strip compacts three providers while keeping the full accessible summary", async () => {
  const { applyProviderUsage } = await importOperatorShell();
  const { document } = parseHTML(
    "<html><body><button id='op-strip-usage'></button></body></html>",
  );

  applyProviderUsage(document, {
    accounts: [
      {
        provider: "codex",
        windows: [{ kind: "weekly", used_percent: 40 }],
        state: { kind: "ok" },
      },
      {
        provider: "claude_code",
        windows: [{ kind: "weekly", used_percent: 70 }],
        state: { kind: "ok" },
      },
      {
        provider: "gemini",
        windows: [{ kind: "weekly", used_percent: 95 }],
        state: { kind: "ok" },
      },
    ],
  });

  const strip = document.getElementById("op-strip-usage");
  const visibleSummaries = strip.querySelectorAll(".op-usage-sum");
  assert.equal(visibleSummaries.length, 1);
  assert.equal(visibleSummaries[0].dataset.provider, "gemini");
  assert.equal(visibleSummaries[0].textContent.trim(), "GE 95%");
  assert.equal(strip.querySelector(".op-usage-more").textContent, "+2");
  assert.equal(
    strip.getAttribute("aria-label"),
    "Provider usage: Codex 40% normal, Claude Code 70% normal, GE 95% danger",
  );
  assert.equal(
    strip.title,
    "Provider usage: Codex 40% normal, Claude Code 70% normal, GE 95% danger",
  );
});

test("Keyboard focus opens the same labeled usage popover", async () => {
  const { createProviderUsageSurface } = await import(
    resolve(here, "../provider-usage-surface.js")
  );
  const { applyProviderUsage } = await importOperatorShell();
  const { document, window } = parseHTML(
    "<html><body><button id='op-strip-usage'></button></body></html>",
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
          windows: [
            {
              kind: "weekly",
              used_percent: 42,
              resets_at: "2026-08-31T01:00:00Z",
            },
          ],
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
    strip.getBoundingClientRect = () => ({ left: 24, top: 640 });

    strip.dispatchEvent(new window.Event("focus"));

    const popover = document.querySelector(".op-usage-hover");
    assert.ok(popover, "focus should create the usage popover");
    assert.equal(popover.id, "provider-usage-popover");
    assert.equal(popover.getAttribute("role"), "region");
    assert.equal(popover.getAttribute("aria-label"), "Usage & Limits");
    assert.equal(strip.getAttribute("aria-controls"), popover.id);
    assert.equal(strip.getAttribute("aria-expanded"), "true");
    assert.match(popover.textContent, /Weekly\s*42%\s*↻/);
  } finally {
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
    globalThis.CustomEvent = previousCustomEvent;
    globalThis.requestAnimationFrame = previousRequestAnimationFrame;
  }
});

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

test("Usage popover labels unknown windows by their reported length (Issue #3860)", async () => {
  const { createProviderUsageSurface } = await import(
    resolve(here, "../provider-usage-surface.js")
  );
  const { document, window } = parseHTML(
    "<html><body><div id='usage-anchor'></div></body></html>",
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
    const surface = createProviderUsageSurface({
      send: () => {},
      renderWorkspaceWindows: () => {},
    });
    surface.applyProviderUsageUi({
      accounts: [
        {
          provider: "codex",
          windows: [
            { kind: "weekly", used_percent: 5, window_minutes: 10080 },
            { kind: "unknown", used_percent: 40, window_minutes: 1440 },
            { kind: "unknown", used_percent: 12 },
          ],
          state: { kind: "ok" },
        },
      ],
      consumption: [],
      sessions: [],
    });
    const anchor = document.getElementById("usage-anchor");
    anchor.getBoundingClientRect = () => ({ left: 24, top: 640 });
    window.__gwtShowUsageHover(anchor);

    const labels = [...document.querySelectorAll(".op-usage-win__lbl")];
    assert.deepEqual(
      labels.map((label) => label.textContent),
      ["Weekly", "Unknown (1-day)", "Unknown"],
    );
    // AC-4: the real window length is exposed on every row that has one.
    assert.deepEqual(
      labels.map((label) => label.dataset.windowMinutes),
      ["10080", "1440", undefined],
    );
    assert.equal(labels[0].title, "Window length: 7 days");
    assert.equal(labels[1].title, "Window length: 1 day");
    assert.equal(labels[2].title, "");
    // AC-5: the unknown window's value is still rendered, not dropped.
    assert.deepEqual(
      [...document.querySelectorAll(".op-usage-win__pct")].map((pct) => pct.textContent),
      ["5%", "40%", "12%"],
    );
  } finally {
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
    globalThis.CustomEvent = previousCustomEvent;
    globalThis.requestAnimationFrame = previousRequestAnimationFrame;
  }
});

test("Usage popover lists per-session tokens and context per provider, capped (Issue #3862)", async () => {
  const { createProviderUsageSurface } = await import(
    resolve(here, "../provider-usage-surface.js")
  );
  const { document, window } = parseHTML(
    "<html><body><div id='usage-anchor'></div></body></html>",
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
    const surface = createProviderUsageSurface({
      send: () => {},
      renderWorkspaceWindows: () => {},
      sessionLabel: (sessionId) =>
        sessionId === "sess-claude-a" ? "Issue #3862 popover" : null,
    });
    const codexSessions = Array.from({ length: 7 }, (_, i) => ({
      session_id: `sess-codex-${i}`,
      provider: "codex",
      model: "gpt-5-codex",
      total_tokens: 1000 * (i + 1),
      context_left_pct: 90 - i * 10,
      eligible: true,
      state: { kind: "ok" },
    }));
    surface.applyProviderUsageUi({
      accounts: [
        {
          provider: "codex",
          windows: [{ kind: "weekly", used_percent: 10 }],
          state: { kind: "ok" },
        },
        {
          provider: "claude_code",
          windows: [],
          state: { kind: "disabled" },
        },
      ],
      consumption: [],
      sessions: [
        ...codexSessions,
        {
          session_id: "sess-claude-a",
          provider: "claude_code",
          model: "claude-fable-5-1",
          total_tokens: 1234567,
          context_used_tokens: 420000,
          context_limit_tokens: 1000000,
          context_left_pct: 58,
          eligible: true,
          state: { kind: "ok" },
        },
        {
          session_id: "sess-claude-b",
          provider: "claude_code",
          model: "claude-sonnet-5",
          total_tokens: 2500,
          context_left_pct: 12,
          eligible: true,
          state: { kind: "ok" },
        },
        {
          session_id: "sess-claude-apikey",
          provider: "claude_code",
          model: null,
          total_tokens: 900,
          context_left_pct: null,
          eligible: false,
          state: { kind: "ok" },
        },
        {
          session_id: "sess-claude-nodata",
          provider: "claude_code",
          model: null,
          total_tokens: 0,
          context_left_pct: null,
          eligible: true,
          state: { kind: "no_data" },
        },
      ],
    });
    const anchor = document.getElementById("usage-anchor");
    anchor.getBoundingClientRect = () => ({ left: 24, top: 640 });
    window.__gwtShowUsageHover(anchor);

    // Claude card: sessions are visible even while account usage is disabled
    // (per-session data is local and opt-in free); the account reason stays.
    const claudeCard = document.querySelector('.op-usage-card[data-provider="claude_code"]');
    assert.ok(claudeCard, "claude card renders");
    assert.match(claudeCard.textContent, /Enable in Settings/);
    const claudeSess = claudeCard.querySelector(".op-usage-sess");
    assert.ok(claudeSess, "claude sessions block renders");
    assert.match(claudeSess.querySelector(".op-usage-sess__head").textContent, /Sessions/);
    const claudeRows = [...claudeSess.querySelectorAll(".op-usage-sess__row")];
    assert.equal(claudeRows.length, 4);
    // Lowest remaining context first, then unknown context, then ineligible.
    assert.deepEqual(
      claudeRows.map((row) => row.dataset.sessionId),
      ["sess-claude-b", "sess-claude-a", "sess-claude-nodata", "sess-claude-apikey"],
    );
    const named = claudeRows[1];
    assert.equal(named.querySelector(".op-usage-sess__name").textContent, "Issue #3862 popover");
    assert.equal(named.querySelector(".op-usage-sess__model").textContent, "claude-fable-5-1");
    assert.equal(named.querySelector(".op-usage-sess__tokens").textContent, "1.2M");
    assert.match(named.querySelector(".op-usage-sess__ctx").textContent, /58%/);
    assert.match(named.querySelector(".op-usage-sess__ctx").title, /420k \/ 1\.0M/);
    // Unnamed sessions fall back to a short session id, never an empty cell.
    assert.equal(claudeRows[0].querySelector(".op-usage-sess__name").textContent, "sess-cla…");
    // Ineligible (API-key backend) sessions never show quota/context values.
    const apiKey = claudeRows[3];
    assert.equal(apiKey.dataset.eligible, "false");
    assert.equal(apiKey.querySelector(".op-usage-sess__ctx").textContent, "n/a");
    assert.equal(apiKey.querySelector(".op-usage-sess__tokens").textContent, "900");
    // A session without data shows its state instead of a fake 0 / 100%.
    const noData = claudeRows[2];
    assert.equal(noData.querySelector(".op-usage-sess__ctx").textContent, "No data yet");
    assert.equal(noData.querySelector(".op-usage-sess__tokens").textContent, "—");

    // Codex card: 7 sessions are capped to 5 rows plus a "+N more" line so the
    // popover never grows unbounded (the reason the old list was removed).
    const codexCard = document.querySelector('.op-usage-card[data-provider="codex"]');
    const codexSess = codexCard.querySelector(".op-usage-sess");
    assert.match(codexSess.querySelector(".op-usage-sess__head").textContent, /Sessions \(7\)/);
    const codexRows = [...codexSess.querySelectorAll(".op-usage-sess__row")];
    assert.equal(codexRows.length, 5);
    assert.equal(codexRows[0].dataset.sessionId, "sess-codex-6");
    assert.equal(codexSess.querySelector(".op-usage-sess__more").textContent, "+2 more sessions");

    // Sessions are absent when the snapshot carries none for the provider.
    surface.applyProviderUsageUi({
      accounts: [{ provider: "codex", windows: [], state: { kind: "no_data" } }],
      consumption: [],
      sessions: [],
    });
    assert.equal(document.querySelector(".op-usage-sess"), null);
    assert.match(document.body.textContent, /No data yet/);
  } finally {
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
    globalThis.CustomEvent = previousCustomEvent;
    globalThis.requestAnimationFrame = previousRequestAnimationFrame;
  }
});
