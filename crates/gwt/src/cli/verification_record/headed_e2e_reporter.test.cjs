const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const Reporter = require("./headed_e2e_reporter.cjs");

function trace(colorScheme, { headless = false, browserName = "chromium", launch = true, version = 7 } = {}) {
  return [
    { type: "context-options", origin: "testRunner", version },
    ...(launch ? [
      { type: "before", callId: "launch", apiName: "browserType.launch", params: { headless: String(headless) } },
      { type: "after", callId: "launch" },
    ] : []),
    { type: "context-options", origin: "library", version, browserName, options: { colorScheme } },
  ];
}

async function capture(results, status = "passed", runnerError = false) {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-headed-test-"));
  const report = path.join(temporary, "report.json");
  const previous = process.env.GWT_HEADED_E2E_REPORT;
  process.env.GWT_HEADED_E2E_REPORT = report;
  try {
    const reporter = new Reporter();
    reporter.traceEvents = async result => result.trace;
    for (const [colorScheme, resultStatus, events = trace(colorScheme), workerIndex = 0] of results) {
      const currentTest = { parent: { project: () => ({ use: { headless: false, browserName: "chromium", colorScheme } }) } };
      const result = { status: resultStatus, workerIndex, trace: events };
      reporter.onTestEnd(currentTest, result);
    }
    if (runnerError) reporter.onError(new Error("runner failed"));
    await reporter.onEnd({ status });
    return JSON.parse(fs.readFileSync(report, "utf8"));
  } finally {
    if (previous === undefined) delete process.env.GWT_HEADED_E2E_REPORT;
    else process.env.GWT_HEADED_E2E_REPORT = previous;
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

test("records actual headed Chromium passes in both themes", async () => {
  assert.deepEqual(await capture([["dark", "passed"], ["light", "passed"]]), {
    chromium_dark_passed: 1, chromium_light_passed: 1, failed: 0, status: "passed",
  });
});

test("all skipped and browser-unused tests do not supply evidence", async () => {
  const evidence = await capture([["dark", "skipped"], ["light", "passed", []]]);
  assert.equal(evidence.chromium_dark_passed + evidence.chromium_light_passed, 0);
});

test("runtime headless and browser overrides take precedence over project settings", async () => {
  const evidence = await capture([
    ["dark", "passed", trace("dark", { headless: true })],
    ["light", "passed", trace("light", { browserName: "firefox" })],
  ]);
  assert.equal(evidence.chromium_dark_passed + evidence.chromium_light_passed, 0);
});

test("runtime theme override and browser reuse are measured", async () => {
  const evidence = await capture([
    ["dark", "passed", trace("light")],
    ["dark", "passed", trace("light", { launch: false })],
  ]);
  assert.equal(evidence.chromium_dark_passed, 0);
  assert.equal(evidence.chromium_light_passed, 2);
});

test("missing traces, unknown versions, and another worker's browser fail closed", async () => {
  const evidence = await capture([
    ["dark", "passed", []],
    ["dark", "passed", trace("dark", { version: 99 })],
    ["light", "passed", trace("light", { launch: false }), 1],
  ]);
  assert.equal(evidence.chromium_dark_passed + evidence.chromium_light_passed, 0);
});

test("failed, timed out, and interrupted tests remain failures", async () => {
  const evidence = await capture([["dark", "failed"], ["light", "timedOut"], ["dark", "interrupted"]], "failed");
  assert.equal(evidence.failed, 3);
  assert.equal(evidence.status, "failed");
});

test("runner errors remain failures even after both themes pass", async () => {
  const evidence = await capture([["dark", "passed"], ["light", "passed"]], "failed", true);
  assert.equal(evidence.failed, 1);
  assert.equal(evidence.status, "failed");
});
