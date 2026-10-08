/* Issue #3775 — the status strip PERF cell showed the multi-core CPU total
 * verbatim, so a busy fleet rendered as "CPU 501%" and read as a defect.
 *
 * The aggregate is now normalized to a host share and clamped to 0–100%,
 * while per-process rows keep the 1 core = 100% convention. This spec drives
 * the real browser through the embedded frontend (`installEmbeddedRoutes`)
 * so the clamp, the per-process exemption, and the units disclosure are all
 * observed as rendered pixels rather than as DOM-shim assertions. Playwright
 * runs it once per theme project (chromium-dark / chromium-light).
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const CPU_UNITS =
  "Aggregate CPU is logical-core-normalized host share (0–100%); process rows use 1 core = 100%.";

// A legacy aggregate payload: 501.2% is the sum across cores, well past the
// host-share ceiling. The `gwt` row carries 118.4% because one process may
// legitimately exceed a single core.
const LEGACY_AGGREGATE_SNAPSHOT = {
  state: "hot",
  cpu_percent: 501.2,
  memory_bytes: 5 * 1024 * 1024 * 1024,
  process_count: 3,
  runner_count: 1,
  dropped_lossy_delta: 0,
  queue: { client_count: 2, queued: 7, dropped_lossy: 1 },
  processes: [
    {
      pid: 100,
      parent_pid: null,
      role: "gwt",
      name: "cpu-burn",
      cpu_percent: 118.4,
      memory_bytes: 3 * 1024 * 1024 * 1024,
    },
    {
      pid: 101,
      parent_pid: 100,
      role: "runner",
      name: "chroma_index_runner",
      cpu_percent: 18.4,
      memory_bytes: 512 * 1024 * 1024,
    },
    {
      pid: 102,
      parent_pid: 100,
      role: "docker",
      name: "docker",
      cpu_percent: 2.1,
      memory_bytes: 64 * 1024 * 1024,
    },
  ],
};

test.describe("Runtime health CPU normalization", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  let pageProblems: string[] = [];

  test.beforeEach(async ({ page }) => {
    pageProblems = [];
    page.on("console", (message) => {
      if (message.type() === "error") {
        pageProblems.push(`console: ${message.text()}`);
      }
    });
    page.on("pageerror", (error) => pageProblems.push(`page: ${error.message}`));

    await installEmbeddedRoutes(page);
    await installRuntimeHealthBackend(page);
    await page.goto(APP_URL);
    await page.waitForFunction(() =>
      Boolean((window as any).__operatorShell?.applyRuntimeHealth),
    );
    await applyLegacyAggregate(page);
  });

  test.afterEach(() => {
    expect(pageProblems).toEqual([]);
  });

  test("the compact PERF value clamps a multi-core total to the host share", async ({
    page,
  }) => {
    const cell = page.locator("#op-strip-runtime-health");
    await expect(page.locator("#op-strip-runtime-health-value")).toHaveText(
      "HOT 100% 5.0G",
    );
    // The raw total must not survive anywhere in the compact cell.
    await expect(cell).not.toContainText("501");
    await expect(cell).toHaveAttribute("title", new RegExp(escapeForRegExp(CPU_UNITS)));
    await expect(cell).toHaveAttribute(
      "aria-label",
      new RegExp(escapeForRegExp(CPU_UNITS)),
    );
  });

  test("the hover detail states the units and leaves process rows unclamped", async ({
    page,
  }) => {
    await page.locator("#op-strip-runtime-health").hover();

    const detail = page.locator("#op-runtime-health-detail");
    await expect(detail).toBeVisible();
    await expect(detail.locator(".op-runtime-health-detail__units")).toHaveText(
      CPU_UNITS,
    );

    const summary = detail.locator(".op-runtime-health-detail__summary");
    await expect(summary).toContainText("100%");
    await expect(summary).not.toContainText("501");

    // 118.4% is one process using more than a single core. Clamping it would
    // hide the actual hot process the detail panel exists to surface.
    const metrics = detail.locator(".op-runtime-health-detail__process-metric");
    await expect(metrics.first()).toHaveText("118%");
  });
});

function escapeForRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

async function applyLegacyAggregate(page: any) {
  await page.evaluate((snapshot: unknown) => {
    const shell = (window as any).__operatorShell;
    const render = shell.applyRuntimeHealth;
    render(snapshot);
    // A live runtime_health event would repaint the cell mid-assertion. Pin
    // the fixture snapshot so the assertions read what this test rendered.
    shell.applyRuntimeHealth = () => render(snapshot);
  }, LEGACY_AGGREGATE_SNAPSHOT);
}

async function installRuntimeHealthBackend(page: any) {
  await page.addInitScript(() => {
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Runtime Health Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(workspaceState);
        }, 0);
      }

      // The runtime-health snapshot under test is injected by the spec, so the
      // fixture only has to accept and drop whatever the frontend sends.
      send() {}

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload) {
        setTimeout(() => {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }, 0);
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}
