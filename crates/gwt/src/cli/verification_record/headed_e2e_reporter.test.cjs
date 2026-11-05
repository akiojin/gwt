const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");
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

async function capture(results, status = "passed", runnerError = false, outputDir, runtime = []) {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-headed-test-"));
  const report = path.join(temporary, "report.json");
  const previous = process.env.GWT_HEADED_E2E_REPORT;
  const previousNodeOptions = process.env.NODE_OPTIONS;
  const previousModuleRoot = process.env.GWT_HEADED_E2E_MODULE_ROOT;
  process.env.GWT_HEADED_E2E_REPORT = report;
  try {
    const reporter = new Reporter();
    reporter.traceEvents = async result => {
      if (result.trace instanceof Error) throw result.trace;
      return result.trace;
    };
    reporter.runtimeContexts = () => runtime;
    reporter.onBegin({ projects: [{ outputDir }] });
    for (const [colorScheme, resultStatus, events = trace(colorScheme), workerIndex = 0, attachments = []] of results) {
      const currentTest = { id: colorScheme, parent: { project: () => ({ use: { headless: false, browserName: "chromium", colorScheme } }) } };
      const result = { status: resultStatus, workerIndex, trace: events, attachments };
      reporter.onTestEnd(currentTest, result);
    }
    if (runnerError) reporter.onError(new Error("runner failed"));
    await reporter.onEnd({ status });
    assert.equal(process.env.NODE_OPTIONS, previousNodeOptions);
    assert.equal(process.env.GWT_HEADED_E2E_MODULE_ROOT, previousModuleRoot);
    return JSON.parse(fs.readFileSync(report, "utf8"));
  } finally {
    if (previous === undefined) delete process.env.GWT_HEADED_E2E_REPORT;
    else process.env.GWT_HEADED_E2E_REPORT = previous;
    if (previousNodeOptions === undefined) delete process.env.NODE_OPTIONS;
    else process.env.NODE_OPTIONS = previousNodeOptions;
    fs.rmSync(temporary, { recursive: true, force: true });
  }
}

test("records actual headed Chromium passes in both themes", async () => {
  assert.deepEqual(await capture([["dark", "passed"], ["light", "passed"]]), {
    chromium_dark_passed: 1, chromium_light_passed: 1, failed: 0, status: "passed",
  });
});

test("records readable absolute paths for screenshot and trace attachments", async () => {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-headed-artifacts-"));
  const screenshot = path.join(temporary, "screenshot.png");
  const archive = path.join(temporary, "trace.zip");
  fs.writeFileSync(screenshot, "screenshot");
  fs.writeFileSync(archive, "trace");
  try {
    const evidence = await capture([["dark", "passed", trace("dark"), 0, [
      { name: "screenshot", path: path.relative(process.cwd(), screenshot) },
      { name: "trace", path: archive },
      { name: "missing", path: path.join(temporary, "missing.png") },
      { name: "inline", body: Buffer.from("inline") },
    ]]]);
    assert.deepEqual(evidence.artifacts, [screenshot, archive]);
    assert.ok(evidence.artifacts.every(artifact => path.isAbsolute(artifact)));
    assert.deepEqual(evidence.artifacts.map(artifact => fs.readFileSync(artifact, "utf8")), ["screenshot", "trace"]);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
});

test("persists inline PNG attachments beyond reporter cleanup", async () => {
  const outputDir = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-headed-output-"));
  const png = Buffer.from("png attachment");
  try {
    const evidence = await capture([["dark", "passed", trace("dark"), 0, [
      { name: "dark screenshot", contentType: "image/png", body: png },
      { name: "text", contentType: "text/plain", body: Buffer.from("text") },
      { name: "another screenshot", contentType: "image/png", body: png },
    ]]], "passed", false, outputDir);
    assert.equal(evidence.artifacts.length, 2);
    assert.notEqual(evidence.artifacts[0], evidence.artifacts[1]);
    for (const artifact of evidence.artifacts) {
      assert.ok(path.isAbsolute(artifact));
      assert.ok(artifact.startsWith(outputDir + path.sep));
      assert.equal(path.extname(artifact), ".png");
      assert.deepEqual(fs.readFileSync(artifact), png);
    }
  } finally {
    fs.rmSync(outputDir, { recursive: true, force: true });
  }
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

test("compatible trace structures do not depend on a version allowlist", async () => {
  const evidence = await capture([["dark", "passed", trace("dark", { version: 99 })]]);
  assert.equal(evidence.chromium_dark_passed, 1);
});

test("v8 launch titles preserve runtime headed proof", async () => {
  const events = trace("dark", { version: 8 }).map(event => event.type === "before"
    ? { ...event, apiName: undefined, class: "Test", method: "pw:api", title: "Launch browser" }
    : event);
  const evidence = await capture([["dark", "passed", events]]);
  assert.equal(evidence.chromium_dark_passed, 1);
});

test("real v8 fixtures count both headed themes", async () => {
  const results = ["dark", "light"].map(theme => [theme, "passed", fs.readFileSync(
    path.join(__dirname, "fixtures", `playwright-v8-${theme}.trace`), "utf8"
  ).trim().split("\n").map(JSON.parse)]);
  const evidence = await capture(results);
  assert.equal(evidence.chromium_dark_passed, 1);
  assert.equal(evidence.chromium_light_passed, 1);
});

test("v10 fixtures fall back to measured worker contexts, never project settings", async () => {
  const results = ["dark", "light"].map(theme => [theme, "passed", fs.readFileSync(
    path.join(__dirname, "fixtures", `playwright-v10-${theme}.trace`), "utf8"
  ).trim().split("\n").map(JSON.parse)]);
  const contexts = [{ browserName: "chromium", headless: false, colorScheme: "light" }];
  const evidence = await capture(results, "passed", false, undefined, contexts);
  assert.equal(evidence.chromium_dark_passed, 0, "runtime theme takes precedence");
  assert.equal(evidence.chromium_light_passed, 2);
  assert.match(evidence.diagnostics.join("\n"), /worker.*fallback/);
  const headless = await capture(results, "passed", false, undefined, [{ ...contexts[0], headless: true }]);
  assert.equal(headless.chromium_dark_passed + headless.chromium_light_passed, 0);
  const missing = await capture(results);
  assert.equal(missing.chromium_dark_passed + missing.chromium_light_passed, 0);
  assert.match(missing.diagnostics.join("\n"), /headed.*proof/);
});

test("unreadable and unsupported traces use actual worker proof with a diagnostic", async () => {
  const runtime = [{ browserName: "chromium", headless: false, colorScheme: "dark" }];
  const evidence = await capture([["dark", "passed", []]], "passed", false, undefined, runtime);
  assert.equal(evidence.chromium_dark_passed, 1);
  assert.match(evidence.diagnostics.join("\n"), /fallback/);
  const unreadable = await capture([["dark", "passed", new SyntaxError("invalid trace")]], "passed", false, undefined, runtime);
  assert.equal(unreadable.chromium_dark_passed, 1);
  assert.match(unreadable.diagnostics.join("\n"), /trace unavailable.*SyntaxError/);
});

test("worker measurement binds successful real pages to test, retry, and worker", async () => {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-headed-worker-"));
  try {
    const reportPath = path.join(temporary, "report.json");
    const context = { _options: { colorScheme: "dark" }, newPage: async () => "page" };
    const browser = { _options: { headless: false }, newContext: async () => context };
    const chromium = { launch: async () => browser, launchPersistentContext: async () => context };
    const info = { testId: "actual-test", retry: 1 };
    const load = name => name === "playwright-core" ? { chromium }
      : name === "playwright/test" ? { test: { info: () => info } } : require(name);
    load.resolve = name => name;
    vm.runInNewContext(fs.readFileSync(path.join(__dirname, "headed_e2e_runtime.cjs"), "utf8"), {
      require: load, process: { argv: [], cwd: () => temporary, env: {
        TEST_WORKER_INDEX: "3", GWT_HEADED_E2E_REPORT: reportPath,
      } },
    });
    const launched = await chromium.launch();
    const created = await launched.newContext();
    assert.equal(await created.newPage(), "page");
    const reporter = new Reporter();
    reporter.reportPath = reportPath;
    const result = { workerIndex: 3, retry: 1 };
    assert.deepEqual(reporter.runtimeContexts("actual-test", result), [{
      testId: "actual-test", retry: 1, browserName: "chromium", headless: false, colorScheme: "dark",
    }]);
    assert.deepEqual(reporter.runtimeContexts("other-test", result), []);
    assert.deepEqual(reporter.runtimeContexts("actual-test", { ...result, retry: 0 }), []);
    assert.deepEqual(reporter.runtimeContexts("actual-test", { ...result, workerIndex: 4 }), []);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
});

test("missing traces and another worker's browser fail closed", async () => {
  const evidence = await capture([
    ["dark", "passed", []],
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
