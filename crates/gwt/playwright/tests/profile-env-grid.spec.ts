import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { gotoLiveGwt, withLiveGwtBackendLock } from "./_helpers/live-gwt";

test.describe("Profile environment variable grid", () => {
  test.use({ viewport: { width: 1440, height: 900 } });
  let browserErrors: string[] = [];
  test.beforeEach(({ page }) => {
    const errors = browserErrors = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  });
  test.afterEach(() => expect(browserErrors).toEqual([]));

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
    const gridMetrics = await profile.locator(".profile-env-grid").evaluate((node) => ({
      clientWidth: node.clientWidth,
      scrollWidth: node.scrollWidth,
    }));
    expect(gridMetrics.scrollWidth).toBeLessThanOrEqual(gridMetrics.clientWidth + 1);
    await expect(pathRow).toHaveCSS("grid-template-columns", /px/);
    const renderedColumns = await pathRow.evaluate((node) =>
      getComputedStyle(node).gridTemplateColumns.split(/\s+/).filter(Boolean).length,
    );
    expect(renderedColumns).toBe(5);
    await expect(pathRow.locator("select")).toHaveValue("use_os");
    await expect(pathRow.locator("select option")).toHaveText([
      "Use OS",
      "Override",
      "Disabled",
    ]);
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
    const pendingRow = profile.locator(".profile-env-row").last();
    await expect(pendingRow.locator("select")).toHaveValue("override");
    await expect(pendingRow.locator("select option")).toHaveText(["Enabled", "Disabled"]);
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
    const addedRow = profile.locator('.profile-env-row[data-env-key="CUSTOM_FLAG"]');
    await expect(addedRow).toHaveCount(1);

    const addedValueInput = addedRow.locator('input[aria-label^="Profile value"]');
    await addedValueInput.fill("1");
    await expect(addedValueInput).toHaveValue("1");
    await addedValueInput.blur();
    await page.waitForFunction(() =>
      window.__profileFixtureSends.some(
        (message) =>
          message.kind === "save_profile" &&
          message.env_vars.some(
            (entry) => entry.key === "CUSTOM_FLAG" && entry.value === "1",
          ),
      ),
    );
    await waitForProfileIdle(profile);

    await addedRow.locator("select").selectOption("disabled");
    await expect(addedRow.locator("select")).toHaveValue("disabled");
    await expect(addedRow.locator(".profile-env-result")).toHaveText("Disabled");
    await page.waitForFunction(() =>
      window.__profileFixtureSends.some(
        (message) =>
          message.kind === "save_profile" &&
          message.disabled_env.includes("CUSTOM_FLAG"),
      ),
    );
  });

  test("keeps focus while autosaving a newly added custom variable", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installProfileBackend(page);

    await page.goto(APP_URL);

    const profile = page.locator(".surface-profile");
    await expect(profile.getByText("Environment Variables")).toBeVisible({
      timeout: 10_000,
    });

    await profile.getByRole("button", { name: "+ Add variable" }).click();
    const keyInput = profile
      .locator('input[aria-label^="Environment variable key"]')
      .last();
    await keyInput.fill("CUSTOM_FOCUS");
    await page.waitForFunction(() =>
      window.__profileFixtureSends.some(
        (message) =>
          message.kind === "save_profile" &&
          message.env_vars.some((entry) => entry.key === "CUSTOM_FOCUS"),
      ),
    );

    await expect(keyInput).toBeFocused();
    await page.keyboard.type("_NEXT");
    await expect(keyInput).toHaveValue("CUSTOM_FOCUS_NEXT");
  });

  test("keeps metadata fixed while only the environment grid scrolls", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installProfileBackend(page);

    await page.goto(APP_URL);

    const profile = page.locator(".surface-profile");
    await expect(profile.getByText("Environment Variables")).toBeVisible({
      timeout: 10_000,
    });

    const editor = profile.locator(".profile-editor-pane");
    const metadata = profile.locator(".profile-editor-pane > .profile-section").first();
    const envSection = profile.locator(".profile-env-section");
    const envGrid = profile.locator(".profile-env-grid");
    const metadataName = profile.locator(".profile-field input").first();
    const metadataBox = await metadata.boundingBox();
    const envSectionBox = await envSection.boundingBox();
    const nameBoxBefore = await metadataName.boundingBox();

    expect(metadataBox?.height).toBeGreaterThan(160);
    expect(envSectionBox?.y || 0).toBeGreaterThan(
      (metadataBox?.y || 0) + (metadataBox?.height || 0),
    );

    const editorMetrics = await editor.evaluate((node) => ({
      clientHeight: node.clientHeight,
      overflowY: getComputedStyle(node).overflowY,
      scrollHeight: node.scrollHeight,
      scrollTop: node.scrollTop,
    }));
    expect(editorMetrics.overflowY).toBe("hidden");
    expect(editorMetrics.scrollTop).toBe(0);
    expect(editorMetrics.scrollHeight).toBeLessThanOrEqual(
      editorMetrics.clientHeight + 1,
    );

    const gridMetrics = await envGrid.evaluate((node) => ({
      clientHeight: node.clientHeight,
      overflowY: getComputedStyle(node).overflowY,
      scrollHeight: node.scrollHeight,
    }));
    expect(gridMetrics.overflowY).toBe("auto");
    expect(gridMetrics.scrollHeight).toBeGreaterThan(gridMetrics.clientHeight);

    await envGrid.evaluate((node) => {
      node.scrollTop = node.scrollHeight;
    });
    const nameBoxAfter = await metadataName.boundingBox();
    expect(nameBoxAfter?.y).toBeCloseTo(nameBoxBefore?.y || 0, 1);
    await expect(editor).toHaveJSProperty("scrollTop", 0);
  });

  test("keeps a newer unfocused draft when an older save ACK arrives", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installProfileBackend(page, { holdSaves: true });
    await page.goto(APP_URL);
    const profile = page.locator(".surface-profile");
    await waitForProfileIdle(profile);
    const value = profile.locator('.profile-env-row[data-env-key="PATH"] input');
    await value.fill("/older/bin");
    await value.blur();
    await page.waitForFunction(() => window.__profileFixturePendingSaves.length === 1);
    await value.fill("/newer/bin");
    await value.blur();
    await page.waitForFunction(() => window.__profileFixturePendingSaves.length === 2);
    await expect(value).not.toBeFocused();

    await page.evaluate(() => window.__profileFixtureReplyNext());
    await expect(value).toHaveValue("/newer/bin");
    await expect(profile.locator('.profile-env-row[data-env-key="PATH"] .profile-env-result')).toHaveText("/newer/bin");
    await page.evaluate(() => window.__profileFixtureReplyNext());
    await waitForProfileIdle(profile);
    await profile.getByRole("button", { name: "Refresh profiles" }).click();
    await waitForProfileIdle(profile);
    await expect(value).toHaveValue("/newer/bin");
  });

  test("retains the draft and existing snapshot on a save error, then recovers", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installProfileBackend(page, { holdSaves: true });
    await page.goto(APP_URL);
    const profile = page.locator(".surface-profile");
    await waitForProfileIdle(profile);
    const name = profile.locator(".profile-field input").first();
    await name.fill("unsaved-name");
    await name.blur();
    await page.waitForFunction(() => window.__profileFixturePendingSaves.length === 1);
    await expect(profile.locator(".profile-status")).toHaveText("Saving profile...");
    await page.evaluate(() => window.__profileFixtureReplyNext("fixture save failed"));
    await expect(profile.locator(".profile-status.error")).toHaveText("fixture save failed");
    await expect(name).toHaveValue("unsaved-name");
    await expect(profile.locator(".profile-row.selected .profile-row-title")).toHaveText("default");
    await expect(profile.locator('.profile-env-row[data-env-key="PATH"] .profile-env-os-value')).toHaveText("/usr/bin");
    await name.fill("default");
    await name.blur();
    await profile.getByRole("button", { name: "Refresh profiles" }).click();
    await waitForProfileIdle(profile);
    await expect(name).toHaveValue("default");
    await expect(profile.locator(".profile-status.error")).toHaveCount(0);
  });

  test("live Profile saves a marker across reload and recovers from a rename error", async ({ page }, info) => {
    const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
    test.skip(!base || !process.env.GWT_PLAYWRIGHT_CHECK_HOME, "requires an isolated live gwt backend");
    test.setTimeout(90_000);
    await withLiveGwtBackendLock(base!, info, async () => {
      const theme = info.project.name.includes("light") ? "light" : "dark";
      await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
      await gotoLiveGwt(page, base!, { enableTestBridge: true, keepPresetModal: true });
      await page.locator("#add-button").click();
      await page.locator('#preset-modal [data-preset="profile"]').click();
      const pane = page.locator('.workspace-window[data-preset="profile"]');
      const profile = pane;
      await expect(profile.getByText("Environment Variables", { exact: true })).toBeVisible();
      // The default window is shorter than the fixed metadata plus env editor.
      // Use normal window gestures to give the live case the fixture's height.
      const titlebar = await pane.locator(".titlebar").boundingBox();
      expect(titlebar).not.toBeNull();
      await page.mouse.move(titlebar!.x + 20, titlebar!.y + 10);
      await page.mouse.down();
      await page.mouse.move(120, 90, { steps: 5 });
      await page.mouse.up();
      const handle = await pane.locator(".resize-handle").boundingBox();
      const frame = await pane.boundingBox();
      expect(handle).not.toBeNull();
      expect(frame).not.toBeNull();
      await page.mouse.move(handle!.x + handle!.width / 2, handle!.y + handle!.height / 2);
      await page.mouse.down();
      await page.mouse.move(handle!.x + handle!.width / 2, handle!.y + handle!.height / 2 + 680 - frame!.height, { steps: 5 });
      await page.mouse.up();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      const id = await pane.getAttribute("data-id");
      expect(id).toBeTruthy();
      await waitForProfileIdle(profile);
      const cursor = () => page.evaluate(() => Number((window as any).__gwtPlaywrightMessageSequence) || 0);
      const refresh = async () => {
        const after = await cursor();
        await profile.getByRole("button", { name: "Refresh profiles" }).click();
        await waitForLiveProfileReply(page, id!, after);
        await waitForProfileIdle(profile);
      };
      await refresh();

      const key = "GWT_PROFILE_LIVE_FIXTURE";
      const marker = `profile-grid-${theme}`;
      await expect(profile.locator(`.profile-env-row[data-env-key="${key}"]`)).toHaveCount(0);
      await profile.getByRole("button", { name: "+ Add variable" }).click();
      const keyInput = profile.locator('input[aria-label^="Environment variable key"]').last();
      const keyCursor = await cursor();
      await keyInput.fill(key);
      await keyInput.blur();
      await waitForLiveProfileReply(page, id!, keyCursor, key);
      const value = profile.locator(`.profile-env-row[data-env-key="${key}"] input[aria-label^="Profile value"]`);
      const saveCursor = await cursor();
      await value.fill(marker);
      await value.blur();
      await waitForLiveProfileReply(page, id!, saveCursor, key, marker);
      await waitForProfileIdle(profile);
      await refresh();
      await expect(value).toHaveValue(marker);
      await page.reload();
      await waitForLiveProfileReply(page, id!, 0, key, marker);
      await expect(value).toHaveValue(marker);
      await waitForProfileIdle(profile);

      const name = profile.locator(".profile-field input").first();
      const errorCursor = await cursor();
      await name.fill("default-fixture-invalid");
      await name.blur();
      await waitForLiveProfileReply(page, id!, errorCursor, undefined, undefined, "profile_error");
      await expect(profile.locator(".profile-status.error")).toContainText("default profile cannot be renamed");
      await expect(name).toHaveValue("default-fixture-invalid");
      await expect(value).toHaveValue(marker);
      await name.fill("default");
      await name.blur();
      await refresh();
      await expect(name).toHaveValue("default");
      await expect(value).toHaveValue(marker);
      await expect(profile.locator(".profile-status.error")).toHaveCount(0);
      await profile.locator(`.profile-env-row[data-env-key="${key}"]`).scrollIntoViewIfNeeded();
      const screenshot = info.outputPath("profile-live.png");
      await pane.screenshot({
        path: screenshot,
        mask: [
          profile.locator(".profile-env-os-value"),
          profile.locator(`.profile-env-row:not([data-env-key="${key}"])`).locator("input, .profile-env-result"),
          profile.locator(".profile-field input, .profile-field textarea"),
        ],
      });
      await info.attach("Profiles live", { path: screenshot, contentType: "image/png" });
    });
  });
});

async function waitForProfileIdle(profile) {
  await expect(profile.locator(".profile-status")).toContainText(/^Active /);
}

async function waitForLiveProfileReply(page, id: string, after: number, key?: string, value?: string, kind = "profile_snapshot") {
  // Return only a boolean; real OS environment values must not become artifacts.
  await page.waitForFunction(({ id, after, key, value, kind }) =>
    (window as any).__gwtPlaywrightMessages.some((entry: any) =>
      entry.sequence > after && entry.payload.id === id && entry.payload.kind === kind &&
      (!key || entry.payload.snapshot?.profiles.some((profile: any) =>
        profile.env_vars.some((env: any) => env.key === key && (value === undefined || env.value === value))))
    ), { id, after, key, value, kind });
}

async function installProfileBackend(page, { holdSaves = false } = {}) {
  await page.addInitScript(({ holdSaves }) => {
    const recordedSends = [];
    window.__profileFixtureSends = recordedSends;

    const profileWindow = {
      id: "profile-1",
      title: "Profile",
      preset: "profile",
      geometry: { x: 96, y: 96, width: 720, height: 680 },
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
        ...Array.from({ length: 36 }, (_, index) => ({
          key: `FIXTURE_ENV_${String(index).padStart(2, "0")}`,
          value: `/fixture/${index}`,
        })),
      ],
      merged_preview: [],
    };
    const pendingSaves = [];
    window.__profileFixturePendingSaves = pendingSaves;
    window.__profileFixtureReplyNext = (error = "") => {
      const pending = pendingSaves.shift();
      if (!pending) throw new Error("No pending Profile save");
      pending.socket.replyToSave(pending.message, error);
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
          if (holdSaves) pendingSaves.push({ socket: this, message });
          else this.replyToSave(message);
        }
      }

      replyToSave(message, error = "") {
        let payload;
        if (error) {
          payload = { kind: "profile_error", id: message.id, message: error };
        } else {
          snapshot.profiles[0].env_vars = message.env_vars;
          snapshot.profiles[0].disabled_env = message.disabled_env;
          payload = { kind: "profile_snapshot", id: message.id, snapshot };
        }
        if (holdSaves) this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) }));
        else this.emit(payload);
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
  }, { holdSaves });
}
