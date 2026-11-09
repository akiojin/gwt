const fs = require("node:fs");
const path = require("node:path");

// Trace API parameters include test.use overrides that project.use omits.
module.exports = class HeadedE2eReporter {
  constructor() {
    this.reportPath = process.env.GWT_HEADED_E2E_REPORT;
    this.result = { chromium_dark_passed: 0, chromium_light_passed: 0, failed: 0, status: "interrupted" };
    this.tests = [];
    this.workerHeaded = new Map();
    this.attachmentIndex = 0;
  }

  onBegin(config) {
    this.outputDir = config.projects[0]?.outputDir;
    // Playwright starts its workers after onBegin. Preserve existing preloads.
    this.nodeOptions = process.env.NODE_OPTIONS;
    this.moduleRoot = process.env.GWT_HEADED_E2E_MODULE_ROOT;
    const preload = path.join(__dirname, "headed_e2e_runtime.cjs");
    try {
      process.env.GWT_HEADED_E2E_MODULE_ROOT = path.dirname(require.resolve("playwright/test", {
        paths: [path.dirname(process.argv[1] || process.cwd()), process.cwd()],
      }));
    } catch { /* The trace decoder can still supply legacy runtime proof. */ }
    process.env.NODE_OPTIONS = `${this.nodeOptions || ""} --require ${JSON.stringify(preload)}`.trim();
  }

  onTestEnd(test, result) {
    if (["failed", "timedOut", "interrupted"].includes(result.status)) this.result.failed += 1;
    for (const attachment of result.attachments || []) {
      let artifact;
      try {
        if (attachment.path) {
          artifact = path.resolve(attachment.path);
          if (!fs.statSync(artifact).isFile()) continue;
        } else if (this.outputDir && attachment.contentType === "image/png" && attachment.body) {
          const directory = path.resolve(this.outputDir, "gwt-attachments");
          fs.mkdirSync(directory, { recursive: true });
          artifact = path.join(directory, `attachment-${this.attachmentIndex++}.png`);
          fs.writeFileSync(artifact, attachment.body);
        } else {
          continue;
        }
      } catch {
        continue;
      }
      (this.result.artifacts ??= []).push(artifact);
    }
    this.tests.push({ testId: test.id, result });
  }

  onError() {
    this.result.failed += 1;
  }

  async traceEvents(result) {
    const attachment = result.attachments?.find(item => item.name === "trace" && item.path);
    if (!attachment) return [];
    // Resolve the package used by the running CLI, including script wrappers.
    const paths = [path.dirname(process.argv[1] || process.cwd()), process.cwd()];
    let ZipFile;
    for (const module of ["playwright-core/lib/utils", "playwright-core/lib/coreBundle"]) {
      try {
        const exports = require(require.resolve(module, { paths }));
        ZipFile = exports.ZipFile || exports.utils?.ZipFile;
        if (ZipFile) break;
      } catch { /* Try the running Playwright's other layout. */ }
    }
    if (!ZipFile) throw new Error("Playwright ZIP reader unavailable");
    const archive = new ZipFile(attachment.path);
    try {
      const events = [];
      for (const entry of await archive.entries()) {
        if (entry.endsWith(".trace")) {
          const lines = (await archive.read(entry)).toString("utf8").split("\n").filter(Boolean);
          for (const line of lines) events.push(JSON.parse(line));
        }
      }
      return events;
    } finally {
      archive.close();
    }
  }

  traceThemes(result, events) {
    const headers = events.filter(event => event.type === "context-options");
    // Decode the fields we use, rather than pinning a trace version number.
    if (!headers.length) {
      this.workerHeaded.delete(result.workerIndex);
      return new Set();
    }
    const launches = events.filter(event => event.type === "before" &&
      (["browserType.launch", "browserType.launchPersistentContext", "browserType.connect", "browserType.connectOverCDP"].includes(event.apiName) ||
        (event.class === "Test" && event.method === "pw:api" && event.title === "Launch browser")));
    if (launches.length) {
      const completed = new Set(events.filter(event => event.type === "after" && !event.error).map(event => event.callId));
      this.workerHeaded.set(result.workerIndex, launches.every(event =>
        completed.has(event.callId) && (event.params?.headless === "false" || event.params?.headless === false)));
    }
    if (result.status !== "passed" || this.workerHeaded.get(result.workerIndex) !== true) return new Set();
    return new Set(headers.filter(header => header.origin === "library" &&
      header.browserName === "chromium").map(header => header.options?.colorScheme));
  }

  runtimeContexts(testId, result) {
    const file = `${this.reportPath}.worker-${result.workerIndex}.jsonl`;
    if (!fs.existsSync(file)) return [];
    return fs.readFileSync(file, "utf8").trim().split("\n").filter(Boolean).map(JSON.parse)
      .filter(record => record.testId === testId && record.retry === result.retry);
  }

  diagnostic(reason) {
    if ((this.result.diagnostics ??= []).includes(reason)) return;
    this.result.diagnostics.push(reason);
    console.error(`gwt headed evidence: ${reason}`);
  }

  async onEnd(result) {
    for (const { testId, result: test } of this.tests) {
      let themes = new Set();
      try {
        themes = this.traceThemes(test, await this.traceEvents(test));
      } catch (error) {
        this.workerHeaded.delete(test.workerIndex);
        this.diagnostic(`trace unavailable (${error.code || error.name}); trying worker measurement fallback`);
      }
      let contexts = [];
      try {
        contexts = this.runtimeContexts(testId, test);
      } catch (error) {
        this.diagnostic(`worker measurement unreadable (${error.code || error.name})`);
      }
      if (contexts.length) {
        if (!themes.size) this.diagnostic("trace has no headed launch proof; using measured worker context fallback");
        themes = new Set(contexts.filter(context => context.browserName === "chromium" &&
          context.headless === false).map(context => context.colorScheme));
      } else if (!themes.size && test.status === "passed") {
        this.diagnostic("no headed Chromium proof in trace or worker measurement; test not counted");
      }
      if (test.status !== "passed") continue;
      if (themes.has("dark")) this.result.chromium_dark_passed += 1;
      if (themes.has("light")) this.result.chromium_light_passed += 1;
    }
    if (this.nodeOptions === undefined) delete process.env.NODE_OPTIONS;
    else process.env.NODE_OPTIONS = this.nodeOptions;
    if (this.moduleRoot === undefined) delete process.env.GWT_HEADED_E2E_MODULE_ROOT;
    else process.env.GWT_HEADED_E2E_MODULE_ROOT = this.moduleRoot;
    this.result.status = result.status;
    fs.writeFileSync(this.reportPath, JSON.stringify(this.result));
  }
};
