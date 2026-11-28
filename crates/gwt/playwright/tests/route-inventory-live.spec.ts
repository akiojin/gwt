// SPEC #3820 T-006d: drive the production routes restored by Issue #8040 and
// collect their perf-log samples for inventory paths P03 (project switch),
// P04 (pane create, the synchronous Shell process side) and P06 (prompt send).
// This is a measurement driver, not a regression gate. It runs only against an
// isolated browser-check instance and writes raw samples to
// GWT_PERF_INVENTORY_OUT. See docs/perf/issue-3820/README.md.
import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readdir, readFile, realpath, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { gotoLiveGwt, liveGwtProjectUrl, readLiveHubCatalog, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const SAMPLES = Number(process.env.GWT_PERF_INVENTORY_SAMPLES ?? "10");
test.use({ launchOptions: { args: ["--window-position=-32000,-32000", "--disable-backgrounding-occluded-windows",
  "--disable-renderer-backgrounding", "--disable-background-timer-throttling"] } });

test.describe("SPEC #3820 route inventory", () => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  const out = process.env.GWT_PERF_INVENTORY_OUT;
  test.skip(!base || !home || !out, "requires an isolated browser-check instance and GWT_PERF_INVENTORY_OUT");
  test.setTimeout(1_200_000);
  test.use({ viewport: { width: 1440, height: 900 } });

  test("P03 / P04 / P06 production route samples", async ({ page }, info) => {
    await withLiveGwtBackendLock(base!, info, async () => {
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      const keys = [await openFixture(page, base!, home!, "a"), await openFixture(page, base!, home!, "b")];
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const created: string[] = [];
      try {
        await page.bringToFront();
        await gotoLiveGwt(page, base!, { projectKey: keys[0], enableTestBridge: true });
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await waitOwnerHealthy(page);

        // P04: every Shell creation is one route:pane.create sample.
        const p04From = new Date();
        for (let index = 0; index < SAMPLES; index += 1) {
          created.push(await createShell(page));
        }
        const p04 = await routeSamples(home!, "route:pane.create", p04From, SAMPLES);

        // P06: every submit (Enter) reaching the Shell PTY is one route:prompt.send sample.
        const shell = created[created.length - 1];
        // Let the 15 s owner attach grace pass so input reaches the resident owner.
        await page.waitForTimeout(16_000);
        const p06From = new Date();
        for (let index = 0; index < SAMPLES; index += 1) {
          await sendLiveGwtEvent(page, { kind: "terminal_input", id: shell, data: `echo p06-${index}\r` });
          await page.waitForTimeout(300);
        }
        const p06 = await routeSamples(home!, "route:prompt.send", p06From, SAMPLES);

        // P03: navigating between the two Project routes is a project switch; the
        // frontend reports navigation start -> first workspace frame.
        const p03From = new Date();
        for (let index = 0; index < SAMPLES; index += 1) {
          const target = keys[(index + 1) % 2];
          await page.goto(liveGwtProjectUrl(base!, target), { referer: page.url() });
          // The frontend itself reports navigation start -> first workspace frame.
          await page.waitForLoadState("load");
          await page.waitForTimeout(1_500);
        }
        const p03 = await routeSamples(home!, "route:project.switch", p03From, SAMPLES);

        expect(errors, "console and page errors").toEqual([]);
        const record = { theme, samples_per_path: SAMPLES, recorded_at: new Date().toISOString(),
          P03: p03, P04: p04, P06: p06, console_errors: errors };
        await mkdir(out!, { recursive: true });
        await writeFile(join(out!, `routes-${theme}.json`), JSON.stringify(record, null, 2) + "\n");
        await info.attach(`routes-${theme}`, { body: JSON.stringify(record), contentType: "application/json" });
      } finally {
        await gotoLiveGwt(page, base!, { projectKey: keys[0], enableTestBridge: true }).catch(() => {});
        for (const id of created) await sendLiveGwtEvent(page, { kind: "close_window", id }).catch(() => {});
        for (const key of keys) {
          await readLiveHubCatalog(page, base!, {
            send: { kind: "preview_close_project", project_key: key, project_wide: true },
          }).catch(() => {});
        }
      }
    });
  });
});

async function openFixture(page: Page, base: string, home: string, label: string): Promise<string> {
  const fixture = await realpath(await mkdtemp(join(home, `perf-3820-route-${label}-`)));
  const source = join(fixture, "source");
  const root = join(fixture, "project");
  const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith("GWT_")));
  Object.assign(env, { HOME: home, USERPROFILE: home, GIT_CONFIG_NOSYSTEM: "1" });
  execFileSync("git", ["init", "-q", source], { env });
  execFileSync("git", ["-C", source, "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
    "commit", "--allow-empty", "-qm", "Perf fixture"], { env });
  execFileSync("git", ["-C", source, "worktree", "add", "--detach", "-q", root], { env });
  const gwtd = join(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT!, "target", "debug", "gwtd");
  const status = JSON.parse(execFileSync(gwtd, { cwd: root, env, encoding: "utf8", input: JSON.stringify({
    schema_version: 1, operation: "issue.monitor.status", params: {},
  }) + "\n" }));
  const key: string = status.project_store?.hash;
  expect(key).toMatch(/^[a-f0-9]{16}$/);
  const projectState = join(home, ".gwt", "projects", key, "project-state");
  await mkdir(projectState, { recursive: true });
  await writeFile(join(projectState, "pm.json"), '{"settings":{"auto_start":false}}');
  await writeFile(join(projectState, "issue-monitor.json"), '{"enabled":false,"max_active_agents":1,"priority_order":[]}');
  await readLiveHubCatalog(page, base, {
    send: { kind: "reopen_recent_project", path: root },
    until: (catalog, expected) => catalog.projects.some(project => project.project_key === expected), arg: key,
  });
  return key;
}

async function waitOwnerHealthy(page: Page) {
  const readiness = await page.waitForFunction(() => (window as any).__gwtPlaywrightMessages
    .find(({ payload }: any) => payload.kind === "migration_detected" ||
      (payload.kind === "project_owner_health" && payload.health.state === "healthy"))?.payload.kind,
  undefined, { timeout: 30_000 }).then(value => value.jsonValue()).catch(() => "owner readiness timed out");
  expect(readiness, "GUI presence (Healthy project owner)").toBe("project_owner_health");
}

async function projectWindows(page: Page): Promise<Array<{ id: string; preset: string }>> {
  return page.evaluate(() => {
    const workspace = (window as any).__gwtPlaywrightMessages.findLast((entry: any) => entry.payload.kind === "workspace_state").payload.workspace;
    return workspace.tabs.flatMap((tab: any) => tab.workspace.windows);
  });
}

async function createShell(page: Page): Promise<string> {
  const before = new Set((await projectWindows(page)).map(window => window.id));
  await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell" });
  await expect.poll(async () => (await projectWindows(page)).filter(window => window.preset === "shell" && !before.has(window.id)),
    { timeout: 30_000 }).toHaveLength(1);
  return (await projectWindows(page)).find(window => window.preset === "shell" && !before.has(window.id))!.id;
}

// Production samples of one route written by the isolated instance since
// `from`. The count is reported beside the number of operations driven rather
// than asserted, so a missing sample is evidence instead of a failed run.
async function routeSamples(home: string, target: string, from: Date, operations: number) {
  const dir = join(home, ".gwt", "logs", "perf");
  const read = async () => {
    const values: number[] = [];
    for (const name of (await readdir(dir).catch(() => [] as string[])).filter(name => name.endsWith(".jsonl"))) {
      for (const line of (await readFile(join(dir, name), "utf8")).split("\n")) {
        if (!line.includes(`"${target}"`)) continue;
        const record = JSON.parse(line);
        if (record.type === "sample" && Date.parse(record.timestamp) >= from.getTime()) values.push(record.value);
      }
    }
    return values;
  };
  let values = await read();
  for (let idle = 0; values.length < operations && idle < 20; idle += 1) {
    await new Promise(resolve => setTimeout(resolve, 1_000));
    const next = await read();
    if (next.length > values.length) idle = 0;
    values = next;
  }
  const sorted = [...values].sort((a, b) => a - b);
  const rank = (q: number) => sorted.length ? sorted[Math.max(0, Math.ceil(q * sorted.length) - 1)] : null;
  return { operations, n: values.length, p50_ms: rank(0.5), p95_ms: rank(0.95), max_ms: sorted.length ? sorted[sorted.length - 1] : null,
    samples_ms: values };
}
