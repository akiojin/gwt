import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { gotoLiveGwt, openLiveGwtProject } from "./_helpers/live-gwt";

type GhIssue = {
  number: number;
  state: string;
  title: string;
  url: string;
};

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const REGISTER_REAL_ISSUE =
  process.env.GWT_PLAYWRIGHT_REGISTER_REAL_ISSUE === "1";
const REPO = process.env.GWT_PLAYWRIGHT_QUICK_ISSUE_REPO ?? "akiojin/gwt";

function ghIssueList(title: string): GhIssue[] {
  const output = execFileSync(
    "gh",
    [
      "issue",
      "list",
      "--repo",
      REPO,
      "--state",
      "all",
      "--search",
      `"${title}" in:title`,
      "--json",
      "number,state,title,url",
      "--limit",
      "20",
    ],
    { encoding: "utf8" },
  );
  return JSON.parse(output) as GhIssue[];
}

function closeGhIssue(issue: GhIssue): void {
  execFileSync(
    "gh",
    [
      "issue",
      "close",
      String(issue.number),
      "--repo",
      REPO,
      "--comment",
      "Closed by gwt Quick issue live E2E registration test.",
    ],
    { stdio: "ignore" },
  );
}

test.describe.serial("Issue Monitor Quick issue live registration", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");
  test.skip(
    !REGISTER_REAL_ISSUE,
    "Set GWT_PLAYWRIGHT_REGISTER_REAL_ISSUE=1 to create a real GitHub Issue",
  );

  test.use({ viewport: { width: 1280, height: 900 } });

  test("Register & Launch creates a GitHub Issue", async ({ page }, testInfo) => {
    test.setTimeout(60_000);
    test.skip(
      testInfo.project.name !== "chromium-dark",
      "Quick issue live registration runs once against the shared backend",
    );

    const title = `[gwt e2e] quick issue registration ${Date.now()}`;
    let created: GhIssue | undefined;

    try {
      await gotoLiveGwt(page, BASE, {
        enableTestBridge: true,
        keepPresetModal: true,
      });
      await openLiveGwtProject(page);

      await page.locator("#add-button").click();
      await page.locator('#preset-modal [data-preset="issue_monitor"]').click();

      const monitor = page
        .locator(".workspace-window.surface-issue-monitor:visible")
        .last()
        .locator(".issue-monitor-card");
      await expect(monitor).toBeVisible();

      await monitor.locator(".issue-monitor-card__quick-issue-input").fill(title);
      await monitor.locator(".issue-monitor-card__quick-issue-launch").click();

      await expect
        .poll(
          () => {
            const issue = ghIssueList(title).find((candidate) => {
              return candidate.title === title;
            });
            created = issue;
            return issue?.number ?? null;
          },
          {
            message: "Quick issue registration should create a GitHub Issue",
            timeout: 45_000,
          },
        )
        .not.toBeNull();

      expect(created?.title).toBe(title);
    } finally {
      if (created) {
        closeGhIssue(created);
      }
    }
  });
});
