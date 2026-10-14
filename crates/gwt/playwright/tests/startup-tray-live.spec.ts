/** Issue #4738: the browser front door remains usable after native startup.
 * Native menu dispatch and its 600-second budget are measured separately by
 * startup_tray_performance; this check covers the real rendered front door.
 */
import { expect, test } from "@playwright/test";
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { gotoLiveGwt } from "./_helpers/live-gwt";

test.skip(process.env.GWT_PLAYWRIGHT_STARTUP_TRAY !== "1", "explicit checkout binaries required for this live startup check");

const checkout = process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT!;
const extension = process.platform === "win32" ? ".exe" : "";
let child: ChildProcess | undefined;
let anchor: WebSocket | undefined;
let base = "";
let output = "";

test.beforeAll(async () => {
  test.setTimeout(120_000);
  expect(checkout, "the runner must identify this checkout").toBeTruthy();
  const localHook = /(^|[/\\])target[/\\](?:[^/\\]+[/\\])*(debug|release)[/\\]gwtd(\.exe)?$|[/\\]verification-drivers[/\\]/i;
  const hookBin = [process.env.GWT_HOOK_BIN, process.env.GWT_BIN_PATH]
    .find(candidate => candidate && (candidate === "gwtd" || (!localHook.test(candidate) && existsSync(candidate)))) ?? "gwtd";
  const home = mkdtempSync(join(tmpdir(), "gwt-startup-tray-"));
  for (const directory of [".gwt", ".codex", ".claude", "tmp"]) {
    mkdirSync(join(home, directory), { recursive: true });
  }
  writeFileSync(join(home, ".codex/config.toml"), "");
  writeFileSync(join(home, ".claude/settings.json"), "{}");
  writeFileSync(join(home, ".gwt/config.toml"), "[usage]\ncodex_enabled = false\nclaude_account_enabled = false\n");
  const runtime = join(homedir(), ".gwt/runtime");
  if (existsSync(runtime)) symlinkSync(runtime, join(home, ".gwt/runtime"), "junction");
  const environment = { ...process.env };
  for (const key of Object.keys(environment)) {
    if (/^(GWT_|CODEX_|CLAUDE_)/.test(key)) delete environment[key];
  }
  Object.assign(environment, {
    HOME: home, USERPROFILE: home, TEMP: join(home, "tmp"), TMP: join(home, "tmp"),
    APPDATA: join(home, "AppData/Roaming"), LOCALAPPDATA: join(home, "AppData/Local"),
    GWT_HOOK_BIN: hookBin, GWT_DISABLE_BACKGROUND_INDEX: "1",
    GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1",
    GWT_BROWSER_URL_FILE: join(home, "url"),
  });
  const daemon = (operation: string, params: object) => JSON.parse(execFileSync(
    join(checkout, `target/debug/gwtd${extension}`), [], {
      cwd: checkout, env: environment, windowsHide: true, encoding: "utf8", timeout: 60_000,
      input: JSON.stringify({ schema_version: 1, operation, params }),
    },
  ));
  const status = daemon("issue.monitor.status", {});
  const hash = status.project_store.hash;
  expect(hash).toMatch(/^[a-zA-Z0-9_-]+$/);
  const state = join(home, ".gwt/projects", hash, "project-state");
  mkdirSync(state, { recursive: true });
  writeFileSync(join(state, "pm.json"), JSON.stringify({ settings: { auto_start: false } }));
  writeFileSync(join(state, "issue-monitor.json"), JSON.stringify({ enabled: false }));
  writeFileSync(join(home, ".gwt/session.json"), JSON.stringify({
    tabs: [{ id: "startup", title: "Startup", project_root: checkout, kind: "git" }], recent_projects: [],
  }));
  const hookParams = { expected_hook_bin: hookBin, runtime_state_path: join(home, ".gwt/missing-runtime.json") };
  expect(daemon("hook.doctor", { ...hookParams, repair: true }).ok).toBe(true);
  child = spawn(join(checkout, `target/debug/gwt${extension}`), ["--no-tray", "--no-open"], {
    cwd: checkout, env: environment, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
  });
  child.stdout!.on("data", data => { output += data; });
  child.stderr!.on("data", data => { output += data; });
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    expect(child.exitCode, output).toBeNull();
    if (existsSync(join(home, "url"))) {
      const candidate = readFileSync(join(home, "url"), "utf8").trim();
      if (candidate) {
        try {
          if ((await fetch(candidate, { method: "HEAD", signal: AbortSignal.timeout(2_000) })).status === 200) {
            base = candidate;
            break;
          }
        } catch { /* wait for this owned process's listener */ }
      }
    }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  expect(base, output).not.toBe("");
  // Keep the no-tray server alive across the helper's Hub/Project navigation.
  anchor = new WebSocket(new URL("/ws", base).toString().replace(/^http/, "ws"));
  await new Promise<void>((resolve, reject) => {
    anchor!.addEventListener("open", () => resolve(), { once: true });
    anchor!.addEventListener("error", () => reject(new Error("owned backend anchor failed")), { once: true });
  });
  const audit = daemon("hook.health", hookParams);
  expect(audit.ok).toBe(true);
  const health = JSON.parse(audit.output);
  expect(health.status).not.toBe("inactive");
  expect(health.issues.filter((issue: string) => !issue.startsWith("managed hook failure: ") || !issue.includes(" state=fail-open"))).toEqual([]);
  console.log(`Owned startup checkout PID ${child.pid}; HOME ${home}`);
});

test.afterAll(async () => {
  anchor?.close();
  if (child && child.exitCode === null && child.signalCode === null) {
    const exited = new Promise<void>(resolve => child!.once("exit", () => resolve()));
    child.kill();
    await Promise.race([exited, new Promise(resolve => setTimeout(resolve, 10_000))]);
    expect(child.exitCode !== null || child.signalCode !== null, "owned checkout must stop").toBe(true);
  }
});

test("startup tray browser front door accepts input without page errors", async ({ page }, info) => {
  const errors: string[] = [];
  const watch = (observed: typeof page) => {
    observed.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    observed.on("pageerror", error => errors.push(String(error)));
  };
  watch(page);
  page.context().on("page", watch);
  await gotoLiveGwt(page, base, { suppressProjectSurfaces: true });
  await page.evaluate(theme => { document.documentElement.dataset.theme = theme; },
    info.project.name.includes("light") ? "light" : "dark");
  await expect(page.locator(".op-rail")).toBeVisible();
  await page.locator("#op-palette-button").click();
  await expect(page.locator("#op-palette-backdrop")).toHaveAttribute("data-open", "true");
  await page.keyboard.press("Escape");
  await expect(page.locator("#op-palette-backdrop")).not.toHaveAttribute("data-open", "true");
  expect(errors).toEqual([]);
  const screenshot = info.outputPath("startup-browser.png");
  await page.screenshot({ path: screenshot });
  await info.attach("startup-browser", { path: screenshot, contentType: "image/png" });
});
