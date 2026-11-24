import { expect, test } from "@playwright/test";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { delimiter, join } from "node:path";
import { startExactRelaunchFixture, writeFixtureNodeCommand } from "./_helpers/exact-relaunch";
import { gotoLiveGwt } from "./_helpers/live-gwt";

test.skip(process.env.GWT_PLAYWRIGHT_ACCOUNT_ADOPTION !== "1", "requires the checkout gwt and gwtd binaries");
test.skip(process.platform === "win32", "isolates the provider path with POSIX system directories");
test.use({ viewport: { width: 1440, height: 900 } });
test.setTimeout(120_000);

test("adopts signed-in Grok Build at startup without a project launch entry", async ({ page }, info) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const fixture = await startExactRelaunchFixture(info, {
    async prepare({ home, bin, env }) {
      env.PATH = [bin, "/usr/bin", "/bin"].join(delimiter);
      env.XDG_DATA_HOME = join(home, ".local", "share");
      env.GH_CONFIG_DIR = join(home, ".config", "gh");
      delete env.COPILOT_GITHUB_TOKEN;
      delete env.GROK_HOME;
      // Keep Codex installed with a deterministic signed-out status.
      await writeFixtureNodeCommand(bin, "codex", `#!${process.execPath}
if (process.argv.includes('--version') || process.argv.includes('-V')) {
  console.log('codex-cli 0.116.0');
} else {
  process.exitCode = 1;
}
`);
      const grokHome = join(home, ".grok");
      await mkdir(grokHome);
      await writeFile(join(grokHome, "auth.json"), JSON.stringify({
        "xai:default": { type: "oauth", key: "fixture-grok-sign-in" },
      }));
    },
  });

  try {
    const storePath = join(fixture.home, ".gwt", "machine-state", "execution-environments.json");
    const readStore = async () => {
      try {
        return JSON.parse(await readFile(storePath, "utf8"));
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === "ENOENT") return null;
        throw error;
      }
    };
    // Check startup before any browser WebSocket can request registration.
    await expect.poll(readStore, { timeout: 30_000 }).toMatchObject({
      accounts: [{ id: "grok", tool: "grok", origin: "adopted" }],
      environments: [{ id: "grok", account_id: "grok", via_tool: "grok" }],
    });
    const startupStore = await readStore();

    await page.addInitScript(({ theme, origin }) => {
      if (location.origin === origin) localStorage.setItem("gwt:ui:theme", theme);
    }, { theme, origin: new URL(fixture.url).origin });
    await gotoLiveGwt(page, fixture.url);

    const surface = page.locator(".exec-env");
    const openEnvironments = () => page.locator(".project-bar").getByRole("button", { name: "Environments" }).click();
    const grok = surface.locator('[data-environment="grok"][data-kind="set"]');
    await openEnvironments();
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(grok.getByRole("heading", { name: "Grok Build 1" })).toBeVisible();
    await expect(grok).toContainText("Not on any project's list");
    await expect(surface.locator("[data-environment]")).toHaveCount(1);
    await info.attach("adopted Grok execution environment", { body: await surface.screenshot(), contentType: "image/png" });

    await surface.getByRole("button", { name: "Add an execution environment" }).click();
    const accounts = surface.getByRole("combobox", { name: "Account", exact: true });
    await expect(accounts).toHaveValue("grok");
    await expect(accounts.locator("option")).toHaveText(["Grok Build 1"]);

    await page.reload();
    await openEnvironments();
    await expect(grok).toBeVisible();
    await expect(surface.locator("[data-environment]")).toHaveCount(1);
    expect(await readStore(), "reads keep the adopted account unchanged").toEqual(startupStore);
    expect(errors, "zero console and page errors").toEqual([]);
  } finally {
    await fixture.stop();
  }
});
