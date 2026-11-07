// Preloaded in Playwright child processes. Measure successful launches and pages;
// project.use is insufficient because test fixtures can override it.
const fs = require("node:fs");
const path = require("node:path");

if (process.env.GWT_HEADED_E2E_REPORT) {
  // Recent Playwright assigns TEST_WORKER_INDEX after Node preloads execute.
  const append = value => {
    if (process.env.TEST_WORKER_INDEX === undefined) return;
    const output = `${process.env.GWT_HEADED_E2E_REPORT}.worker-${process.env.TEST_WORKER_INDEX}.jsonl`;
    fs.appendFileSync(output, JSON.stringify(value) + "\n");
  };
  try {
    const paths = [process.env.GWT_HEADED_E2E_MODULE_ROOT, path.dirname(process.argv[1] || process.cwd()), process.cwd()].filter(Boolean);
    const { chromium } = require(require.resolve("playwright-core", { paths }));
    const { test } = require(require.resolve("playwright/test", { paths }));
    const instrumentContext = (context, browser) => {
      const newPage = context.newPage;
      context.newPage = async function (...args) {
        const page = await newPage.apply(this, args);
        try {
          const info = test.info();
          append({
            testId: info.testId, retry: info.retry,
            browserName: "chromium", headless: browser?._options?.headless,
            colorScheme: context._options?.colorScheme,
          });
        } catch { /* No current test or unreadable measurement supplies no proof. */ }
        return page;
      };
      return context;
    };
    const launch = chromium.launch;
    chromium.launch = async function (...args) {
      const browser = await launch.apply(this, args);
      // These are the merged options of the browser that actually launched.
      // An omitted/unknown headless value supplies no headed proof.
      for (const method of ["newContext", "_newContextForReuse"]) {
        if (typeof browser[method] !== "function") continue;
        const original = browser[method];
        browser[method] = async function (...options) {
          return instrumentContext(await original.apply(this, options), browser);
        };
      }
      return browser;
    };
    const persistent = chromium.launchPersistentContext;
    chromium.launchPersistentContext = async function (...args) {
      const context = await persistent.apply(this, args);
      return instrumentContext(context, context.browser());
    };
  } catch (error) {
    append({ error: `worker measurement unavailable (${error.code || error.name})` });
  }
}
