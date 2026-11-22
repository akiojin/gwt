import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";
import { accountRows } from "../execution-environments.js";

const here = dirname(fileURLToPath(import.meta.url));
const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");

// Issue #7648: the account card is drawn on demand into the Execution
// environments account detail, so each read renders the current snapshot.
function usageCard(surface, document, provider) {
  let container = document.getElementById("usage-slot");
  if (!container) {
    container = document.createElement("div");
    container.id = "usage-slot";
    document.body.appendChild(container);
  }
  surface.renderAccountUsage(container, provider);
  return container.querySelector(".op-usage-card");
}

test("Usage severity styles use Operator tokens without raw color fallbacks", () => {
  assert.match(
    componentsCss,
    /\.op-usage-bar__fill\[data-severity="warning"\]\s*\{[^}]*background:\s*var\(--color-state-needs-input\);/s,
  );
  assert.match(
    componentsCss,
    /\.op-usage-bar__fill\[data-severity="danger"\]\s*\{[^}]*background:\s*var\(--color-state-blocked\);/s,
  );
  assert.doesNotMatch(componentsCss, /op-strip-usage/, "the status-strip usage cell is gone");
  assert.doesNotMatch(componentsCss, /\.op-usage-hover/, "the usage hover popover is gone");
});

test("Codex full subscription distinguishes available credits from exhausted credits", () => {
  const account = { provider: "codex", state: { kind: "ok" }, limit_reached: false,
    windows: [{ kind: "weekly", used_percent: 100 }],
    credits: { has_credits: true, unlimited: false, balance: 37669.9 } };
  let [row] = accountRows({ accounts: [account] });
  assert.match(row.usage, /Using credits.*37,669\.9/);
  assert.equal(row.usingCredits, true);
  assert.equal(row.limitReached, false);
  account.credits.has_credits = false;
  [row] = accountRows({ accounts: [account] });
  assert.equal(row.limitReached, true);
  delete account.credits;
  [row] = accountRows({ accounts: [account] });
  assert.equal(row.usage, "100% used");
  assert.equal(row.limitReached, false);
});

test("Account row summarizes the provider window nearest its limit", () => {
  const [row] = accountRows({ accounts: [{ provider: "codex", state: { kind: "ok" }, windows: [
    { kind: "weekly", used_percent: 29 },
    { kind: "five_hour", used_percent: 96 },
  ] }] });
  assert.equal(row.usage, "96% used");
});

test("Account row excludes missing usage values from its summary", () => {
  const [row] = accountRows({ accounts: [{ provider: "codex", state: { kind: "ok" }, windows: [
    { kind: "weekly", used_percent: null },
    { kind: "five_hour", used_percent: "" },
    { kind: "opus_weekly", used_percent: "unknown" },
  ] }] });
  assert.equal(row.usage, null);
  assert.equal(row.severity, "");
});

test("Claude account summary uses shared windows while Codex keeps all windows", () => {
  const modelWindows = ["fable_weekly", "opus_weekly", "sonnet_weekly"].map(
    (kind) => ({ kind, used_percent: 100 }),
  );
  const account = { provider: "claude_code", state: { kind: "ok" }, limit_reached: false };
  const cases = [
    {
      windows: [...modelWindows, { kind: "five_hour", used_percent: 29 }, { kind: "weekly", used_percent: 12 }],
      usage: "29% used", severity: "normal",
    },
    {
      windows: [...modelWindows, { kind: "five_hour", used_percent: 12 }, { kind: "weekly", used_percent: 96 }],
      usage: "96% used", severity: "danger",
    },
    { windows: modelWindows, usage: null, severity: "" },
  ];
  for (const { windows, usage, severity } of cases) {
    const [row] = accountRows({ accounts: [{ ...account, windows }] });
    assert.equal(row.usage, usage);
    assert.equal(row.severity, severity);
    assert.equal(row.limitReached, false);
  }

  const [codex] = accountRows({ accounts: [{ provider: "codex", state: { kind: "ok" }, windows: [
    { kind: "weekly", used_percent: 29 },
    { kind: "code_review_weekly", used_percent: 100 },
  ] }] });
  assert.equal(codex.usage, "100% used");
});

test("Account row classifies usage at the 80 and 95 percent boundaries", () => {
  for (const [percent, expected] of [[79, "normal"], [80, "warning"], [94, "warning"], [95, "danger"]]) {
    const [row] = accountRows({ accounts: [{ provider: "codex", state: { kind: "ok" },
      windows: [{ kind: "weekly", used_percent: percent }] }] });
    assert.equal(row.severity, expected, `${percent}% should be ${expected}`);
  }
  const [limited] = accountRows({ accounts: [{ provider: "codex", state: { kind: "ok" }, limit_reached: true,
    windows: [{ kind: "weekly", used_percent: 1 }] }] });
  assert.equal(limited.severity, "danger");
  assert.equal(limited.limitReached, true);
  assert.equal(limited.usage, "1% used");
});

test("Settings usage panel only links to Accounts", async () => {
  const { createProviderUsageSurface } = await import(pathToFileURL(resolve(here, "../provider-usage-surface.js")).href);
  const { document, window } = parseHTML("<html><body><section id='usage-panel'></section></body></html>");
  const previous = { document: globalThis.document, window: globalThis.window,
    CustomEvent: globalThis.CustomEvent, requestAnimationFrame: globalThis.requestAnimationFrame };
  Object.assign(globalThis, { document, window, CustomEvent: window.CustomEvent,
    requestAnimationFrame: callback => callback() });
  try {
    const surface = createProviderUsageSurface({ renderWorkspaceWindows: () => {} });
    surface.applyProviderUsageUi({ accounts: [{ provider: "codex", state: { kind: "ok" },
      windows: [{ kind: "weekly", used_percent: 42 }] }], consumption: [], sessions: [] });
    const panel = document.getElementById("usage-panel");
    surface.renderUsagePanel(panel);
    surface.renderUsagePanel(panel);
    assert.equal(panel.querySelectorAll("button").length, 1, "re-rendering replaces the panel");
    assert.equal(panel.querySelector("input"), null);
    assert.doesNotMatch(panel.textContent, /42%/, "usage is not repeated in Settings");
    const opened = [];
    document.addEventListener("environments:open", event => opened.push(event.detail?.view));
    const button = panel.querySelector("button");
    assert.equal(button.textContent, "Open Accounts");
    button.click();
    assert.deepEqual(opened, ["accounts"]);
  } finally {
    Object.assign(globalThis, previous);
  }
});

test("Codex account card derives credit pace only from distinct same-account observations", async () => {
  const { createProviderUsageSurface } = await import(pathToFileURL(resolve(here, "../provider-usage-surface.js")).href);
  const { document, window } = parseHTML("<html><body></body></html>");
  const previous = { document: globalThis.document, window: globalThis.window,
    CustomEvent: globalThis.CustomEvent, requestAnimationFrame: globalThis.requestAnimationFrame };
  Object.assign(globalThis, { document, window, CustomEvent: window.CustomEvent,
    requestAnimationFrame: callback => callback() });
  try {
    const surface = createProviderUsageSurface({ renderWorkspaceWindows() {} });
    const update = (balance, fetched_at, account_id = "account-a", state = { kind: "ok" }) => {
      surface.applyProviderUsageUi({ accounts: [{ provider: "codex", account_id, fetched_at, state,
        limit_reached: false, windows: [{ kind: "weekly", used_percent: 100 }],
        credits: { has_credits: true, unlimited: false, balance } }], sessions: [], consumption: [] });
    };
    update(100, "2031-01-02T10:00:00Z");
    const card = () => usageCard(surface, document, "codex");
    assert.match(card().textContent, /Using credits.*100 credits/);
    assert.doesNotMatch(card().textContent, /\/hour|Limit reached/);
    update(90, "2031-01-02T10:30:00Z");
    assert.match(card().textContent, /90 credits.*20 credits\/hour/);
    update(90, "2031-01-02T10:30:00Z", "account-a", { kind: "stale", age_secs: 420 });
    assert.match(card().textContent, /stale 7m/);
    assert.match(card().textContent, /20 credits\/hour/);
    update(80, "2031-01-02T11:00:00Z", "account-b");
    assert.doesNotMatch(card().textContent, /\/hour/);
    update(120, "2031-01-02T11:30:00Z", "account-b");
    assert.doesNotMatch(card().textContent, /\/hour/);
    update(null, "2031-01-02T12:00:00Z", "account-b");
    assert.match(card().textContent, /— credits/);
    assert.doesNotMatch(card().textContent, /\/hour/);
  } finally {
    Object.assign(globalThis, previous);
  }
});
test("Account card bars use the same 80 and 95 percent severity boundaries", async () => {
  const { createProviderUsageSurface } = await import(
    pathToFileURL(resolve(here, "../provider-usage-surface.js")).href
  );
  const { document, window } = parseHTML(
    "<html><body></body></html>",
  );
  const previousDocument = globalThis.document;
  const previousWindow = globalThis.window;
  const previousCustomEvent = globalThis.CustomEvent;
  const previousRequestAnimationFrame = globalThis.requestAnimationFrame;
  globalThis.document = document;
  globalThis.window = window;
  globalThis.CustomEvent = window.CustomEvent;
  globalThis.requestAnimationFrame = (cb) => cb();

  try {
    const surface = createProviderUsageSurface({
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

    assert.deepEqual(
      [...usageCard(surface, document, "codex").querySelectorAll(".op-usage-bar__fill")].map(
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
    const limited = usageCard(surface, document, "codex");
    assert.equal(limited.querySelector(".op-usage-bar__fill").dataset.severity, "danger");
    assert.match(limited.textContent, /Limit reached/);
  } finally {
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
    globalThis.CustomEvent = previousCustomEvent;
    globalThis.requestAnimationFrame = previousRequestAnimationFrame;
  }
});
test("Account card shows the account label", async () => {
  const { createProviderUsageSurface } = await import(
    pathToFileURL(resolve(here, "../provider-usage-surface.js")).href
  );
  const { document, window } = parseHTML(
    "<html><body></body></html>",
  );
  const previousDocument = globalThis.document;
  const previousWindow = globalThis.window;
  const previousCustomEvent = globalThis.CustomEvent;
  const previousRequestAnimationFrame = globalThis.requestAnimationFrame;
  globalThis.document = document;
  globalThis.window = window;
  globalThis.CustomEvent = window.CustomEvent;
  globalThis.requestAnimationFrame = (cb) => cb();

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
      renderWorkspaceWindows: () => {},
    });
    surface.applyProviderUsageUi(snapshot);

    assert.match(usageCard(surface, document, "codex").textContent, /Account:\s*codex@example\.com/);
  } finally {
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
    globalThis.CustomEvent = previousCustomEvent;
    globalThis.requestAnimationFrame = previousRequestAnimationFrame;
  }
});
test("Account card labels unknown windows by their reported length (Issue #3860)", async () => {
  const { createProviderUsageSurface } = await import(
    pathToFileURL(resolve(here, "../provider-usage-surface.js")).href
  );
  const { document, window } = parseHTML(
    "<html><body></body></html>",
  );
  const previousDocument = globalThis.document;
  const previousWindow = globalThis.window;
  const previousCustomEvent = globalThis.CustomEvent;
  const previousRequestAnimationFrame = globalThis.requestAnimationFrame;
  globalThis.document = document;
  globalThis.window = window;
  globalThis.CustomEvent = window.CustomEvent;
  globalThis.requestAnimationFrame = (cb) => cb();

  try {
    const surface = createProviderUsageSurface({
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

    const card = usageCard(surface, document, "codex");
    const labels = [...card.querySelectorAll(".op-usage-win__lbl")];
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
      [...card.querySelectorAll(".op-usage-win__pct")].map((pct) => pct.textContent),
      ["5%", "40%", "12%"],
    );
  } finally {
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
    globalThis.CustomEvent = previousCustomEvent;
    globalThis.requestAnimationFrame = previousRequestAnimationFrame;
  }
});
test("Claude account card labels Fable weekly with its reported usage and reset", async () => {
  const { createProviderUsageSurface } = await import(
    pathToFileURL(resolve(here, "../provider-usage-surface.js")).href
  );
  const { document, window } = parseHTML("<html><body></body></html>");
  const previous = { document: globalThis.document, window: globalThis.window,
    CustomEvent: globalThis.CustomEvent, requestAnimationFrame: globalThis.requestAnimationFrame };
  Object.assign(globalThis, { document, window, CustomEvent: window.CustomEvent,
    requestAnimationFrame: callback => callback() });
  try {
    const surface = createProviderUsageSurface({ renderWorkspaceWindows() {} });
    const resetsAt = "2031-01-02T03:04:05Z";
    surface.applyProviderUsageUi({ accounts: [{ provider: "claude_code", state: { kind: "ok" }, limit_reached: true,
      windows: [{ kind: "fable_weekly", used_percent: 0, resets_at: resetsAt }] }],
      consumption: [], sessions: [] });

    const row = usageCard(surface, document, "claude_code").querySelector(".op-usage-win");
    assert.equal(row.querySelector(".op-usage-win__lbl").textContent, "Fable weekly");
    assert.equal(row.querySelector(".op-usage-win__pct").textContent, "0%");
    assert.equal(row.querySelector(".op-usage-bar__fill").style.width, "0%");
    assert.equal(row.querySelector(".op-usage-bar__fill").dataset.severity, "normal");
    assert.equal(document.querySelector(".op-usage-card__limit").textContent, "Limit reached");
    assert.equal(row.querySelector(".op-usage-win__reset").textContent,
      `↻ ${new Date(resetsAt).toLocaleString(undefined, {
        month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit",
      })}`);
    surface.applyProviderUsageUi({ accounts: [{ provider: "claude_code", state: { kind: "stale", age_secs: 420 },
      windows: [{ kind: "fable_weekly", used_percent: null },
        { kind: "opus_weekly", used_percent: "invalid" }] }], sessions: [], consumption: [] });
    usageCard(surface, document, "claude_code");
    assert.deepEqual([...document.querySelectorAll(".op-usage-win__pct")].map(el => el.textContent), ["—", "—"]);
    assert.equal(document.querySelectorAll(".op-usage-bar").length, 0);
    assert.match(document.querySelector(".op-usage-card").textContent, /stale 7m/);
  } finally {
    Object.assign(globalThis, previous);
  }
});
test("Claude usage credits preserve supplied amounts and missing values", async () => {
  const { createProviderUsageSurface } = await import(
    pathToFileURL(resolve(here, "../provider-usage-surface.js")).href
  );
  const { document, window } = parseHTML("<html><body></body></html>");
  const previous = { document: globalThis.document, window: globalThis.window,
    CustomEvent: globalThis.CustomEvent, requestAnimationFrame: globalThis.requestAnimationFrame };
  Object.assign(globalThis, { document, window, CustomEvent: window.CustomEvent,
    requestAnimationFrame: callback => callback() });
  try {
    const surface = createProviderUsageSurface({ renderWorkspaceWindows() {} });
    const account = { provider: "claude_code", state: { kind: "ok" },
      windows: [{ kind: "weekly", used_percent: 12 }] };
    const applyExtraUsage = extraUsage => {
      surface.applyProviderUsageUi({
        accounts: [{ ...account, extra_usage: extraUsage }], consumption: [], sessions: [],
      });
      usageCard(surface, document, "claude_code");
    };
    applyExtraUsage({ is_enabled: true, used_credits: 24.5, monthly_limit: 100,
      utilization: 24.5, currency: "USD", disabled_reason: null });

    let credits = document.querySelector('[data-usage-credits="true"]');
    assert.ok(credits, "enabled usage credits should be visible");
    assert.equal(credits.querySelector(".op-usage-win__lbl").textContent, "Usage credits");
    assert.equal(credits.querySelector(".op-usage-win__pct").textContent, "25%");
    assert.equal(credits.querySelector(".op-usage-bar__fill").style.width, "25%");
    assert.equal(credits.querySelector(".op-usage-win__reset").textContent, "24.5 / 100 USD");

    applyExtraUsage({ is_enabled: true, used_credits: 0, monthly_limit: null,
      utilization: null, currency: "USD", disabled_reason: null });
    credits = document.querySelector('[data-usage-credits="true"]');
    assert.equal(credits.querySelector(".op-usage-win__reset").textContent, "0 / — USD");
    assert.equal(credits.querySelector(".op-usage-win__pct").textContent, "—");
    assert.equal(credits.querySelector(".op-usage-bar"), null);

    applyExtraUsage({ is_enabled: true, used_credits: null, monthly_limit: null,
      utilization: null, currency: null, disabled_reason: null });
    credits = document.querySelector('[data-usage-credits="true"]');
    assert.equal(credits.querySelector(".op-usage-win__reset").textContent, "— / —");
    assert.doesNotMatch(credits.textContent, /0|USD/);

    for (const extraUsage of [undefined, null, { is_enabled: false, used_credits: 24.5,
      monthly_limit: 100, utilization: 24.5, currency: "USD", disabled_reason: null }]) {
      applyExtraUsage(extraUsage);
      assert.equal(document.querySelector('[data-usage-credits="true"]'), null);
      assert.equal(document.querySelector(".op-usage-win__pct").textContent, "12%");
    }
  } finally {
    Object.assign(globalThis, previous);
  }
});
test("Account card lists per-session tokens and context per provider, capped (Issue #3862)", async () => {
  const { createProviderUsageSurface } = await import(
    pathToFileURL(resolve(here, "../provider-usage-surface.js")).href
  );
  const { document, window } = parseHTML(
    "<html><body></body></html>",
  );
  const previousDocument = globalThis.document;
  const previousWindow = globalThis.window;
  const previousCustomEvent = globalThis.CustomEvent;
  const previousRequestAnimationFrame = globalThis.requestAnimationFrame;
  globalThis.document = document;
  globalThis.window = window;
  globalThis.CustomEvent = window.CustomEvent;
  globalThis.requestAnimationFrame = (cb) => cb();

  try {
    const surface = createProviderUsageSurface({
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

    // Claude card: sessions are visible even while account usage is disabled
    // (per-session data is local and opt-in free); the account reason stays.
    const claudeCard = usageCard(surface, document, "claude_code");
    assert.ok(claudeCard, "claude card renders");
    assert.match(claudeCard.textContent, /Usage collection is off/);
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
    // card never grows unbounded (the reason the old list was removed).
    const codexCard = usageCard(surface, document, "codex");
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
    const emptyCard = usageCard(surface, document, "codex");
    assert.equal(emptyCard.querySelector(".op-usage-sess"), null);
    assert.match(emptyCard.textContent, /No data yet/);
  } finally {
    globalThis.document = previousDocument;
    globalThis.window = previousWindow;
    globalThis.CustomEvent = previousCustomEvent;
    globalThis.requestAnimationFrame = previousRequestAnimationFrame;
  }
});

test("Usage snapshot owns its data and batches Workspace refreshes per frame", async () => {
  const { createProviderUsageSurface } = await import(pathToFileURL(resolve(here, "../provider-usage-surface.js")).href);
  const { document, window } = parseHTML("<html><body><div id='slot'></div></body></html>");
  const previous = {
    document: globalThis.document,
    window: globalThis.window,
    requestAnimationFrame: globalThis.requestAnimationFrame,
    cancelAnimationFrame: globalThis.cancelAnimationFrame,
  };
  const frames = new Map();
  let frameId = 0;
  const flush = () => {
    for (const callback of frames.values()) callback();
    frames.clear();
  };
  Object.assign(globalThis, { document, window,
    requestAnimationFrame: callback => { frames.set(++frameId, callback); return frameId; },
    cancelAnimationFrame: id => frames.delete(id),
  });
  const payload = {
    accounts: [{ provider: "codex", windows: [{ kind: "weekly", used_percent: 10 }], state: { kind: "ok" } }],
    sessions: [],
    consumption: [],
  };
  try {
    let workspaceRefreshes = 0;
    const surface = createProviderUsageSurface({ renderWorkspaceWindows: () => workspaceRefreshes++ });
    assert.equal(workspaceRefreshes, 0, "factory construction must not call a late-bound Workspace view");
    surface.applyProviderUsageUi(payload);
    assert.equal(workspaceRefreshes, 0, "views wait for the shared frame");
    flush();
    assert.equal(workspaceRefreshes, 1);
    // A caller changing the received payload must not change model data.
    payload.accounts[0].windows[0].used_percent = 70;
    const slot = document.getElementById("slot");
    surface.renderAccountUsage(slot, "codex");
    assert.match(slot.textContent, /Weekly\s*10%/);

    surface.applyProviderUsageUi({ ...payload, accounts: [] });
    surface.applyProviderUsageUi(payload);
    assert.equal(frames.size, 1, "burst updates share one frame");
    flush();
    assert.equal(workspaceRefreshes, 2);
    surface.renderAccountUsage(slot, "codex");
    assert.match(slot.textContent, /Weekly\s*70%/);
  } finally {
    flush();
    Object.assign(globalThis, previous);
  }
});
