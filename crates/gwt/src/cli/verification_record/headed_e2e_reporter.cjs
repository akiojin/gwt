const fs = require("node:fs");
const path = require("node:path");

// Trace API parameters include test.use overrides that project.use omits.
module.exports = class HeadedE2eReporter {
  constructor() {
    this.reportPath = process.env.GWT_HEADED_E2E_REPORT;
    this.result = { chromium_dark_passed: 0, chromium_light_passed: 0, failed: 0, status: "interrupted" };
    this.tests = [];
    this.workerHeaded = new Map();
  }

  onTestEnd(_test, result) {
    if (["failed", "timedOut", "interrupted"].includes(result.status)) this.result.failed += 1;
    this.tests.push(result);
  }

  onError() {
    this.result.failed += 1;
  }

  async traceEvents(result) {
    const attachment = result.attachments?.find(item => item.name === "trace" && item.path);
    if (!attachment) return [];
    // Resolve the package used by the running CLI, including script wrappers.
    const { ZipFile } = require(require.resolve("playwright-core/lib/utils", {
      paths: [path.dirname(process.argv[1] || process.cwd()), process.cwd()],
    }));
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

  recordTrace(result, events) {
    const headers = events.filter(event => event.type === "context-options");
    // Playwright 1.49 uses trace v7. Unknown/missing formats supply no proof.
    if (!headers.length || headers.some(header => header.version !== 7)) {
      this.workerHeaded.delete(result.workerIndex);
      return;
    }
    const launches = events.filter(event => event.type === "before" &&
      ["browserType.launch", "browserType.launchPersistentContext", "browserType.connect", "browserType.connectOverCDP"].includes(event.apiName));
    if (launches.length) {
      const completed = new Set(events.filter(event => event.type === "after" && !event.error).map(event => event.callId));
      this.workerHeaded.set(result.workerIndex, launches.every(event =>
        completed.has(event.callId) && (event.params?.headless === "false" || event.params?.headless === false)));
    }
    if (result.status !== "passed" || this.workerHeaded.get(result.workerIndex) !== true) return;
    const themes = new Set(headers.filter(header => header.origin === "library" &&
      header.browserName === "chromium").map(header => header.options?.colorScheme));
    if (themes.has("dark")) this.result.chromium_dark_passed += 1;
    if (themes.has("light")) this.result.chromium_light_passed += 1;
  }

  async onEnd(result) {
    for (const test of this.tests) {
      try {
        this.recordTrace(test, await this.traceEvents(test));
      } catch {
        // Missing packages, corrupt archives and unreadable traces fail closed.
        this.workerHeaded.delete(test.workerIndex);
      }
    }
    this.result.status = result.status;
    fs.writeFileSync(this.reportPath, JSON.stringify(this.result));
  }
};
