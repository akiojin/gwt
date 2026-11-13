import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";

test("Issue #6997 source preflight precedes real themed Chromium", async ({ page }, info) => {
  test.skip(process.env.GWT_PLAYWRIGHT_ISSUE_6997 !== "1", "requires verify-issue-6997.py");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const evidence = await readFile(join(process.env.GWT_PLAYWRIGHT_CHECK_HOME!, "preflight.json"), "utf8");
  const preflight = JSON.parse(evidence);
  for (const proof of [preflight.before, preflight.after]) {
    expect(proof).toMatchObject({ context: "current", result: "proceed", reasons: [] });
  }
  const theme = info.project.name.endsWith("light") ? "light" : "dark";
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  await gotoLiveGwt(page, process.env.GWT_PLAYWRIGHT_BASE_URL!, {
    projectKey: process.env.GWT_PLAYWRIGHT_PROJECT_KEY,
  });
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await expect(page.locator(".project-bar")).toBeVisible();
  await expect(page.locator(".op-rail")).toBeVisible();
  await page.screenshot({ path: info.outputPath(`${theme}-preflight.png`) });
  expect(errors, "console and page errors").toEqual([]);
  await info.attach("source-preflight", { body: evidence, contentType: "application/json" });
});
