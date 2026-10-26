import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { gotoLiveGwt, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const base = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";

test("Issue search returns ordinary Issues and SPECs from the real unified index", async ({ page }, info) => {
  test.skip(!base || !home, "requires the Issue #5562 isolated mixed-cache fixture");
  test.setTimeout(180_000);
  await withLiveGwtBackendLock(base, info, async () => {
    const fixture = JSON.parse(await readFile(join(home, "issue-5562-isolated.json"), "utf8"));
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const errors: string[] = [];
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("pageerror", error => errors.push(error.message));
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
    await gotoLiveGwt(page, base, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await page.locator('button[data-surface="issues"]').click();
    const issues = page.locator('.workspace-window[data-preset="issue"]');
    await expect(issues.locator(".issue-bridge-root")).toBeVisible();
    // Both titles deliberately omit the query: a cache-backed exact-title
    // match cannot satisfy this real semantic-search assertion.
    expect(fixture.issue_title.toLowerCase()).not.toContain(fixture.query.toLowerCase());
    expect(fixture.spec_title.toLowerCase()).not.toContain(fixture.query.toLowerCase());
    await issues.locator(".knowledge-search").fill(fixture.query);
    const response = await page.waitForFunction(query => {
      const messages = (window as any).__gwtPlaywrightMessages ?? [];
      return messages.map(({ payload }: any) => payload).findLast((payload: any) =>
        payload.kind === "knowledge_search_results" && payload.knowledge_kind === "issue"
        && payload.query === query && !payload.semantic_retry);
    }, fixture.query, { timeout: 120_000 }).then(handle => handle.jsonValue());
    expect(response.entries).toEqual(expect.arrayContaining([
      expect.objectContaining({ number: fixture.issue_number, title: fixture.issue_title, is_spec: false }),
      expect.objectContaining({ number: fixture.spec_number, title: fixture.spec_title, is_spec: true }),
    ]));
    await expect(issues.locator(`[data-issue-number="${fixture.issue_number}"]`).first())
      .toContainText(fixture.issue_title);
    await expect(issues.locator(`[data-issue-number="${fixture.spec_number}"]`).first())
      .toContainText(fixture.spec_title);
    const screenshot = info.outputPath(`unified-issue-search-${theme}.png`);
    await page.screenshot({ path: screenshot });
    await info.attach(`unified-issue-search-${theme}`, { path: screenshot, contentType: "image/png" });
    await info.attach("real-search-response", { body: JSON.stringify(response, null, 2), contentType: "application/json" });
    expect(errors, "zero console and page errors").toEqual([]);
  });
});
