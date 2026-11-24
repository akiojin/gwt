import { expect, test, type Page } from "@playwright/test";
import { readFile, rename, writeFile } from "node:fs/promises";
import { delimiter, join } from "node:path";
import { startExactRelaunchFixture, writeFixtureNodeCommand } from "./_helpers/exact-relaunch";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

test.skip(process.env.GWT_PLAYWRIGHT_ACCOUNTS_POOL !== "1", "requires the checkout gwt and gwtd binaries");
test.skip(process.platform === "win32", "isolates the provider executable with a POSIX wrapper");
test.use({ viewport: { width: 1440, height: 900 } });
test.setTimeout(120_000);

const authFixture = (identity: string) => JSON.stringify({
  auth_mode: "chatgpt",
  tokens: { access_token: `fixture-access-${identity}`, refresh_token: `fixture-refresh-${identity}`, account_id: identity },
});

async function loginOutput(page: Page) {
  return page.evaluate(() => {
    const messages = (window as typeof window & {
      __gwtPlaywrightMessages?: { payload: { kind: string; login_id?: string; data_base64?: string } }[];
    }).__gwtPlaywrightMessages || [];
    const login = messages.findLast(({ payload }) => payload.kind === "pool_account_login_started")?.payload.login_id;
    return messages.filter(({ payload }) => payload.kind === "pool_account_login_output" && payload.login_id === login)
      .map(({ payload }) => atob(payload.data_base64 || "")).join("");
  });
}

test("adds and signs in an account while the running account stays in its original pane", async ({ page }, info) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const fixture = await startExactRelaunchFixture(info, {
    async prepare({ home, bin, env }) {
      // Select the default account from the pool rather than an explicit override.
      delete env.CODEX_HOME;
      env.PATH = [bin, "/usr/bin", "/bin"].join(delimiter);
      const recorder = join(bin, "codex-recorder");
      await rename(join(bin, "codex"), recorder);
      await writeFixtureNodeCommand(bin, "codex-fixture-login", `#!${process.execPath}
const fs = require("node:fs");
const path = require("node:path");
const readline = require("node:readline");
const home = process.env.CODEX_HOME;
if (!home || home === path.join(process.env.HOME, ".codex")) process.exit(2);
console.log("GWT_FIXTURE_LOGIN_READY");
readline.createInterface({input: process.stdin}).once("line", line => {
  if (line === "fixture-confirm") fs.writeFileSync(path.join(home, "auth.json"), ${JSON.stringify(authFixture("added"))});
  process.exit(0);
});
`);
      await writeFixtureNodeCommand(bin, "codex", `#!/bin/sh
if [ "$1" = login ]; then
  if [ "$2" = status ]; then exit 1; fi
  exec "$(dirname "$0")/codex-fixture-login"
fi
exec "$(dirname "$0")/codex-recorder" "$@"
`);
      await writeFile(join(home, ".codex", "auth.json"), authFixture("default"));
    },
  });
  try {
    await page.addInitScript(({ theme, origin }) => {
      if (location.origin === origin) localStorage.setItem("gwt:ui:theme", theme);
    }, { theme, origin: new URL(fixture.url).origin });
    await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect.poll(() => page.evaluate(() => {
      const messages = (window as typeof window & {
        __gwtPlaywrightMessages?: { payload: { kind: string; status?: { view_state: string } } }[];
      }).__gwtPlaywrightMessages || [];
      return messages.findLast(({ payload }) => payload.kind === "project_execution_status")?.payload.status?.view_state;
    }), { message: "the host accepted the actual Project view", timeout: 30_000 }).toBe("open");
    await sendLiveGwtEvent(page, { kind: "create_window", preset: "codex" });
    await expect.poll(async () => {
      const errors = await page.evaluate(() => {
        const messages = (window as typeof window & {
          __gwtPlaywrightMessages?: { payload: { kind: string; status?: string; detail?: string; message?: string } }[];
        }).__gwtPlaywrightMessages || [];
        return messages.filter(({ payload }) => payload.kind === "window_command_error"
          || (payload.kind === "terminal_status" && payload.status === "error"))
          .map(({ payload }) => payload.detail || payload.message || payload.kind);
      });
      if (errors.length) throw new Error(`Fixture launch failed. ${errors.join(" ")}`);
      return fixture.launches();
    }, { timeout: 45_000 }).toHaveLength(1);
    const running = (await fixture.launches())[0];

    await page.locator(".project-bar").getByRole("button", { name: "Environments" }).click();
    const surface = page.locator(".exec-env");
    await surface.getByRole("button", { name: "Accounts", exact: true }).click();
    const defaultAccount = surface.locator('[data-account="codex"]');
    const addedAccount = surface.locator('[data-account="codex-2"]');
    const accountOrder = () => surface.locator('[data-account^="codex"]').evaluateAll(rows => rows.map(row => row.getAttribute("data-account")));
    await expect(defaultAccount).toContainText("First choice");

    await surface.getByRole("button", { name: "Add an account" }).click();
    const add = page.getByRole("dialog", { name: "Add an account" });
    const provider = add.getByRole("combobox", { name: "Provider" });
    await expect(provider.locator('option[value="agy"]')).toBeDisabled();
    await expect(add.getByRole("list", { name: "Providers that cannot be added" })).toContainText("Antigravity");
    await provider.selectOption("codex");
    await add.getByRole("button", { name: "Open login terminal" }).click();
    const terminal = page.getByRole("dialog", { name: /^Login terminal/ });
    const input = terminal.locator(".xterm-helper-textarea");
    await expect(input).toBeFocused();
    await expect.poll(() => loginOutput(page)).toContain("GWT_FIXTURE_LOGIN_READY");
    await expect(addedAccount).toContainText("Waiting for sign-in");
    await info.attach("sign-in terminal", { body: await terminal.screenshot(), contentType: "image/png" });
    await input.press("Enter");
    await terminal.getByRole("button", { name: "Done" }).click();
    await expect(addedAccount).toContainText("Waiting for sign-in");
    await expect(addedAccount.getByRole("button", { name: "Use Codex 2" })).toBeDisabled();

    await addedAccount.getByRole("button", { name: "Sign in to Codex 2" }).click();
    await expect(input).toBeFocused();
    await expect.poll(() => loginOutput(page)).toContain("GWT_FIXTURE_LOGIN_READY");
    await page.keyboard.type("fixture-confirm");
    await input.press("Enter");
    await expect(terminal.getByRole("status")).toHaveText("Signed in");
    await terminal.getByRole("button", { name: "Done" }).click();
    await expect(addedAccount).toContainText("Signed in");

    await addedAccount.getByRole("button", { name: "Move Codex 2 up" }).click();
    await expect.poll(accountOrder).toEqual(["codex-2", "codex"]);
    await addedAccount.getByRole("button", { name: "Move Codex 2 down" }).click();
    await expect.poll(accountOrder).toEqual(["codex", "codex-2"]);
    await addedAccount.getByRole("button", { name: "Use Codex 2" }).click();
    await expect(addedAccount).toContainText("First choice");
    const active = surface.locator("[data-account-session]").filter({ hasText: "Using Codex 1" });
    await expect(active).toHaveCount(1);
    await expect(active.getByRole("button", { name: "Resume with the next account" })).toBeEnabled();
    expect(await fixture.launches()).toEqual([running]);
    expect(() => process.kill(running.pid, 0), "the original native provider remains alive").not.toThrow();
    await info.attach("account priorities", { body: await surface.screenshot(), contentType: "image/png" });

    await addedAccount.getByRole("button", { name: "Delete Codex 2" }).click();
    await page.getByRole("dialog", { name: "Delete Codex 2?" }).getByRole("button", { name: "Delete account" }).click();
    await expect(addedAccount).toHaveCount(0);
    await expect(defaultAccount).toContainText("First choice");
    await expect(active).toHaveCount(1);
    expect(await readFile(join(fixture.home, ".codex", "auth.json"), "utf8")).toBe(authFixture("default"));
    expect(errors, "zero console and page errors").toEqual([]);
  } finally {
    await fixture.stop();
  }
});
