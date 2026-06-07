import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Browser file drop attachments", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("dropping a file on an Agent window body uploads and sends uploaded attach_files", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(APP_URL);
    await expect(page.locator(".workspace-window[data-id='agent-1']")).toBeVisible();

    await dropTextFileOn(page, ".workspace-window[data-id='agent-1'] .window-body", {
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
    await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(APP_URL);
    await expect(page.locator(".workspace-window[data-id='shell-1']")).toBeVisible();

    await dropTextFileOn(page, ".workspace-window[data-id='shell-1'] .window-body", {
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
    await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(APP_URL);
    await expect(page.locator(".workspace-window[data-id='shell-1']")).toBeVisible();

    await dropTextFileOn(page, ".workspace-window[data-id='shell-1'] .terminal-root", {
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
    await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(APP_URL);
    await expect(page.locator(".workspace-window[data-id='agent-1']")).toBeVisible();

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
    }, ".workspace-window[data-id='agent-1'] .window-body");

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
    await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(APP_URL);
    await expect(page.locator(".workspace-window[data-id='agent-1']")).toBeVisible();
    await expect(page.locator(".workspace-window[data-id='shell-1']")).toBeVisible();

    await dropTextFileOn(page, ".workspace-window[data-id='agent-1'] .window-body", {
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
      ".workspace-window[data-id='agent-1'] .attachment-progress",
    );
    await expect(agentProgress).toBeVisible();
    await expect(agentProgress).toContainText("資料 日本語.txt");
    await expect(agentProgress.locator(".attachment-progress__cancel")).toHaveCount(0);
    await expect(
      page.locator(".workspace-window[data-id='shell-1'] .attachment-progress"),
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

  test("pasting an image on an Agent terminal uploads and shows progress", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installFileDropBackend(page);

    await page.goto(APP_URL);
    await expect(page.locator(".workspace-window[data-id='agent-1']")).toBeVisible();

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
    }, ".workspace-window[data-id='agent-1'] .terminal-root");

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
    await installEmbeddedRoutes(page);
    await installFileDropBackend(page, { failUploads: true });

    await page.goto(APP_URL);
    await expect(page.locator(".workspace-window[data-id='agent-1']")).toBeVisible();

    await dropTextFileOn(page, ".workspace-window[data-id='agent-1'] .window-body", {
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
