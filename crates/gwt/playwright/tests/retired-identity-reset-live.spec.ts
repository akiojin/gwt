/**
 * #4825: seed the fixture as project-state/current.json and a version-0
 * agent_identity.migration.json before starting the isolated checkout.
 * GWT_PLAYWRIGHT_IDENTITY_STATE points at that isolated project-state directory.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, openLiveGwtProject } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const STATE = process.env.GWT_PLAYWRIGHT_IDENTITY_STATE ?? "";

test("startup retains saved identity and the retired reset marker", async ({ page }, testInfo) => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set");
  expect(STATE, "Seed GWT_PLAYWRIGHT_IDENTITY_STATE before startup").not.toBe("");
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  await gotoLiveGwt(page, BASE, { hub: true });
  await openLiveGwtProject(page);
  const theme = testInfo.project.use.colorScheme === "light" ? "light" : "dark";
  await page.locator(`#op-theme-toggle [data-theme-value="${theme}"]`).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

  const projection = JSON.parse(await readFile(join(STATE, "current.json"), "utf8"));
  expect(projection.agents.find((agent: { session_id: string }) => agent.session_id === "saved-identity"))
    .toMatchObject({ title_summary: "Saved purpose", current_focus: "Saved progress" });
  expect(await readFile(join(STATE, "agent_identity.migration.json"), "utf8"))
    .toBe('{"version":0}');
  expect(errors).toEqual([]);
});
