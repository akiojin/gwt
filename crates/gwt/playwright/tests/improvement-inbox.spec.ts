import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

function improvementWindow() {
  return {
    id: "improvement-live-1",
    title: "Improvement Inbox",
    preset: "improvement",
    geometry: { x: 96, y: 76, width: 1260, height: 760 },
    z_index: 10,
    status: "running",
    minimized: false,
    maximized: false,
    pre_maximize_geometry: null,
    persist: true,
    purpose_title: null,
    dynamic_title: null,
    dynamic_title_detail: null,
    agent_id: null,
    agent_color: null,
    tab_group_id: null,
    tab_group_active: false,
  };
}

function workspaceState() {
  return {
    kind: "workspace_state",
    workspace: {
      app_version: "playwright",
      tabs: [
        {
          id: "improvement-live-tab",
          title: "Improvement Live",
          project_root: "/fixture/improvement-live",
          kind: "git",
          workspace: {
            viewport: { x: 0, y: 0, zoom: 1 },
            windows: [improvementWindow()],
          },
        },
      ],
      active_tab_id: "improvement-live-tab",
      recent_projects: [],
    },
  };
}

function improvementCandidate() {
  return {
    id: "impr-00321e9c2844449f8848f28e1b413e68",
    state: "pending",
    confidence: "high",
    target_artifact: "skill",
    classification: "gwt-caused",
    summary: "Visual check pending candidate for Approve preview",
    dedupe_key: "visual:approve-preview-20260625",
    occurrences: 1,
    issue_preview: {
      repository: "akiojin/gwt",
      title: "fix(gwt): Visual check pending candidate for Approve preview",
      body: [
        "## Problem",
        "",
        "Visual check pending candidate for Approve preview",
        "",
        "## Expected behavior",
        "",
        "gwt should handle `skill` self-improvement failures with enough public-safe context.",
        "",
        "## Observed evidence",
        "",
        "Clicking Approve should open an in-app confirmation modal with the public Issue preview.",
        "",
        "## Impact",
        "",
        "Repeated gwt-caused failures can stay local instead of becoming trackable upstream work.",
        "",
        "## Suggested verification",
        "",
        "- Verify the preview before creating the Issue.",
        "",
        "## Source candidate",
        "",
        "- Candidate ID: impr-00321e9c2844449f8848f28e1b413e68",
        "",
        "## Privacy",
        "",
        "- Public body generated from sanitized candidate fields only.",
      ].join("\n"),
    },
  };
}

async function inject(page: any, detail: unknown) {
  await page.evaluate((payload) => {
    (window as any).__improvementInboxSocket.emit(payload);
  }, detail);
}

test.describe("Improvement Inbox", () => {
  test("mounted inbox refreshes when candidates arrive and opens the public Issue preview", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await page.addInitScript((initialWorkspaceState) => {
      try {
        window.sessionStorage.setItem("gwt:ui:briefing", "1");
      } catch {
        /* no-op */
      }

      class FixtureWebSocket extends EventTarget {
        static CONNECTING = 0;
        static OPEN = 1;
        static CLOSING = 2;
        static CLOSED = 3;

        url: string;
        readyState: number;

        constructor(url: string) {
          super();
          this.url = url;
          this.readyState = FixtureWebSocket.CONNECTING;
          (window as any).__improvementInboxSocket = this;
          setTimeout(() => {
            this.readyState = FixtureWebSocket.OPEN;
            this.dispatchEvent(new Event("open"));
          }, 0);
        }

        send(raw: string) {
          const message = JSON.parse(raw);
          if (message.kind === "frontend_ready") {
            this.emit(initialWorkspaceState);
          }
        }

        close() {
          this.readyState = FixtureWebSocket.CLOSED;
          this.dispatchEvent(new CloseEvent("close"));
        }

        emit(payload: unknown) {
          setTimeout(() => {
            this.dispatchEvent(
              new MessageEvent("message", { data: JSON.stringify(payload) }),
            );
          }, 0);
        }
      }

      Object.defineProperty(window, "WebSocket", {
        configurable: true,
        value: FixtureWebSocket,
      });
    }, workspaceState());

    await page.goto(APP_URL);
    const inbox = page.locator('.workspace-window[data-preset="improvement"]');
    await expect(inbox).toBeVisible();
    await expect(inbox.locator(".improvement-inbox-count")).toHaveText("0");

    await inject(page, {
      kind: "improvement_candidates",
      candidates: [improvementCandidate()],
    });

    await expect(inbox.locator(".improvement-inbox-count")).toHaveText("1");
    await expect(inbox.locator(".improvement-inbox-row[data-improvement-id]")).toHaveCount(1);

    await inbox.locator("[data-action='approve-improvement']").click();
    const modal = inbox.locator("[data-improvement-modal='approve']");
    await expect(modal).toBeVisible();
    await expect(modal).toContainText("Create public Issue");
    await expect(modal).toContainText("## Problem");
    await expect(modal).toContainText("## Expected behavior");
    await expect(modal).toContainText("## Observed evidence");
    await expect(modal).toContainText("## Impact");
    await expect(modal).toContainText("## Suggested verification");
    await expect(modal).toContainText("## Source candidate");
    await expect(modal).toContainText("## Privacy");
  });
});
