import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { basename, dirname, join } from "node:path";
import { expect, test, type APIRequestContext, type Page } from "@playwright/test";
import { readProcessState, startExactRelaunchFixture } from "./_helpers/exact-relaunch";
import { gotoLiveGwt } from "./_helpers/live-gwt";

type Fixture = Awaited<ReturnType<typeof startExactRelaunchFixture>>;

async function requestStagedUpdate(fixture: Fixture, page: Page, request: APIRequestContext) {
  const state = join(fixture.home, ".gwt");
  const run = join(state, "run");
  const locks = (await readdir(run)).filter(name => /^tray-.*\.lock$/.test(name));
  expect(locks).toHaveLength(1);
  const resident = JSON.parse(await readFile(join(run, locks[0]), "utf8"));
  expect(resident.pid).toBe(fixture.ownedPid);
  const stage = join(fixture.home, "staged");
  // Refuse the helper's first write, even if a regression passes the daemon gate.
  await mkdir(join(stage, "restart-args.json"), { recursive: true });
  const payload = join(stage, "payload");
  await writeFile(payload, "fixture payload");
  const pending = join(state, "pending-update");
  await mkdir(pending);
  const manifest = JSON.stringify({
    version: "999.0.1",
    asset_url: "https://example.invalid/update.zip",
    payload: { PortableBinary: { path: payload } },
    downloaded_at: new Date().toISOString(),
    origin: "manual_later",
    auto_apply_deferred: true,
  });
  const manifestPath = join(pending, "manifest.json");
  await writeFile(manifestPath, manifest);
  const response = await request.post(new URL("/internal/update/apply", fixture.url).href, {
    headers: { Authorization: `Bearer ${resident.control_token}` },
    data: { version: "999.0.1" },
  });
  expect(response.status()).toBe(202);
  const reason = await page.waitForFunction(() => (window as any).__gwtPlaywrightMessages.find(
    (entry: any) => entry.payload.kind === "update_apply_error")?.payload.reason,
  ).then(handle => handle.jsonValue());
  return { reason, manifest, manifestPath };
}

test.skip(process.platform === "win32" || !process.env.GWT_PLAYWRIGHT_EXACT_RELAUNCH,
  "requires the isolated checkout fixture and flock");
test.setTimeout(90_000);

test("update stays staged when the project's daemon lease cannot be retired", async ({ page, request }, info) => {
  const errors: string[] = [];
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  page.on("pageerror", error => errors.push(error.message));

  let holder: ReturnType<typeof spawn> | undefined;
  let fixture: Fixture | undefined;
  try {
    fixture = await startExactRelaunchFixture(info, {
      page,
      async beforeStart({ preferences, env }) {
        // Hold the exact kernel lock in the fresh HOME, with no daemon descriptor.
        holder = spawn("python3", ["-c", [
          "import fcntl, sys",
          "with open(sys.argv[1], 'a') as lock:",
          "    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)",
          "    print('locked', flush=True)",
          "    sys.stdin.read()",
        ].join("\n"), join(preferences, "issue-monitor.authority.lock")], { env });
        const ready = await Promise.race([
          once(holder.stdout!, "data").then(([data]) => String(data).trim()),
          once(holder, "exit").then(([code]) => `holder exited: ${code}`),
        ]);
        expect(ready).toBe("locked");
      },
    });
    await gotoLiveGwt(page, fixture.url, {
      enableTestBridge: true, projectKey: basename(dirname(fixture.preferences)),
    });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const { reason, manifest, manifestPath } = await requestStagedUpdate(fixture, page, request);
    expect(reason).toContain("authority lease");
    await expect(page.locator("#update-cta")).toHaveAttribute("data-status", "error");
    await expect(page.locator("#update-cta")).toContainText("authority lease");
    expect(await readFile(manifestPath, "utf8")).toBe(manifest);
    expect(holder!.exitCode).toBeNull();
    expect((await request.head(fixture.url)).ok()).toBe(true);
    expect(errors).toEqual([]);
  } finally {
    if (holder?.exitCode === null && holder.signalCode === null) {
      const exited = once(holder, "exit");
      holder.stdin?.end();
      await exited;
    }
    await page.close();
    await fixture?.stop();
  }
});

test("a responsive GUI-managed daemon exits before the update helper is prepared", async ({ page, request }, info) => {
  const errors: string[] = [];
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  page.on("pageerror", error => errors.push(error.message));
  const fixture = await startExactRelaunchFixture(info, { page });
  try {
    await gotoLiveGwt(page, fixture.url, {
      enableTestBridge: true, projectKey: basename(dirname(fixture.preferences)),
    });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const directory = join(dirname(fixture.preferences), "runtime", "daemon");
    let owner: { pid: number; birth: string } | undefined;
    await expect(async () => {
      const descriptor = (await readdir(directory)).find(name => name.endsWith(".json"));
      expect(descriptor).toBeDefined();
      const endpoint = JSON.parse(await readFile(join(directory, descriptor!), "utf8"));
      const birth = (await readProcessState(endpoint.pid))?.birth;
      expect(birth).toBeDefined();
      owner = { pid: endpoint.pid, birth: birth! };
    }).toPass({ timeout: 20_000 });

    const { reason } = await requestStagedUpdate(fixture, page, request);
    expect(reason).toContain("Failed to write update restart args");
    expect((await readProcessState(owner!.pid))?.birth).not.toBe(owner!.birth);
    expect(errors).toEqual([]);
  } finally {
    await page.close();
    await fixture.stop();
  }
});
