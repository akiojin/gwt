import { expect, test, type WebSocketRoute } from "@playwright/test";
import { gotoLiveGwt, openLiveGwtProject } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";
const CPU_UNITS =
  /Aggregate CPU is logical-core-normalized host share \(0–100%\); process rows use 1 core = 100%\./;
const transports = new WeakMap<object, WebSocketRoute>();
const browserErrors = new WeakMap<object, string[]>();

test.describe("Runtime health sort controls", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test.beforeEach(async ({ page }) => {
    const errors: string[] = [];
    browserErrors.set(page, errors);
    page.on("console", (message) => {
      if (message.type() === "error") {
        const location = message.location();
        const source = location.url
          ? ` @ ${location.url}:${location.lineNumber}:${location.columnNumber}`
          : "";
        errors.push(`console: ${message.text()}${source}`);
      }
    });
    page.on("pageerror", (error) => errors.push(`page: ${error.message}`));
    await page.routeWebSocket(/\/ws(?:\?|$)/, socket => {
      transports.set(page, socket);
      const server = socket.connectToServer();
      server.onMessage(message => {
        const payload = typeof message === "string" ? JSON.parse(message) : null;
        if (payload?.kind !== "runtime_health") socket.send(message);
      });
    });
    await gotoLiveGwt(page, BASE, { suppressProjectSurfaces: true });
    await openLiveGwtProject(page, process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? process.cwd());
    await page.waitForFunction(() => Boolean((window as any).__operatorShell?.applyRuntimeHealth));
  });

  test.afterEach(async ({ page }) => {
    expect(browserErrors.get(page) ?? []).toEqual([]);
  });

  test("sort buttons keep the process detail open after click", async ({ page }, info) => {
    transports.get(page)!.send(JSON.stringify({ kind: "runtime_health", snapshot: {
        state: "hot",
        cpu_percent: 501.2,
        memory_bytes: 5 * 1024 * 1024 * 1024,
        process_count: 3,
        runner_count: 1,
        queue: {
          client_count: 1,
          queued_entries: 0,
          dirty_panes: 0,
          dropped_lossy_delta: 0,
        },
        processes: [
          {
            pid: 701,
            parent_pid: null,
            role: "runner",
            name: "memory-hog",
            cpu_percent: 2,
            memory_bytes: 3 * 1024 * 1024 * 1024,
          },
          {
            pid: 702,
            parent_pid: null,
            role: "gwt",
            name: "cpu-burn",
            cpu_percent: 118.4,
            memory_bytes: 128 * 1024 * 1024,
          },
          {
            pid: 703,
            parent_pid: null,
            role: "gwtd",
            name: "balanced",
            cpu_percent: 20,
            memory_bytes: 768 * 1024 * 1024,
          },
        ],
    } }));

    const perfCell = page.locator("#op-strip-runtime-health");
    const perfValue = page.locator("#op-strip-runtime-health-value");
    const detail = page.locator("#op-runtime-health-detail");

    await expect(perfValue).toHaveText("HOT 100% 5.0G");
    await expect(perfCell).toHaveAttribute("title", CPU_UNITS);
    await expect(perfCell).toHaveAttribute("aria-label", CPU_UNITS);

    await perfCell.hover();
    await expect(detail).toBeVisible();
    await expect(detail.locator(".op-runtime-health-detail__units")).toHaveText(CPU_UNITS);
    await expect(detail).toContainText("118%");
    await expect(detail.locator(".op-runtime-health-detail__process-more")).toContainText(
      "sorted by Load",
    );

    await perfCell.focus();
    await expect(detail).toBeVisible();

    const theme = info.project.name.endsWith("light") ? "light" : "dark";
    await page.locator(`#op-theme-toggle [data-theme-value="${theme}"]`).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await perfCell.focus();
    await expect(detail).toBeVisible();
    await expect(perfValue).toHaveText("HOT 100% 5.0G");

    await detail.getByRole("button", { name: "CPU" }).click();
    await page.waitForTimeout(200);

    await expect(detail).toBeVisible();
    await expect(detail.locator(".op-runtime-health-detail__process-more")).toContainText(
      "sorted by CPU",
    );

    await detail.getByRole("button", { name: "Mem" }).click();
    await page.waitForTimeout(200);

    await expect(detail).toBeVisible();
    await expect(detail.locator(".op-runtime-health-detail__process-more")).toContainText(
      "sorted by Mem",
    );
    transports.get(page)!.send(JSON.stringify({ kind: "runtime_health", snapshot: {
      state: "ok", cpu_percent: 5, memory_bytes: 1024 ** 3, process_count: 1, runner_count: 0,
      processes: [{ pid: 704, role: "gwt", name: "latest-snapshot", cpu_percent: 5, memory_bytes: 1024 ** 3 }],
    } }));
    await expect(perfValue).toHaveText("OK 5% 1.0G");
    await expect(detail).toContainText("latest-snapshot");
    await expect(detail).not.toContainText("memory-hog");
    await expect(detail.locator(".op-runtime-health-detail__process-more")).toContainText("sorted by Mem");
    await perfCell.focus();
    await expect(detail).toBeVisible();
    const burst = await page.evaluate(async () => {
      const value = document.getElementById("op-strip-runtime-health-value")!;
      const before = value.textContent;
      let renders = 0;
      const observer = new MutationObserver(records => { renders += records.length; });
      observer.observe(value, { childList: true });
      try {
        // Exercise synchronous post-dispatch model updates from the served
        // checkout so transport coalescing cannot hide repeated rendering.
        for (let cpu = 1; cpu <= 50; cpu++) {
          (window as any).__operatorShell.applyRuntimeHealth({
            state: "hot", cpu_percent: cpu, memory_bytes: 1024 ** 3,
            processes: [{ pid: 705, role: "gwt", name: `burst-${cpu}`, cpu_percent: cpu, memory_bytes: 1024 ** 3 }],
          });
        }
        const immediate = value.textContent;
        await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        return { before, immediate, renders, latest: value.textContent };
      } finally { observer.disconnect(); }
    });
    expect(burst).toEqual({ before: "OK 5% 1.0G", immediate: "OK 5% 1.0G", renders: 1, latest: "HOT 50% 1.0G" });
    await expect(detail).toContainText("burst-50");
    await expect(detail).not.toContainText("latest-snapshot");
    await expect(detail.locator(".op-runtime-health-detail__process-more")).toContainText("sorted by Mem");
    await perfCell.focus();
    await expect(detail).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await info.attach("runtime-frame-burst", { body: JSON.stringify(burst), contentType: "application/json" });
    const screenshot = await page.screenshot({ fullPage: true });
    await info.attach("runtime-shared-state", { body: screenshot, contentType: "image/png" });
    if (process.env.GWT_PLAYWRIGHT_CHECK_HOME) {
      await page.screenshot({ path: `${process.env.GWT_PLAYWRIGHT_CHECK_HOME}/runtime-${info.project.name}.png`, fullPage: true });
    }
  });
});
