// Auxiliary shell-path measurement only; this does not replace PM/agent checks.
// Usage: node scripts/diagnostics/measure-input-4172.cjs <isolated-checkout-url> <output-dir>
// Requires Playwright 1.49.1 and Chromium installed under the temporary
// gwt-playwright-1.49.1 directory used by this diagnostic run.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { chromium } = require(path.join(os.tmpdir(), 'gwt-playwright-1.49.1/node_modules/playwright'));

const [url, outputArg] = process.argv.slice(2);
if (!url || !outputArg || !['127.0.0.1', 'localhost'].includes(new URL(url).hostname)) {
  throw new Error('Supply the fresh isolated checkout URL and an output directory.');
}
const output = path.resolve(outputArg);
fs.mkdirSync(output, { recursive: true });
const result = {
  url,
  startedAt: new Date().toISOString(),
  scope: 'Auxiliary real WebSocket shell input-to-echo measurement; not a replacement for PM/agent live verification.',
  measurement: 'Browser performance.now from terminal_input dispatch to an exact echo output line in xterm buffer; requestAnimationFrame polling.',
  samples: [], themes: [], console: [], pageErrors: [], cleanupErrors: [],
};
let browser;
let page;
const created = [];
let baseline = [];
let baselineCaptured = false;
const save = () => fs.writeFileSync(path.join(output, 'result.json'), JSON.stringify(result, null, 2));

async function send(message) {
  await page.evaluate(detail => {
    window.dispatchEvent(new CustomEvent('__gwt_test_send', { detail }));
  }, message);
}

async function windowIds() {
  return page.locator('.workspace-window').evaluateAll(nodes => nodes.map(node => node.dataset.id));
}

async function createShell() {
  const before = await windowIds();
  await send({ kind: 'create_window', preset: 'shell', bounds: { x: 100, y: 100, width: 760, height: 440 } });
  const handle = await page.waitForFunction(ids => {
    const node = [...document.querySelectorAll('.workspace-window')].find(node =>
      !ids.includes(node.dataset.id) && node.dataset.preset === 'shell' && node.querySelector('.terminal-root'));
    return node?.dataset.id || false;
  }, before, { timeout: 120000 });
  const id = await handle.jsonValue();
  await handle.dispose();
  created.push(id);
  await page.waitForFunction(id => window.__gwtTerminalTestApi.metrics(id).isReady === true,
    id, { timeout: 120000 });
  // A warm-up proves the actual shell is ready, beyond the xterm renderer.
  await measureEcho(id, `GWT_READY_${created.length}_${Date.now()}`);
  return id;
}

async function measureEcho(id, marker) {
  return page.evaluate(({ id, marker }) => new Promise((resolve, reject) => {
    const started = performance.now();
    let frame;
    const timeout = setTimeout(() => {
      cancelAnimationFrame(frame);
      reject(new Error(`Shell echo timed out: ${id}`));
    }, 120000);
    const poll = () => {
      // Match only a complete output line, never the echoed command/prompt.
      const lines = window.__gwtTerminalTestApi.bufferText(id).split(/\r?\n/);
      if (lines.some(line => line.trim() === marker)) {
        clearTimeout(timeout);
        resolve(performance.now() - started);
      } else {
        frame = requestAnimationFrame(poll);
      }
    };
    window.dispatchEvent(new CustomEvent('__gwt_test_send', {
      detail: { kind: 'terminal_input', id, data: `echo ${marker}\r` },
    }));
    poll();
  }), { id, marker });
}

(async () => {
  try {
    browser = await chromium.launch({ headless: false });
    const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } });
    await context.addInitScript(() => { window.__gwtPlaywrightTestBridge = true; });
    page = await context.newPage();
    page.setDefaultTimeout(120000);
    page.on('console', message => result.console.push({ type: message.type(), text: message.text() }));
    page.on('pageerror', error => result.pageErrors.push(String(error)));
    await page.goto(url, { waitUntil: 'domcontentloaded' });
    await page.waitForFunction(() => window.__gwtPlaywrightTestBridgeInstalled === true);
    // Real click actionability must wait for startup to finish. Never hide overlays.
    await page.locator('#op-theme-toggle [data-theme-value="dark"]').click();
    baseline = await windowIds();
    baselineCaptured = true;
    result.baselineWindowCount = baseline.length;

    for (const count of [1, 8, 24]) {
      while (created.length < count) await createShell();
      for (const theme of ['dark', 'light']) {
        const clickStarted = performance.now();
        await page.locator(`#op-theme-toggle [data-theme-value="${theme}"]`).click();
        await page.waitForFunction(theme =>
          document.documentElement.getAttribute('data-theme') === theme &&
          document.querySelector(`#op-theme-toggle [data-theme-value="${theme}"]`)
            ?.getAttribute('aria-checked') === 'true', theme);
        const themeSample = { theme, shellCount: count, clickAndConfirmationMs: performance.now() - clickStarted };
        const screenshot = `${count}-shells-${theme}.png`;
        await page.screenshot({ path: path.join(output, screenshot), fullPage: true });
        result.themes.push({ ...themeSample, screenshot });
        for (let trial = 1; trial <= 3; trial++) {
          const id = created[created.length - 1];
          const marker = `GWT_${count}_${theme}_${trial}_${Date.now()}`;
          const elapsedMs = await measureEcho(id, marker);
          result.samples.push({ theme, shellCount: count, trial, windowId: id, elapsedMs });
          save();
        }
      }
    }
    result.status = result.pageErrors.length || result.console.some(entry => entry.type === 'error')
      ? 'failed-console-or-page-error' : 'measured';
  } catch (error) {
    result.status = 'failed';
    result.error = String(error.stack || error);
    process.exitCode = 1;
  } finally {
    if (page && !page.isClosed() && baselineCaptured) {
      // Also catches a created shell whose readiness wait failed before registration.
      const owned = new Set(created);
      try {
        const shells = await page.locator('.workspace-window[data-preset="shell"]')
          .evaluateAll(nodes => nodes.map(node => node.dataset.id));
        for (const id of shells) if (!baseline.includes(id)) owned.add(id);
        for (const id of owned) await send({ kind: 'close_window', id });
        await page.waitForFunction(ids => ![...document.querySelectorAll('.workspace-window')]
          .some(node => ids.includes(node.dataset.id)), [...owned], { timeout: 120000 });
      } catch (error) {
        result.cleanupErrors.push(String(error));
        process.exitCode = 1;
      }
    }
    if (browser) await browser.close().catch(error => result.cleanupErrors.push(String(error)));
    result.finishedAt = new Date().toISOString();
    if (result.status !== 'measured' || result.cleanupErrors.length) process.exitCode = 1;
    save();
    console.log(path.join(output, 'result.json'));
  }
})();
