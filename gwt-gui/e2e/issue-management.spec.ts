import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  branchMain,
  branchDevelop,
  branchFeature,
  openRecentProject,
  setMockCommandResponses,
  standardBranchResponses,
} from "./support/helpers";

const linkedIssueFixture = {
  number: 101,
  title: "Fix login flow",
  source: "branch_name",
  body: "Login flow is broken",
  labels: [{ name: "bug", color: "d73a4a" }],
  assignees: [{ login: "dev-1" }],
};

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("Issue tab shows no linked issue message by default", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Issue", exact: true })
    .click();

  await expect(page.getByText("No linked issue")).toBeVisible();
});

test("Issue tab button exists in summary tabs", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await expect(
    page
      .locator(".summary-tabs")
      .getByRole("button", { name: "Issue", exact: true }),
  ).toBeVisible();
});

test("Issue tab shows linked issue when available", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    fetch_branch_linked_issue: linkedIssueFixture,
  });
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Issue", exact: true })
    .click();

  await expect(page.getByText("Fix login flow")).toBeVisible();
});

test("Issue tab shows issue number", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    fetch_branch_linked_issue: linkedIssueFixture,
  });
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Issue", exact: true })
    .click();

  await expect(page.locator(".issue-panel .quick-subtitle")).toContainText(
    "#101",
  );
});

test("Issue tab becomes active when clicked", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  const issueBtn = page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Issue", exact: true });
  await issueBtn.click();

  await expect(issueBtn).toHaveClass(/active/);
});

test("switching from Issue tab to Summary tab works", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Issue", exact: true })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Summary", exact: true })
    .click();

  await expect(
    page
      .locator(".summary-tabs")
      .getByRole("button", { name: "Summary", exact: true }),
  ).toHaveClass(/active/);
});

test("Docker tab exists in summary tabs", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await expect(
    page
      .locator(".summary-tabs")
      .getByRole("button", { name: "Docker", exact: true }),
  ).toBeVisible();
});

test("all summary tabs are visible for a selected branch", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  const tabs = page.locator(".summary-tabs");
  await expect(
    tabs.getByRole("button", { name: "Summary", exact: true }),
  ).toBeVisible();
  await expect(
    tabs.getByRole("button", { name: "Git", exact: true }),
  ).toBeVisible();
  await expect(
    tabs.getByRole("button", { name: "Issue", exact: true }),
  ).toBeVisible();
  await expect(
    tabs.getByRole("button", { name: "PR", exact: true }),
  ).toBeVisible();
  await expect(
    tabs.getByRole("button", { name: "Docker", exact: true }),
  ).toBeVisible();
});

test("Issue tab shows 'No issue linked to this branch' message", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".summary-tabs")
    .getByRole("button", { name: "Issue", exact: true })
    .click();

  await expect(
    page.getByText("No issue linked to this branch"),
  ).toBeVisible();
});
