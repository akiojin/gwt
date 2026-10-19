import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const fileDropBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
const fileDropAppUrl = fileDropBase ? new URL(new URL(APP_URL).pathname, fileDropBase).href : APP_URL;

test.describe("Browser file drop attachments", () => {
  const browserErrors = new WeakMap<Page, string[]>();
  test.beforeEach(async ({ page }, testInfo) => {
    const errors: string[] = [];
    browserErrors.set(page, errors);
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  });
  test.afterEach(async ({ page }, testInfo) => {
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.includes("light") ? "light" : "dark");
    expect(browserErrors.get(page), "console and page errors").toEqual([]);
  });
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("runtime status receivers preserve details and isolate windows", async ({ page }, testInfo) => {
    if (!fileDropBase) await installEmbeddedRoutes(page);
    await installFileDropBackend(page);
    await page.goto(fileDropAppUrl);
    const tile = page.locator(".agent-tile[data-agent-id='agent-1']");
    await expect(tile).toBeVisible();
    const tileNode = await tile.elementHandle();
    const agent = page.locator(".workspace-window[data-id='agent-1']");
    const chip = agent.locator(".status-chip");
    const chipNode = await chip.elementHandle();
    const message = agent.locator(".overlay-message");
    const shellChip = page.locator(".workspace-window[data-id='shell-1'] .status-chip");
    const shellBefore = await shellChip.getAttribute("class");
    const emit = (event: Record<string, unknown>) => page.evaluate(event => {
      window.__emitFileDropBackendEvent?.(event);
    }, event);

    // Controlled inbound frames exercise the checkout receiver and real renderers.
    await emit({ kind: "terminal_status", id: "agent-1", status: "error", detail: "Launch <failed> & retry" });
    await expect(chip).toHaveClass(/\berror\b/);
    await expect(chip).toHaveAttribute("title", /Launch <failed> & retry/);
    await expect(message).toHaveText("Launch <failed> & retry");
    await expect(shellChip).toHaveAttribute("class", shellBefore!);
    await emit({ kind: "window_state", window_id: "agent-1", state: "stopped" });
    await expect(chip).toHaveClass(/\bstopped\b/);
    await expect(message).toHaveText("Launch <failed> & retry");
    await expect(shellChip).toHaveAttribute("class", shellBefore!);
    await emit({ kind: "window_state", window_id: "agent-1", state: "running" });
    await expect(chip).toHaveClass(/\brunning\b/);
    await expect(message).toBeEmpty();
    await expect(chip).not.toHaveAttribute("title", /Launch <failed> & retry/);
    await expect(shellChip).toHaveAttribute("class", shellBefore!);
    await emit({ kind: "terminal_status", id: "agent-1", status: "starting", detail: "Preparing runtime" });
    await expect(chip).toHaveClass(/\bstarting\b/);
    await expect(message).toHaveText("Preparing runtime");
    await expect(shellChip).toHaveAttribute("class", shellBefore!);
    await emit({ kind: "terminal_status", id: "agent-1", status: "running", detail: null });
    await expect(chip).toHaveClass(/\brunning\b/);
    await expect(message).toBeEmpty();
    await expect(shellChip).toHaveAttribute("class", shellBefore!);
    expect(await chip.evaluate((node, original) => node === original, chipNode)).toBe(true);
    expect(await tile.evaluate((node, original) => node === original, tileNode)).toBe(true);
    await expect(agent.locator(".terminal-overlay")).toBeHidden();
    await page.screenshot({ path: testInfo.outputPath("terminal-status.png") });
  });

  test("launch progress updates only its window message and copy state", async ({ page }, testInfo) => {
    if (!fileDropBase) await installEmbeddedRoutes(page);
    await installFileDropBackend(page);
    await page.goto(fileDropAppUrl);
    await expect(page.locator(".agent-tile[data-agent-id='agent-1']")).toBeVisible();
    await page.evaluate(() => window.__emitFileDropBackendEvent?.({
      kind: "terminal_status", id: "agent-1", status: "starting", detail: null,
    }));
    // Legacy overlays stay parked and hidden; progress must not cover the TTY.
    const overlay = page.locator(".workspace-window[data-id='agent-1'] .terminal-overlay");
    await expect(overlay).toHaveCount(1);
    await expect(overlay).toBeHidden();
    const message = overlay.locator(".overlay-message");
    const copy = overlay.locator(".overlay-copy-button");
    const messageNode = await message.elementHandle();
    const otherBefore = await page.locator(".workspace-window[data-id='shell-1'] .overlay-message").textContent();
    const emit = (id: string, text: string) => page.evaluate(({ id, text }) => {
      window.__emitFileDropBackendEvent?.({ kind: "launch_progress", id, message: text });
    }, { id, text });

    await emit("agent-1", "Preparing <workspace> & runtime");
    await expect(message).toHaveText("Preparing <workspace> & runtime");
    expect(await copy.evaluate(node => (node as HTMLButtonElement).hidden)).toBe(false);
    await expect(copy).toBeEnabled();
    await emit("agent-1", "");
    await expect(message).toBeEmpty();
    expect(await copy.evaluate(node => (node as HTMLButtonElement).hidden)).toBe(true);
    await expect(copy).toBeDisabled();
    await emit("missing-window", "Ignore this unknown window");
    await emit("agent-1", "Starting Agent — progress received");
    await expect(message).toHaveText("Starting Agent — progress received");
    expect(await copy.evaluate(node => (node as HTMLButtonElement).hidden)).toBe(false);
    await expect(copy).toBeEnabled();
    expect(await message.evaluate((node, original) => node === original, messageNode)).toBe(true);
    await expect(page.locator(".workspace-window[data-id='shell-1'] .overlay-message")).toHaveText(otherBefore || "");
    await expect(overlay).toBeHidden();
    await page.screenshot({ path: testInfo.outputPath("launch-progress.png") });
  });

  test("dropping a file on an Agent window body uploads and sends uploaded attach_files", async ({
    page,
  }) => {
    if (!fileDropBase) await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(fileDropAppUrl);
    await expect(page.locator(".agent-tile[data-agent-id='agent-1']")).toBeVisible();

    await dropTextFileOn(page, ".agent-tile[data-agent-id='agent-1'] .agent-tile__header", {
      name: "notes.txt",
      type: "text/plain",
      content: "hello from browser drop\n",
    });

    const attach = await waitForAttachFiles(page);
    expect(attach).toMatchObject({
      kind: "attach_files",
      id: "agent-1",
    });
    expect(attach.files).toHaveLength(1);
    expect(attach.files[0]).toMatchObject({
      source: "uploaded",
      upload_id: "upload-1",
      filename: "notes.txt",
      mime_type: "text/plain",
      size: "hello from browser drop\n".length,
    });
    await expect.poll(() => page.evaluate(() => window.__attachmentUploads)).toEqual([
      { name: "notes.txt", size: "hello from browser drop\n".length },
    ]);
    await expect.poll(() => page.evaluate(() => window.__fileDropAlerts)).toEqual([]);
  });

  test("dropping a file on a non-Agent terminal alerts without attach_files", async ({
    page,
  }) => {
    if (!fileDropBase) await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(fileDropAppUrl);
    await expect(page.locator(".agent-tile[data-agent-id='agent-1']")).toBeVisible();
    await page.getByRole("tab", { name: "All agents", exact: true }).click();
    await expect(page.locator(".agent-tile[data-agent-id='shell-1']")).toBeVisible();

    await dropTextFileOn(page, ".agent-tile[data-agent-id='shell-1'] .agent-tile__header", {
      name: "notes.txt",
      type: "text/plain",
      content: "shell drop\n",
    });

    await expect
      .poll(() => page.evaluate(() => window.__fileDropAlerts))
      .toContainEqual(expect.stringContaining("Agent"));
    expect(await attachFilesMessages(page)).toEqual([]);
  });

  test("dropping a file on a non-Agent terminal root alerts without attach_files", async ({
    page,
  }) => {
    if (!fileDropBase) await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(fileDropAppUrl);
    await expect(page.locator(".agent-tile[data-agent-id='agent-1']")).toBeVisible();
    await page.getByRole("tab", { name: "All agents", exact: true }).click();
    await expect(page.locator(".agent-tile[data-agent-id='shell-1']")).toBeVisible();

    await dropTextFileOn(page, ".agent-tile[data-agent-id='shell-1'] .terminal-root", {
      name: "notes.txt",
      type: "text/plain",
      content: "shell root drop\n",
    });

    await expect
      .poll(() => page.evaluate(() => window.__fileDropAlerts))
      .toContainEqual(expect.stringContaining("Agent"));
    expect(await attachFilesMessages(page)).toEqual([]);
  });

  test("large browser drops show progress and do not use the old size alert", async ({ page }) => {
    if (!fileDropBase) await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(fileDropAppUrl);
    await expect(page.locator(".agent-tile[data-agent-id='agent-1']")).toBeVisible();

    await page.evaluate((selector) => {
      const target = document.querySelector(selector);
      if (!target) throw new Error(`drop target not found: ${selector}`);
      const transfer = new DataTransfer();
      transfer.items.add(
        new File([new Uint8Array(10 * 1024 * 1024 + 1)], "large.bin", {
          type: "application/octet-stream",
        }),
      );
      target.dispatchEvent(
        new DragEvent("dragover", {
          bubbles: true,
          cancelable: true,
          dataTransfer: transfer,
        }),
      );
      target.dispatchEvent(
        new DragEvent("drop", {
          bubbles: true,
          cancelable: true,
          dataTransfer: transfer,
        }),
      );
    }, ".agent-tile[data-agent-id='agent-1'] .agent-tile__header");

    const progress = page.locator(".attachment-progress");
    expect(
      await page.evaluate(() => !document.querySelector(".attachment-progress")?.hidden),
    ).toBe(true);
    await expect(progress).toBeVisible();
    await expect(progress).toContainText("Uploading 1 file");
    await expect(progress.locator('[role="progressbar"]')).toHaveAttribute(
      "aria-valuenow",
      "50",
    );

    await page.evaluate(() => window.__finishAttachmentUpload?.());

    const attach = await waitForAttachFiles(page);
    expect(attach.files[0]).toMatchObject({
      source: "uploaded",
      upload_id: "upload-1",
      filename: "large.bin",
      mime_type: "application/octet-stream",
      size: 10 * 1024 * 1024 + 1,
    });
    await expect.poll(() => page.evaluate(() => window.__fileDropAlerts)).toEqual([]);
  });

  test("drop progress is scoped to the target Agent window and keeps Japanese filenames", async ({
    page,
  }) => {
    if (!fileDropBase) await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(fileDropAppUrl);
    await expect(page.locator(".agent-tile[data-agent-id='agent-1']")).toBeVisible();
    await page.getByRole("tab", { name: "All agents", exact: true }).click();
    await expect(page.locator(".agent-tile[data-agent-id='shell-1']")).toBeVisible();

    await dropTextFileOn(page, ".agent-tile[data-agent-id='agent-1'] .agent-tile__header", {
      name: "資料 日本語.txt",
      type: "text/plain",
      content: "nihongo filename\n",
    });

    const attach = await waitForAttachFiles(page);
    expect(attach).toMatchObject({
      kind: "attach_files",
      id: "agent-1",
    });
    expect(attach.operation_id).toEqual(expect.stringMatching(/^attachment-/));
    expect(attach.files[0]).toMatchObject({
      filename: "資料 日本語.txt",
    });

    const agentProgress = page.locator(
      ".agent-tile[data-agent-id='agent-1'] .attachment-progress",
    );
    await expect(agentProgress).toBeVisible();
    const tileBounds = (await page.locator(".agent-tile[data-agent-id='agent-1']").boundingBox())!;
    const progressBounds = (await agentProgress.boundingBox())!;
    expect(progressBounds.x).toBeGreaterThanOrEqual(tileBounds.x);
    expect(progressBounds.y).toBeGreaterThanOrEqual(tileBounds.y);
    expect(progressBounds.x + progressBounds.width).toBeLessThanOrEqual(tileBounds.x + tileBounds.width);
    expect(progressBounds.y + progressBounds.height).toBeLessThanOrEqual(tileBounds.y + tileBounds.height);
    await expect(agentProgress).toContainText("資料 日本語.txt");
    await expect(agentProgress.locator(".attachment-progress__cancel")).toHaveCount(0);
    await expect(
      page.locator(".agent-tile[data-agent-id='shell-1'] .attachment-progress"),
    ).toHaveCount(0);

    await page.evaluate((operationId) => {
      window.__emitFileDropBackendEvent?.({
        kind: "attachment_progress",
        id: "agent-1",
        operation_id: operationId,
        phase: "staging",
        file_index: 0,
        file_count: 1,
        filename: "資料 日本語.txt",
        bytes_done: 8,
        bytes_total: 16,
        message: null,
      });
    }, attach.operation_id);
    await expect(agentProgress).toContainText("Staging");
    await expect(agentProgress.locator('[role="progressbar"]')).toHaveAttribute(
      "aria-valuenow",
      "50",
    );

    await page.evaluate((operationId) => {
      window.__emitFileDropBackendEvent?.({
        kind: "attachment_progress",
        id: "agent-1",
        operation_id: operationId,
        phase: "attached",
        file_index: 0,
        file_count: 1,
        filename: "資料 日本語.txt",
        bytes_done: 16,
        bytes_total: 16,
        message: null,
      });
    }, attach.operation_id);
    await expect(agentProgress).toContainText("Attached");
    await expect.poll(() => page.evaluate(() => window.__fileDropAlerts)).toEqual([]);
  });

  test("the attachment surface auto-hides after the backend reports Attached (Issue #3315)", async ({
    page,
  }, testInfo) => {
    if (!fileDropBase) await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(fileDropAppUrl);
    await expect(page.locator(".agent-tile[data-agent-id='agent-1']")).toBeVisible();

    const agentProgress = page.locator(
      ".agent-tile[data-agent-id='agent-1'] .attachment-progress",
    );

    await emitAttachmentProgress(page, { operationId: "", phase: "failed", message: "ignored" });
    await expect(agentProgress).toHaveCount(0);
    await emitAttachmentProgress(page, {
      operationId: "op-attach",
      phase: "staging",
      filename: "/workspace/notes.txt",
      bytesDone: 8,
      bytesTotal: 16,
    });
    await expect(agentProgress).toBeVisible();
    await expect(agentProgress).toContainText("Staging");
    await expect(agentProgress).toContainText("notes.txt");
    await expect(agentProgress).not.toContainText("/workspace/");
    const surfaceNode = await agentProgress.elementHandle();
    const progressbar = agentProgress.locator('[role="progressbar"]');
    await expect(progressbar).toHaveAttribute("aria-valuenow", "50");
    await emitAttachmentProgress(page, {
      operationId: "op-attach", phase: "injecting", filename: "notes.txt", bytesDone: 12, bytesTotal: 16,
    });
    await expect(agentProgress).toContainText("Injecting");
    await expect(progressbar).toHaveAttribute("aria-valuenow", "75");
    expect(await agentProgress.evaluate((node, original) => node === original, surfaceNode)).toBe(true);
    await expect(page.locator(".agent-tile[data-agent-id='shell-1'] .attachment-progress")).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath("attachment-progress.png") });

    await emitAttachmentProgress(page, {
      operationId: "op-attach",
      phase: "attached",
      filename: "notes.txt",
      bytesDone: 16,
      bytesTotal: 16,
    });
    await expect(agentProgress).toContainText("Attached");

    // The Attached state clears itself shortly after (~0.7s). This is the
    // regression the backend delivery fix guarantees: a delivered terminal
    // Attached must never leave the surface stuck at `Queued · 100%`.
    await expect(agentProgress).toBeHidden();
    await expect(
      page.locator(".agent-tile[data-agent-id='shell-1'] .attachment-progress"),
    ).toHaveCount(0);
  });

  test("a Failed phase persists until the next attachment replaces it (Issue #3315)", async ({
    page,
  }, testInfo) => {
    if (!fileDropBase) await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(fileDropAppUrl);
    await expect(page.locator(".agent-tile[data-agent-id='agent-1']")).toBeVisible();

    const agentProgress = page.locator(
      ".agent-tile[data-agent-id='agent-1'] .attachment-progress",
    );

    await emitAttachmentProgress(page, {
      operationId: "op-fail",
      phase: "failed",
      filename: "notes.txt",
      bytesDone: 4,
      bytesTotal: 16,
      message: "disk full",
    });
    await expect(agentProgress).toBeVisible();
    await expect(agentProgress).toContainText("disk full");
    await expect(agentProgress).toHaveAttribute("data-state", "error");
    const surfaceNode = await agentProgress.elementHandle();

    // Unlike Attached, a failure has no auto-hide timer — it must stay visible
    // past the success dwell so the user can read it.
    await page.waitForTimeout(900);
    await expect(agentProgress).toBeVisible();
    await expect(agentProgress).toContainText("disk full");

    // The next attachment operation reuses the same surface and replaces the
    // error in place.
    await emitAttachmentProgress(page, {
      operationId: "op-next",
      phase: "staging",
      filename: "report.pdf",
      bytesDone: 8,
      bytesTotal: 32,
    });
    await expect(agentProgress).toBeVisible();
    await expect(agentProgress).toContainText("Staging");
    await expect(agentProgress).toContainText("report.pdf");
    await expect(agentProgress).not.toContainText("disk full");
    expect(await agentProgress.evaluate((node, original) => node === original, surfaceNode)).toBe(true);
    await expect(page.locator(".agent-tile[data-agent-id='shell-1'] .attachment-progress")).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath("attachment-replacement.png") });
  });

  test("pasting an image on an Agent terminal uploads and shows progress", async ({ page }) => {
    if (!fileDropBase) await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(fileDropAppUrl);
    await expect(page.locator(".agent-tile[data-agent-id='agent-1']")).toBeVisible();

    await page.evaluate((selector) => {
      const target = document.querySelector(selector);
      if (!target) throw new Error(`paste target not found: ${selector}`);
      const transfer = new DataTransfer();
      transfer.items.add(
        new File([new Uint8Array(2 * 1024 * 1024)], "paste.png", {
          type: "image/png",
        }),
      );
      target.dispatchEvent(
        new ClipboardEvent("paste", {
          bubbles: true,
          cancelable: true,
          clipboardData: transfer,
        }),
      );
    }, ".agent-tile[data-agent-id='agent-1'] .terminal-root");

    const progress = page.locator(".attachment-progress");
    expect(
      await page.evaluate(() => !document.querySelector(".attachment-progress")?.hidden),
    ).toBe(true);
    await expect(progress).toBeVisible();
    await expect(progress).toContainText("Uploading 1 file");
    await expect(progress.locator('[role="progressbar"]')).toHaveAttribute(
      "aria-valuenow",
      "50",
    );

    await page.evaluate(() => window.__finishAttachmentUpload?.());

    const paste = await waitForPasteImageUploaded(page);
    expect(paste).toMatchObject({
      kind: "paste_image_uploaded",
      id: "agent-1",
      upload_id: "upload-1",
      mime_type: "image/png",
      filename: "paste.png",
      size: 2 * 1024 * 1024,
    });
    expect(paste).not.toHaveProperty("data_base64");
    await expect.poll(() => page.evaluate(() => window.__fileDropAlerts)).toEqual([]);
  });

  test("browser upload failures show an error without attach_files", async ({ page }) => {
    if (!fileDropBase) await installEmbeddedRoutes(page);
    await installFileDropBackend(page, { failUploads: true });

    await page.goto(fileDropAppUrl);
    await expect(page.locator(".agent-tile[data-agent-id='agent-1']")).toBeVisible();

    await dropTextFileOn(page, ".agent-tile[data-agent-id='agent-1'] .agent-tile__header", {
      name: "notes.txt",
      type: "text/plain",
      content: "upload failure\n",
    });

    await expect
      .poll(() => page.evaluate(() => window.__fileDropAlerts))
      .toContainEqual(expect.stringContaining("Could not upload"));
    await expect(page.locator(".attachment-progress")).toContainText("Could not upload");
    expect(await attachFilesMessages(page)).toEqual([]);
  });
});

async function dropTextFileOn(
  page,
  selector: string,
  file: { name: string; type: string; content: string },
) {
  await page.evaluate(
    ({ selector, file }) => {
      const target = document.querySelector(selector);
      if (!target) throw new Error(`drop target not found: ${selector}`);
      const transfer = new DataTransfer();
      transfer.items.add(new File([file.content], file.name, { type: file.type }));
      target.dispatchEvent(
        new DragEvent("dragover", {
          bubbles: true,
          cancelable: true,
          dataTransfer: transfer,
        }),
      );
      target.dispatchEvent(
        new DragEvent("drop", {
          bubbles: true,
          cancelable: true,
          dataTransfer: transfer,
        }),
      );
    },
    { selector, file },
  );
}

async function emitAttachmentProgress(
  page,
  event: {
    operationId: string;
    phase: string;
    filename?: string;
    bytesDone?: number;
    bytesTotal?: number;
    message?: string | null;
    windowId?: string;
  },
) {
  await page.evaluate((event) => {
    window.__emitFileDropBackendEvent?.({
      kind: "attachment_progress",
      id: event.windowId ?? "agent-1",
      operation_id: event.operationId,
      phase: event.phase,
      file_index: 0,
      file_count: 1,
      filename: event.filename ?? "file",
      bytes_done: event.bytesDone ?? 0,
      bytes_total: event.bytesTotal ?? 0,
      message: event.message ?? null,
    });
  }, event);
}

async function waitForAttachFiles(page) {
  await expect
    .poll(() => attachFilesMessages(page))
    .toHaveLength(1);
  const messages = await attachFilesMessages(page);
  return messages[0];
}

async function waitForPasteImageUploaded(page) {
  await expect
    .poll(() => pasteImageUploadedMessages(page))
    .toHaveLength(1);
  const messages = await pasteImageUploadedMessages(page);
  return messages[0];
}

async function attachFilesMessages(page) {
  return page.evaluate(() =>
    (window.__fileDropSent || []).filter((message) => message.kind === "attach_files"),
  );
}

async function pasteImageUploadedMessages(page) {
  return page.evaluate(() =>
    (window.__fileDropSent || []).filter((message) => message.kind === "paste_image_uploaded"),
  );
}

async function installFileDropBackend(page, options: { failUploads?: boolean } = {}) {
  await page.addInitScript((options) => {
    window.__fileDropSent = [];
    window.__fileDropAlerts = [];
    window.__attachmentUploads = [];
    window.__attachmentUploadSequence = 0;
    window.__emitFileDropBackendEvent = null;
    window.alert = (message) => {
      window.__fileDropAlerts.push(String(message));
    };
    window.__gwtAttachmentUploader = ({ file, onProgress, signal }) =>
      new Promise((resolve, reject) => {
        const uploadNumber = ++window.__attachmentUploadSequence;
        window.__attachmentUploads.push({ name: file.name, size: file.size });
        const total = file.size || 0;
        const halfway = Math.floor(total / 2);
        onProgress?.({ loaded: halfway, total });
        if (options.failUploads) {
          setTimeout(() => reject(new Error("forced upload failure")), 0);
          return;
        }
        const finish = () => {
          onProgress?.({ loaded: total, total });
          resolve({
            upload_id: `upload-${uploadNumber}`,
            filename: file.name || "file",
            mime_type: file.type || null,
            size: file.size,
          });
        };
        window.__finishAttachmentUpload = finish;
        signal?.addEventListener("abort", () => reject(new Error("aborted")));
        if (file.size <= 1024 * 1024) {
          setTimeout(finish, 0);
        }
      });

    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "File Drop Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                fileDropWindow({
                  id: "agent-1",
                  title: "Codex",
                  preset: "agent",
                  x: 180,
                  y: 120,
                  z: 2,
                  agentId: "codex",
                }),
                fileDropWindow({
                  id: "shell-1",
                  title: "Shell",
                  preset: "shell",
                  x: 180,
                  y: 520,
                  z: 1,
                  agentId: null,
                }),
              ],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };

    function fileDropWindow({ id, title, preset, x, y, z, agentId }) {
      return {
        id,
        title,
        preset,
        geometry: { x, y, width: 720, height: 300 },
        geometry_revision: 0,
        z_index: z,
        status: "running",
        minimized: false,
        maximized: false,
        pre_maximize_geometry: null,
        persist: true,
        purpose_title: null,
        dynamic_title: null,
        dynamic_title_detail: null,
        agent_id: agentId,
        agent_color: agentId ? "cyan" : null,
        tab_group_id: null,
        tab_group_active: false,
      };
    }

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        window.__emitFileDropBackendEvent = (payload) => this.emit(payload);
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(workspaceState);
        }, 0);
      }

      send(raw) {
        let message;
        try {
          message = JSON.parse(raw);
        } catch {
          return;
        }
        window.__fileDropSent.push(message);
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
  }, options);
}
