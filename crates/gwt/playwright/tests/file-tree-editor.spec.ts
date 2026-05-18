/* SPEC-2006 / SPEC-2009 Phase 1 + Phase 2 — Headed-browser E2E coverage
 * for the File Tree window.
 *
 * Runs the embedded gwt frontend with a deterministic FixtureWebSocket so
 * the full Worktree Picker → Split Viewer → Text/Hex editor → Save/
 * Conflict/Discard flow can be exercised without booting the real gwt
 * binary or touching disk. The backend stub mirrors the Phase 2 wire
 * contract (`list_file_tree_worktrees`, `select_file_tree_worktree`,
 * `load_file_tree`, `load_file_content`, `save_file_content`) and records
 * every outbound payload so test bodies can assert on them.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("File Tree v2 editor E2E", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1600, height: 900 },
  });

  test("worktree picker → text edit → Save → external conflict → discard flow", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installFileTreeBackend(page);

    await page.goto(APP_URL);

    // Phase 1 FR-022: Worktree Picker auto-opens on File Tree window create.
    const picker = page.locator("#file-tree-worktree-picker-modal");
    await expect(picker).toHaveAttribute("aria-hidden", "false");
    const pickerRows = picker.locator(".worktree-picker-row");
    await expect(pickerRows).toHaveCount(2);

    // Select the workspace entry so we drive a non-bare worktree path.
    await pickerRows.nth(1).click();
    await expect(picker).toHaveAttribute("aria-hidden", "true");

    // Phase 1 FR-024: tree pane lists the files served by the stub.
    const treeList = page
      .locator(".workspace-window.surface-file-tree .file-tree-list")
      .first();
    await expect(treeList.locator(".file-tree-row")).toHaveCount(3);

    // Open the text file (Phase 1 FR-026: viewer mode is auto text).
    await treeList.locator(".file-tree-row", { hasText: "README.md" }).click();
    const viewer = page.locator(
      ".workspace-window.surface-file-tree .file-tree-viewer",
    );
    const textarea = viewer.locator("textarea.file-tree-viewer-editor");
    await expect(textarea).toBeVisible();
    await expect(textarea).toHaveValue("hello\n");

    // Phase 2 FR-039: header shows encoding / newline / size badges.
    const meta = viewer.locator(".file-tree-viewer-meta");
    await expect(meta).toContainText("UTF-8");
    await expect(meta).toContainText("LF");

    // Save button starts disabled because the viewer is not yet dirty.
    const saveBtn = viewer.locator("button.file-tree-viewer-save");
    await expect(saveBtn).toBeDisabled();
    await expect(viewer.locator(".file-tree-viewer-dirty")).toHaveCount(0);

    // Phase 2 FR-032: editing flips dirty marker + enables Save.
    await textarea.fill("hello world\n");
    await expect(viewer.locator(".file-tree-viewer-dirty")).toHaveText("●");
    await expect(saveBtn).toBeEnabled();

    // Phase 2 FR-033: Ctrl+S triggers Save and backend roundtrip clears
    // the dirty marker.
    await textarea.press("ControlOrMeta+s");
    await expect(saveBtn).toBeDisabled();
    await expect(viewer.locator(".file-tree-viewer-dirty")).toHaveCount(0);
    await expect(viewer.locator(".file-tree-viewer-saved")).toHaveText("Saved");

    // The stub recorded the SaveFileContent payload with the right mode +
    // encoding so the round-trip respected Phase 2 wire contract.
    const calls = await page.evaluate(() => window.__fileTreeCalls);
    const saves = calls.filter((c) => c.kind === "save_file_content");
    expect(saves).toHaveLength(1);
    expect(saves[0]).toMatchObject({
      mode: "text",
      encoding: "utf-8",
      newline: "lf",
      has_bom: false,
      text: "hello world\n",
    });

    // Phase 2 FR-035: simulate an external mutation, edit again, Save →
    // Conflict modal must appear.
    await page.evaluate(() => {
      window.__fileTreeFixture.simulateExternalEdit({
        path: "README.md",
        mtime: 999_999,
        size: 999,
      });
    });
    await textarea.fill("hello conflicting\n");
    await saveBtn.click();
    const conflictModal = page.locator("#file-tree-conflict-modal");
    await expect(conflictModal).toHaveAttribute("aria-hidden", "false");
    await expect(conflictModal).toContainText("changed externally");
    await conflictModal.getByRole("button", { name: "Cancel" }).click();
    await expect(conflictModal).toHaveAttribute("aria-hidden", "true");

    // Phase 2 FR-034: dirty edit blocks file switch with Discard modal.
    await treeList.locator(".file-tree-row", { hasText: "data.bin" }).click();
    const discardModal = page.locator("#file-tree-discard-modal");
    await expect(discardModal).toHaveAttribute("aria-hidden", "false");
    await expect(discardModal).toContainText("Unsaved changes");
    await discardModal.getByRole("button", { name: "Discard" }).click();
    await expect(discardModal).toHaveAttribute("aria-hidden", "true");
    // After Discard the queued navigation runs: binary notice appears.
    await expect(viewer.locator(".file-tree-viewer-notice")).toContainText(
      "Cannot display as text",
    );
  });

  test("read-only file disables edit affordances", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installFileTreeBackend(page);

    await page.goto(APP_URL);
    const picker = page.locator("#file-tree-worktree-picker-modal");
    await expect(picker).toHaveAttribute("aria-hidden", "false");
    await picker.locator(".worktree-picker-row").nth(0).click();

    const treeList = page
      .locator(".workspace-window.surface-file-tree .file-tree-list")
      .first();
    await treeList.locator(".file-tree-row", { hasText: "readonly.txt" }).click();

    const viewer = page.locator(
      ".workspace-window.surface-file-tree .file-tree-viewer",
    );
    await expect(viewer.locator(".file-tree-viewer-readonly")).toHaveText(
      "read-only",
    );
    await expect(
      viewer.locator("textarea.file-tree-viewer-editor"),
    ).toBeDisabled();
    await expect(viewer.locator("button.file-tree-viewer-save")).toBeDisabled();
  });
});

async function installFileTreeBackend(page) {
  await page.addInitScript(() => {
    // Test asset: 3-entry directory + per-file content fixtures so the
    // backend stub can answer load_file_tree / load_file_content without
    // touching disk.
    const FIXTURE_FILES = {
      "README.md": {
        kind: "text",
        text: "hello\n",
        encoding: "utf-8",
        newline: "lf",
        has_bom: false,
        read_only: false,
        mtime: 1_700_000_000,
        size: 6,
      },
      "readonly.txt": {
        kind: "text",
        text: "do not edit\n",
        encoding: "utf-8",
        newline: "lf",
        has_bom: false,
        read_only: true,
        mtime: 1_700_000_100,
        size: 12,
      },
      "data.bin": {
        kind: "binary",
        mtime: 1_700_000_200,
        size: 4,
        bytes_b64: "AAECAw==", // 00 01 02 03
        read_only: false,
      },
    };

    const fixtureState = {
      externalEdits: new Map(),
    };
    const recordedCalls = [];
    window.__fileTreeCalls = recordedCalls;
    window.__fileTreeFixture = {
      simulateExternalEdit({ path, mtime, size }) {
        fixtureState.externalEdits.set(path, { mtime, size });
      },
    };

    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  id: "ft-1",
                  title: "Files",
                  preset: "file_tree",
                  geometry: { x: 96, y: 96, width: 1200, height: 700 },
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
                },
              ],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
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
        recordedCalls.push(message);
        switch (message.kind) {
          case "frontend_ready":
            this.emit(workspaceState);
            break;
          case "list_file_tree_worktrees":
            this.emit({
              kind: "file_tree_worktrees",
              id: message.id,
              entries: [
                {
                  id: "wt-main",
                  kind: "bare_main",
                  path: "/fixture",
                  label: "main repository",
                  branch: null,
                  is_active: true,
                },
                {
                  id: "wt-feature",
                  kind: "workspace",
                  path: "/fixture/work/feature",
                  label: "feature/a",
                  branch: "feature/a",
                  is_active: false,
                },
              ],
            });
            break;
          case "select_file_tree_worktree":
            this.emit({
              kind: "file_tree_worktree_selected",
              id: message.id,
              worktree_id: message.worktree_id,
            });
            break;
          case "load_file_tree":
            this.emit({
              kind: "file_tree_entries",
              id: message.id,
              path: message.path || "",
              entries: [
                { name: "README.md", path: "README.md", kind: "file" },
                { name: "readonly.txt", path: "readonly.txt", kind: "file" },
                { name: "data.bin", path: "data.bin", kind: "file" },
              ],
            });
            break;
          case "load_file_content": {
            const file = FIXTURE_FILES[message.path];
            if (!file) {
              this.emit({
                kind: "file_content_error",
                id: message.id,
                path: message.path,
                error_kind: "io_error",
                message: "missing fixture",
              });
              return;
            }
            if (message.mode === "text") {
              if (file.kind === "binary") {
                this.emit({
                  kind: "file_content_error",
                  id: message.id,
                  path: message.path,
                  error_kind: "binary_not_text",
                  message: "Cannot decode as text",
                  size: file.size,
                });
                return;
              }
              this.emit({
                kind: "file_content_text",
                id: message.id,
                path: message.path,
                encoding: file.encoding,
                text: file.text,
                total_size: file.size,
                mtime: file.mtime,
                has_bom: file.has_bom,
                newline: file.newline,
                read_only: file.read_only,
              });
            } else {
              this.emit({
                kind: "file_content_hex",
                id: message.id,
                path: message.path,
                offset: 0,
                bytes_b64: file.bytes_b64 || "",
                total_size: file.size,
                mtime: file.mtime,
                read_only: file.read_only || false,
              });
            }
            break;
          }
          case "save_file_content": {
            const expected = {
              mtime: message.expected_mtime,
              size: message.expected_size,
            };
            const external = fixtureState.externalEdits.get(message.path);
            if (
              external &&
              (external.mtime !== expected.mtime || external.size !== expected.size)
            ) {
              this.emit({
                kind: "file_content_save_error",
                id: message.id,
                path: message.path,
                mode: message.mode,
                error_kind: "conflict",
                message: "File changed externally",
                current_mtime: external.mtime,
                current_size: external.size,
              });
              return;
            }
            const updatedMtime = Date.now();
            const updatedSize =
              message.mode === "text"
                ? new TextEncoder().encode(message.text || "").length
                : expected.size;
            this.emit({
              kind: "file_content_saved",
              id: message.id,
              path: message.path,
              mode: message.mode,
              new_mtime: updatedMtime,
              new_size: updatedSize,
              encoding_fallback: 0,
            });
            break;
          }
          default:
            break;
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
