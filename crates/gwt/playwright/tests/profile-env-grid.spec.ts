import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Profile environment variable grid", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("renders OS rows, auto-switches overrides, and saves added variables", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installProfileBackend(page);

    await page.goto(APP_URL);

    const profile = page.locator(".surface-profile");
    await expect(profile.getByText("Environment Variables")).toBeVisible({
      timeout: 10_000,
    });
    await expect(profile.getByText("Save now")).toHaveCount(0);
    await expect(profile.getByText("Merged preview")).toHaveCount(0);

    const pathRow = profile.locator(".profile-env-row", { hasText: "PATH" });
    await expect(pathRow.locator("select")).toHaveValue("use_os");
    await expect(pathRow.locator(".profile-env-result")).toHaveText("/usr/bin");

    await pathRow.locator('input[aria-label^="Profile value"]').fill("/custom/bin");
    await expect(pathRow.locator("select")).toHaveValue("override");
    await expect(pathRow.locator(".profile-env-result")).toHaveText("/custom/bin");
    await page.waitForFunction(() =>
      window.__profileFixtureSends.some(
        (message) =>
          message.kind === "save_profile" &&
          message.env_vars.some(
            (entry) => entry.key === "PATH" && entry.value === "/custom/bin",
          ),
      ),
    );

    const tokenRow = profile.locator(".profile-env-row", { hasText: "GITHUB_TOKEN" });
    await expect(tokenRow.locator(".profile-env-os-value")).toHaveText("fixture-token");
    await expect(tokenRow.locator("select")).toHaveValue("disabled");
    await expect(tokenRow.locator(".profile-env-result")).toHaveText("Disabled");

    await profile.getByRole("button", { name: "+ Add variable" }).click();
    await profile
      .locator('input[aria-label^="Environment variable key"]')
      .last()
      .fill("CUSTOM_FLAG");
    await page.waitForFunction(() =>
      window.__profileFixtureSends.some(
        (message) =>
          message.kind === "save_profile" &&
          message.env_vars.some((entry) => entry.key === "CUSTOM_FLAG"),
      ),
    );

    await profile
      .locator('input[aria-label^="Profile value"]')
      .last()
      .fill("1");
    await page.waitForFunction(() =>
      window.__profileFixtureSends.some(
        (message) =>
          message.kind === "save_profile" &&
          message.env_vars.some(
            (entry) => entry.key === "CUSTOM_FLAG" && entry.value === "1",
          ),
      ),
    );
  });
});

async function installProfileBackend(page) {
  await page.addInitScript(() => {
    const recordedSends = [];
    window.__profileFixtureSends = recordedSends;

    const profileWindow = {
      id: "profile-1",
      title: "Profile",
      preset: "profile",
      geometry: { x: 96, y: 96, width: 1040, height: 680 },
      z_index: 1,
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
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Fixture Project",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [profileWindow],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };
    const snapshot = {
      active_profile: "default",
      selected_profile: "default",
      profiles: [
        {
          name: "default",
          description: "Default profile",
          env_vars: [],
          disabled_env: ["GITHUB_TOKEN"],
          is_default: true,
          is_active: true,
        },
      ],
      os_env: [
        { key: "GITHUB_TOKEN", value: "fixture-token" },
        { key: "PATH", value: "/usr/bin" },
      ],
      merged_preview: [],
    };

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw) {
        const message = JSON.parse(raw);
        recordedSends.push(message);
        if (message.kind === "frontend_ready") {
          this.emit(workspaceState);
          return;
        }
        if (message.kind === "load_profile") {
          this.emit({ kind: "profile_snapshot", id: message.id, snapshot });
          return;
        }
        if (message.kind === "save_profile") {
          snapshot.profiles[0].env_vars = message.env_vars;
          snapshot.profiles[0].disabled_env = message.disabled_env;
          this.emit({ kind: "profile_snapshot", id: message.id, snapshot });
        }
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload) {
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
  });
}
