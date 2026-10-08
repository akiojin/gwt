import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// SPEC #3885 Phase 3: deterministic protocol data, real Chromium and checkout assets.
// A browser-check Project URL uses the isolated binary's assets; without one the
// shared embedded frontend fixture serves this checkout for local development.
const errors = new WeakMap<Page, string[]>();
const configuredUrl = process.env.GWT_PLAYWRIGHT_BASE_URL;
// A browser-check URL may point at the Hub; these fixtures exercise Project assets.
const liveUrl = configuredUrl
  ? new URL(new URL(configuredUrl).pathname === "/" ? new URL(APP_URL).pathname : new URL(configuredUrl).pathname, configuredUrl).toString()
  : undefined;

test.use({ viewport: { width: 1600, height: 1100 } });
test.beforeEach(async ({ page }, info) => {
  const captured: string[] = [];
  errors.set(page, captured);
  page.on("pageerror", (error) => captured.push(String(error)));
  page.on("console", (message) => {
    if (message.type() === "error") captured.push(message.text());
  });
  await page.addInitScript((theme) => {
    localStorage.setItem("gwt:ui:theme", theme);
  }, info.project.name.includes("light") ? "light" : "dark");
  if (!liveUrl) await installEmbeddedRoutes(page);
});
test.afterEach(async ({ page }) => {
  expect(errors.get(page), "zero console/page errors").toEqual([]);
});

async function boot(page: Page, preset = "issue") {
  await installBackend(page, preset);
  await page.goto(liveUrl || APP_URL);
  await expect(page.locator(".issue-bridge-root")).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme",
    test.info().project.name.includes("light") ? "light" : "dark");
  await expect(page.locator(".issue-other-summary")).toHaveText("Other (2)");
}

const row = (page: Page, id: string) => page.locator(
  `.issue-other-group .workspace-overview-row[data-workspace-id="${id}"]`,
);
async function messages(page: Page, kind: string) {
  return page.evaluate((kind) => (window as any).__otherMessages.filter(
    (message: any) => message.kind === kind,
  ), kind);
}

test("Other stays authoritative under search and queue columns, with no Workspace surface", async ({ page }) => {
  await boot(page);
  const other = page.locator("details.issue-other-group");
  await expect(other).not.toHaveAttribute("open");
  await expect(row(page, "other-paused")).toBeHidden();
  await expect(page.locator("#op-workspace-overview-entry")).toHaveAccessibleName("Issues");
  await expect(page.locator(".workspace-window.surface-work")).toHaveCount(0);
  await other.locator("summary").click();
  await expect(row(page, "other-paused")).toBeVisible();
  await expect(row(page, "linked-visible")).toHaveCount(0);
  await expect(row(page, "linked-uncached")).toHaveCount(0);
  await expect(row(page, "unknown-authority")).toHaveCount(0);
  await expect(row(page, "remote-start:feature/remote")).toBeVisible();
  await expect(row(page, "remote-start:feature/linked")).toHaveCount(0);
  await page.screenshot({ path: test.info().outputPath("issue-other.png") });

  await page.locator(".knowledge-search").fill("no matching issue");
  await expect.poll(() => messages(page, "search_knowledge_bridge")).not.toHaveLength(0);
  await expect(page.locator(".knowledge-row")).toHaveCount(0);
  await expect(row(page, "other-paused")).toBeVisible();
  await expect(row(page, "linked-visible")).toHaveCount(0);
  await expect(page.locator("[data-issue-filter], [data-issue-lane-filter]")).toHaveCount(0);
  await expect(page.locator("[data-queue-column]")).toHaveCount(4);
  await expect(row(page, "linked-uncached")).toHaveCount(0);
  await expect(row(page, "other-paused")).toBeVisible();
  await expect(row(page, "remote-start:feature/remote")).toBeVisible();
});

test("Other disclosure survives a cache refresh between pointerdown and pointerup", async ({ page }) => {
  await boot(page);
  await page.evaluate(() => { (window as any).__otherHoldEntries = true; });
  await page.locator('[data-action="refresh-knowledge"]').click();
  await expect.poll(() => page.evaluate(() => (window as any).__otherPendingRefresh)).toBe(true);
  const other = page.locator("details.issue-other-group");
  const summary = other.locator("summary");
  await summary.scrollIntoViewIfNeeded();
  await other.evaluate(node => { (window as any).__otherBeforeRefresh = node; });
  const bounds = await summary.boundingBox();
  await page.mouse.move(bounds!.x + 30, bounds!.y + bounds!.height / 2);
  await page.mouse.down();
  await page.evaluate(() => (window as any).__otherRefresh());
  await expect.poll(() => other.evaluate(node => node === (window as any).__otherBeforeRefresh)).toBe(true);
  await page.mouse.up();
  await expect(other).toHaveAttribute("open");
  await expect(row(page, "other-paused")).toBeVisible();
});

test("Other keeps branch launch, conversation resume and backend-approved cleanup reachable", async ({ page }) => {
  await boot(page);
  await page.locator(".issue-other-summary").click();
  await row(page, "other-paused").click();
  const other = page.locator(".issue-other-group");
  await other.locator('[data-action="launch-workspace"]').click();
  await expect.poll(() => messages(page, "open_launch_wizard")).toEqual([
    expect.objectContaining({ branch_name: "feature/other" }),
  ]);
  await other.locator('[data-action="resume-session"]').click();
  await expect.poll(() => messages(page, "resume_workspace_agent")).toEqual([
    expect.objectContaining({ session_id: "ledger-other", agent_session_id: "conversation-other" }),
  ]);
  await other.locator('[data-action="cleanup-merged-workspace"]').click();
  const modal = page.locator("#branch-cleanup-modal");
  await expect(modal).toBeVisible();
  await expect(modal).toContainText("feature/other");
  await modal.getByRole("button", { name: "Run cleanup" }).click();
  await expect.poll(() => messages(page, "run_branch_cleanup")).toEqual([
    expect.objectContaining({ id: "tab-other::issue-1", branches: ["feature/other"], delete_remote: false }),
  ]);
});

test("a persisted work preset restores the Issue list with collapsed Other", async ({ page }) => {
  await boot(page, "work");
  await expect(page.locator(".workspace-window.surface-knowledge")).toHaveCount(1);
  await expect(page.locator(".workspace-window.surface-work")).toHaveCount(0);
  await expect(page.locator(".knowledge-row[data-issue-number='4556']")).toBeVisible();
  await expect(page.locator("details.issue-other-group")).not.toHaveAttribute("open");
  await page.locator(".issue-other-summary").click();
  await expect(row(page, "other-paused")).toBeVisible();
});

async function installBackend(page: Page, preset: string) {
  const projectKey = new URL(liveUrl || APP_URL).pathname.split("/")[2];
  await page.addInitScript(({ preset, projectKey }) => {
    const fixture = window as any;
    fixture.__otherMessages = [];
    let lastKnowledgeResponse: unknown;
    const issueWindow = {
      id: "tab-other::issue-1", title: "Issues", preset,
      geometry: { x: 50, y: 60, width: 1430, height: 940 },
      z_index: 1, status: "running", persist: true,
      minimized: false, maximized: false,
    };
    const workspace = {
      kind: "workspace_state", workspace: {
        app_version: "playwright",
        tabs: [{ id: "tab-other", title: "Other fixture", project_root: "/fixture",
          project_key: projectKey, kind: "git",
          workspace: { viewport: { x: 0, y: 0, zoom: 1 }, windows: [issueWindow] },
        }], active_tab_id: "tab-other", recent_projects: [],
      },
    };
    const projection = {
      id: "projection-other", title: "Other fixture", agents: [], works: [],
      unassigned_agents: [], journal_entries: [],
      active_works: [
        { id: "linked-visible", title: "Linked issue", linked_issue_numbers: [4556],
          branch: "feature/linked", status_category: "active", lifecycle_state: "active", agents: [], works: [] },
        { id: "linked-uncached", title: "Issue outside cached results", linked_issue_numbers: [9999],
          branch: "feature/uncached", status_category: "idle", lifecycle_state: "paused", agents: [], works: [] },
        { id: "unknown-authority", title: "Unknown owner", branch: "feature/unknown", agents: [], works: [] },
        { id: "other-paused", title: "Independent work", linked_issue_numbers: [],
          branch: "feature/other", worktree_path: "/fixture/work/other", status_category: "idle",
          lifecycle_state: "paused", agents: [],
          cleanup_candidate: { branch: "feature/other", reason: "no_changes", remote_delete_available: false },
          works: [{ id: "child-other", title: "Independent task", lifecycle_state: "paused",
            status_category: "idle", manual_close_allowed: true,
            agents: [{ session_id: "ledger-other", display_name: "Codex", agent_id: "codex",
              status_category: "idle", sessions: [{ agent_session_id: "conversation-other",
                started_at: "2026-09-20T00:00:00Z", resumable: true, is_active: false }],
            }],
          }],
        },
      ],
    };
    const entries = [{ number: 4556, title: "Integrate Other", state: "open", labels: [],
      is_spec: false, linked_branch_count: 1, match_score: 100,
      related_work_refs: [{ id: "linked-visible", branch: "feature/linked", updated_at: "" }],
    }];
    class FixtureSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      readyState = 0;
      constructor(public readonly url: string) {
        super();
        setTimeout(() => { this.readyState = 1; this.dispatchEvent(new Event("open")); }, 0);
      }
      emit(payload: any) {
        if (payload.kind === "knowledge_entries") {
          lastKnowledgeResponse = payload;
          fixture.__otherRefresh = () => this.dispatchEvent(new MessageEvent("message", {
            data: JSON.stringify(lastKnowledgeResponse),
          }));
          if (fixture.__otherHoldEntries) {
            fixture.__otherPendingRefresh = true;
            return;
          }
        }
        setTimeout(() => this.dispatchEvent(new MessageEvent("message", {
          data: JSON.stringify(payload),
        })), 0);
      }
      send(raw: string) {
        const message = JSON.parse(raw);
        fixture.__otherMessages.push(message);
        if (message.kind === "frontend_ready") {
          this.emit(workspace);
          this.emit({ kind: "active_work_projection", projection });
        } else if (message.kind === "request_remote_start_work_branches") {
          this.emit({ kind: "remote_start_work_branches", id: message.id,
            branches: ["origin/feature/linked", "origin/feature/other", "origin/feature/remote"] });
        } else if (["load_knowledge_bridge", "search_knowledge_bridge"].includes(message.kind)) {
          this.emit({ kind: "knowledge_entries", id: message.id, knowledge_kind: message.knowledge_kind,
            request_id: message.request_id, entries: message.query ? [] : entries,
            selected_number: message.query ? null : 4556, refresh_enabled: true, empty_message: null });
        } else if (message.kind === "select_knowledge_bridge_entry") {
          this.emit({ kind: "knowledge_detail", id: message.id, knowledge_kind: message.knowledge_kind,
            request_id: message.request_id, detail: { number: message.number, title: "Integrate Other",
              state: "open", labels: [], sections: [], related_works: [] } });
        }
      }
      close() { this.readyState = 3; this.dispatchEvent(new CloseEvent("close")); }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureSocket });
  }, { preset, projectKey });
}
