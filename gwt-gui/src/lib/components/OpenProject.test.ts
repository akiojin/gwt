import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();
const openDialogMock = vi.fn();
const listenMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: (...args: unknown[]) => listenMock(...args),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => openDialogMock(...args),
}));

async function renderOpenProject(props?: any) {
  const { default: OpenProject } = await import("./OpenProject.svelte");
  return render(OpenProject, {
    props: { onOpen: vi.fn(), ...props },
  });
}

describe("OpenProject", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    openDialogMock.mockReset();
    listenMock.mockReset();
    listenMock.mockResolvedValue(() => {});
    cleanup();
  });

  afterEach(() => {
    cleanup();
  });

  it("renders title and subtitle", async () => {
    invokeMock.mockResolvedValue([]);
    const rendered = await renderOpenProject();

    expect(rendered.getByText("gwt")).toBeTruthy();
    expect(rendered.getByText("Git Worktree Manager")).toBeTruthy();
  });

  it("renders Open Project and New Project buttons", async () => {
    invokeMock.mockResolvedValue([]);
    const rendered = await renderOpenProject();

    expect(rendered.getByText("Open Project...")).toBeTruthy();
    expect(rendered.getByText("New Project")).toBeTruthy();
  });

  it("does not show Recent Projects section when empty", async () => {
    invokeMock.mockResolvedValue([]);
    const rendered = await renderOpenProject();

    expect(rendered.queryByText("Recent Projects")).toBeNull();
  });

  it("shows recent project paths and names for multiple items", async () => {
    // Flush pending microtasks from previous test's $effect cleanup
    await new Promise((r) => setTimeout(r, 10));

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") {
        return [
          { path: "/home/user/myproject", lastOpened: "2026-01-01T00:00:00Z" },
          { path: "/home/user/other-repo", lastOpened: "2026-01-02T00:00:00Z" },
        ];
      }
      return null;
    });

    const rendered = await renderOpenProject();

    await waitFor(() => {
      expect(rendered.getByText("/home/user/myproject")).toBeTruthy();
    }, { timeout: 3000 });

    // Multiple projects shown with names and paths
    expect(rendered.getByText("myproject")).toBeTruthy();
    expect(rendered.getByText("other-repo")).toBeTruthy();
    expect(rendered.getByText("/home/user/other-repo")).toBeTruthy();
    expect(rendered.container.querySelector(".recent h3")).toBeTruthy();
    expect(rendered.container.querySelectorAll(".recent-item").length).toBe(2);
  });

  it("toggles New Project form on button click", async () => {
    invokeMock.mockResolvedValue([]);
    const rendered = await renderOpenProject();

    expect(rendered.queryByText("Repository URL")).toBeNull();

    await fireEvent.click(rendered.getByText("New Project"));

    await waitFor(() => {
      expect(rendered.getByText("Repository URL")).toBeTruthy();
      expect(rendered.getByText("Parent Directory")).toBeTruthy();
      expect(rendered.getByText("Clone Mode")).toBeTruthy();
    });
  });

  it("shows Shallow (Recommended) selected by default in clone mode", async () => {
    invokeMock.mockResolvedValue([]);
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("New Project"));

    await waitFor(() => {
      const shallowBtn = rendered.getByText("Shallow (Recommended)");
      expect(shallowBtn.classList.contains("active")).toBe(true);
    });
  });

  it("disables Create button when repoUrl or parentDir is empty", async () => {
    invokeMock.mockResolvedValue([]);
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("New Project"));

    await waitFor(() => {
      const createBtn = rendered.getByText("Create") as HTMLButtonElement;
      expect(createBtn.disabled).toBe(true);
    });
  });

  it("probes path and opens gwt project on recent project click", async () => {
    const onOpen = vi.fn();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") {
        return [{ path: "/home/user/project", lastOpened: "2026-01-01T00:00:00Z" }];
      }
      if (cmd === "probe_path") {
        return { kind: "gwtProject", projectPath: "/home/user/project" };
      }
      if (cmd === "open_project") {
        return { action: "opened", info: { path: "/home/user/project" } };
      }
      return null;
    });

    const rendered = await renderOpenProject({ onOpen });

    await waitFor(() => {
      expect(rendered.getByText("project")).toBeTruthy();
    });

    const recentBtn = rendered.container.querySelector(".recent-item") as HTMLButtonElement;
    await fireEvent.click(recentBtn);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("probe_path", { path: "/home/user/project" });
      expect(onOpen).toHaveBeenCalledWith(
        "/home/user/project",
        expect.any(String),
      );
    });
  });

  it("shows error when path does not exist", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "probe_path") {
        return { kind: "notFound", message: "Path does not exist." };
      }
      return null;
    });

    openDialogMock.mockResolvedValue("/nonexistent/path");
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("Open Project..."));

    await waitFor(() => {
      expect(rendered.getByText("Path does not exist.")).toBeTruthy();
    });
  });

  it("shows error for non-git repository", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "probe_path") {
        return { kind: "notGwtProject", message: "Not a gwt project." };
      }
      return null;
    });

    openDialogMock.mockResolvedValue("/some/dir");
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("Open Project..."));

    await waitFor(() => {
      expect(rendered.getByText("Not a gwt project.")).toBeTruthy();
    });
  });

  it("opens migration modal when probe returns migrationRequired", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "probe_path") {
        return { kind: "migrationRequired", migrationSourceRoot: "/old/repo" };
      }
      return null;
    });

    openDialogMock.mockResolvedValue("/old/repo");
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("Open Project..."));

    await waitFor(() => {
      expect(rendered.getByText("Migration Required")).toBeTruthy();
    });
  });

  it("shows new project form when probe returns emptyDir", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "probe_path") {
        return { kind: "emptyDir", projectPath: "/empty/dir" };
      }
      return null;
    });

    openDialogMock.mockResolvedValue("/empty/dir");
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("Open Project..."));

    await waitFor(() => {
      expect(rendered.getByText("Repository URL")).toBeTruthy();
    });
  });

  it("handles dialog cancellation (returns null)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      return null;
    });

    openDialogMock.mockResolvedValue(null);
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("Open Project..."));

    // Should not show any error
    expect(rendered.queryByText(/error/i)).toBeNull();
  });

  it("shows Opening... while opening a project", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "probe_path") return new Promise(() => {}); // never resolves
      return null;
    });

    openDialogMock.mockResolvedValue("/some/path");
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("Open Project..."));

    await waitFor(() => {
      expect(rendered.getByText("Opening...")).toBeTruthy();
    });
  });

  it("shows Creating... while creating a project", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "create_project") return new Promise(() => {}); // never resolves
      return null;
    });
    listenMock.mockResolvedValue(() => {});

    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("New Project"));

    await waitFor(() => {
      expect(rendered.getByText("Repository URL")).toBeTruthy();
    });

    // Fill in the fields
    const urlInput = rendered.container.querySelector('input[placeholder*="github.com"]') as HTMLInputElement;
    await fireEvent.input(urlInput, { target: { value: "https://github.com/test/repo" } });

    openDialogMock.mockResolvedValue("/parent/dir");
    await fireEvent.click(rendered.getByText("Choose..."));

    await waitFor(() => {
      const createBtn = rendered.getByText("Create") as HTMLButtonElement;
      expect(createBtn.disabled).toBe(false);
    });

    await fireEvent.click(rendered.getByText("Create"));

    await waitFor(() => {
      expect(rendered.getByText("Creating...")).toBeTruthy();
    });
  });

  it("shows error when create_project fails with generic error", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "create_project") throw new Error("network failure");
      return null;
    });
    listenMock.mockResolvedValue(() => {});

    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("New Project"));

    const urlInput = rendered.container.querySelector('input[placeholder*="github.com"]') as HTMLInputElement;
    await fireEvent.input(urlInput, { target: { value: "https://github.com/test/repo" } });

    openDialogMock.mockResolvedValue("/parent");
    await fireEvent.click(rendered.getByText("Choose..."));

    await waitFor(() => {
      expect((rendered.getByText("Create") as HTMLButtonElement).disabled).toBe(false);
    });

    await fireEvent.click(rendered.getByText("Create"));

    await waitFor(() => {
      expect(rendered.getByText(/Failed to create project/)).toBeTruthy();
    });
  });

  it("disables buttons during project opening", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "probe_path") return new Promise(() => {});
      return null;
    });

    openDialogMock.mockResolvedValue("/some/path");
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("Open Project..."));

    await waitFor(() => {
      const openBtn = rendered.getByText("Opening...") as HTMLButtonElement;
      expect(openBtn.disabled).toBe(true);
      const newBtn = rendered.getByText("New Project") as HTMLButtonElement;
      expect(newBtn.disabled).toBe(true);
    });
  });

  // --- Clone flow completion ---

  it("calls onOpen when createProject succeeds", async () => {
    const onOpen = vi.fn();
    const mockUnlisten = vi.fn();
    listenMock.mockResolvedValue(mockUnlisten);

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "create_project") {
        return { action: "opened", info: { path: "/cloned/repo" } };
      }
      return null;
    });

    const rendered = await renderOpenProject({ onOpen });

    // Open new project form
    await fireEvent.click(rendered.getByText("New Project"));
    await waitFor(() => {
      expect(rendered.getByText("Repository URL")).toBeTruthy();
    });

    // Fill in repo URL
    const urlInput = rendered.container.querySelector(
      'input[placeholder*="github.com"]',
    ) as HTMLInputElement;
    await fireEvent.input(urlInput, {
      target: { value: "https://github.com/owner/repo" },
    });

    // Choose parent dir
    openDialogMock.mockResolvedValue("/parent");
    await fireEvent.click(rendered.getByText("Choose..."));

    await waitFor(() => {
      expect(
        (rendered.getByText("Create") as HTMLButtonElement).disabled,
      ).toBe(false);
    });

    await fireEvent.click(rendered.getByText("Create"));

    await waitFor(() => {
      expect(onOpen).toHaveBeenCalledWith("/cloned/repo");
    });

    // unlisten should have been called for cleanup
    expect(mockUnlisten).toHaveBeenCalled();
  });

  it("shows 'Invalid repository URL' when create_project fails with that message", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "create_project")
        throw new Error("Invalid repository URL: bad");
      return null;
    });
    listenMock.mockResolvedValue(() => {});

    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("New Project"));

    const urlInput = rendered.container.querySelector(
      'input[placeholder*="github.com"]',
    ) as HTMLInputElement;
    await fireEvent.input(urlInput, {
      target: { value: "not-a-url" },
    });

    openDialogMock.mockResolvedValue("/parent");
    await fireEvent.click(rendered.getByText("Choose..."));

    await waitFor(() => {
      expect(
        (rendered.getByText("Create") as HTMLButtonElement).disabled,
      ).toBe(false);
    });

    await fireEvent.click(rendered.getByText("Create"));

    await waitFor(() => {
      expect(rendered.getByText("Invalid repository URL")).toBeTruthy();
    });
  });

  // --- normalizeOpenProjectError coverage ---

  it("normalizes 'Migration required' error from open_project", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") {
        return [
          { path: "/home/user/proj", lastOpened: "2026-01-01T00:00:00Z" },
        ];
      }
      if (cmd === "probe_path") {
        return { kind: "gwtProject", projectPath: "/home/user/proj" };
      }
      if (cmd === "open_project")
        throw new Error("Migration required for this project");
      return null;
    });

    const rendered = await renderOpenProject();

    await waitFor(() => {
      expect(rendered.getByText("proj")).toBeTruthy();
    });

    const recentBtn = rendered.container.querySelector(
      ".recent-item",
    ) as HTMLButtonElement;
    await fireEvent.click(recentBtn);

    await waitFor(() => {
      expect(rendered.getByText("Migration required.")).toBeTruthy();
    });
  });

  it("normalizes 'Path does not exist' error from open_project", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") {
        return [
          { path: "/home/user/proj", lastOpened: "2026-01-01T00:00:00Z" },
        ];
      }
      if (cmd === "probe_path") {
        return { kind: "gwtProject", projectPath: "/home/user/proj" };
      }
      if (cmd === "open_project")
        throw new Error("Path does not exist: /home/user/proj");
      return null;
    });

    const rendered = await renderOpenProject();

    await waitFor(() => {
      expect(rendered.getByText("proj")).toBeTruthy();
    });

    const recentBtn = rendered.container.querySelector(
      ".recent-item",
    ) as HTMLButtonElement;
    await fireEvent.click(recentBtn);

    await waitFor(() => {
      expect(rendered.getByText("Path does not exist.")).toBeTruthy();
    });
  });

  it("normalizes 'Not a git repository' error from open_project", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") {
        return [
          { path: "/home/user/proj", lastOpened: "2026-01-01T00:00:00Z" },
        ];
      }
      if (cmd === "probe_path") {
        return { kind: "gwtProject", projectPath: "/home/user/proj" };
      }
      if (cmd === "open_project")
        throw new Error("Not a git repository at /home/user/proj");
      return null;
    });

    const rendered = await renderOpenProject();

    await waitFor(() => {
      expect(rendered.getByText("proj")).toBeTruthy();
    });

    const recentBtn = rendered.container.querySelector(
      ".recent-item",
    ) as HTMLButtonElement;
    await fireEvent.click(recentBtn);

    await waitFor(() => {
      expect(rendered.getByText("Not a git repository.")).toBeTruthy();
    });
  });

  it("normalizes 'Not a gwt project' error from open_project", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") {
        return [
          { path: "/home/user/proj", lastOpened: "2026-01-01T00:00:00Z" },
        ];
      }
      if (cmd === "probe_path") {
        return { kind: "gwtProject", projectPath: "/home/user/proj" };
      }
      if (cmd === "open_project")
        throw new Error("Not a gwt project: missing .gwt");
      return null;
    });

    const rendered = await renderOpenProject();

    await waitFor(() => {
      expect(rendered.getByText("proj")).toBeTruthy();
    });

    const recentBtn = rendered.container.querySelector(
      ".recent-item",
    ) as HTMLButtonElement;
    await fireEvent.click(recentBtn);

    await waitFor(() => {
      expect(rendered.getByText("Not a gwt project.")).toBeTruthy();
    });
  });

  // --- normalizeProbeError coverage ---

  it("shows 'Invalid path.' for probe kind=invalid", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "probe_path") {
        return { kind: "invalid", message: null };
      }
      return null;
    });

    openDialogMock.mockResolvedValue("/bad/path");
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("Open Project..."));

    await waitFor(() => {
      expect(rendered.getByText("Invalid path.")).toBeTruthy();
    });
  });

  it("shows fallback 'Failed to open project.' when probe has no message", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "probe_path") {
        // A kind that doesn't match notFound/invalid/notGwtProject and has no message
        return { kind: "unknown" as any, message: null };
      }
      return null;
    });

    openDialogMock.mockResolvedValue("/some/path");
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("Open Project..."));

    await waitFor(() => {
      expect(rendered.getByText("Failed to open project.")).toBeTruthy();
    });
  });

  // --- toErrorMessage edge cases ---

  it("handles string error thrown during probeAndOpen", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "probe_path") throw "plain string error";
      return null;
    });

    openDialogMock.mockResolvedValue("/some/path");
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("Open Project..."));

    await waitFor(() => {
      expect(rendered.getByText("plain string error")).toBeTruthy();
    });
  });

  it("handles object error without message property via JSON.stringify", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "probe_path") throw { code: 42 };
      return null;
    });

    openDialogMock.mockResolvedValue("/some/path");
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("Open Project..."));

    await waitFor(() => {
      expect(rendered.container.querySelector(".error")).toBeTruthy();
      expect(
        rendered.container.querySelector(".error")!.textContent,
      ).toContain('{"code":42}');
    });
  });

  // --- openFolder error ---

  it("shows error when openFolder dialog throws", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      return null;
    });

    openDialogMock.mockRejectedValue(new Error("dialog crashed"));
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("Open Project..."));

    await waitFor(() => {
      expect(
        rendered.getByText("Failed to open folder dialog: dialog crashed"),
      ).toBeTruthy();
    });
  });

  // --- chooseParentDir error ---

  it("shows error when chooseParentDir dialog throws", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      return null;
    });

    const rendered = await renderOpenProject();

    // Open new project form
    await fireEvent.click(rendered.getByText("New Project"));
    await waitFor(() => {
      expect(rendered.getByText("Choose...")).toBeTruthy();
    });

    openDialogMock.mockRejectedValue(new Error("dir picker failed"));
    await fireEvent.click(rendered.getByText("Choose..."));

    await waitFor(() => {
      expect(
        rendered.getByText(
          "Failed to open folder dialog: dir picker failed",
        ),
      ).toBeTruthy();
    });
  });

  // --- loadRecentProjects error ---

  it("handles loadRecentProjects failure gracefully", async () => {
    invokeMock.mockRejectedValue(new Error("backend down"));
    const rendered = await renderOpenProject();

    // Should still render without crashing, no recent projects shown
    await waitFor(() => {
      expect(rendered.getByText("gwt")).toBeTruthy();
    });
    expect(rendered.queryByText("Recent Projects")).toBeNull();
  });

  // --- progressLabel coverage ---

  it("shows 'Receiving objects' for receiving stage", async () => {
    let cloneProgressHandler: ((event: { payload: any }) => void) | null = null;
    listenMock.mockImplementation(async (eventName: string, handler: any) => {
      if (eventName === "clone-progress") {
        cloneProgressHandler = handler;
      }
      return () => {};
    });

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "create_project") return new Promise(() => {}); // never resolves
      return null;
    });

    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("New Project"));
    await waitFor(() => {
      expect(rendered.getByText("Repository URL")).toBeTruthy();
    });

    const urlInput = rendered.container.querySelector('input[placeholder*="github.com"]') as HTMLInputElement;
    await fireEvent.input(urlInput, { target: { value: "https://github.com/test/repo" } });

    openDialogMock.mockResolvedValue("/parent");
    await fireEvent.click(rendered.getByText("Choose..."));

    await waitFor(() => {
      expect((rendered.getByText("Create") as HTMLButtonElement).disabled).toBe(false);
    });

    await fireEvent.click(rendered.getByText("Create"));

    await waitFor(() => {
      expect(cloneProgressHandler).not.toBeNull();
    });

    cloneProgressHandler!({ payload: { stage: "receiving", percent: 42 } });

    await waitFor(() => {
      expect(rendered.getByText("Receiving objects")).toBeTruthy();
      expect(rendered.getByText("42%")).toBeTruthy();
    });
  });

  it("shows 'Resolving deltas' for resolving stage", async () => {
    let cloneProgressHandler: ((event: { payload: any }) => void) | null = null;
    listenMock.mockImplementation(async (eventName: string, handler: any) => {
      if (eventName === "clone-progress") {
        cloneProgressHandler = handler;
      }
      return () => {};
    });

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "create_project") return new Promise(() => {}); // never resolves
      return null;
    });

    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("New Project"));
    await waitFor(() => {
      expect(rendered.getByText("Repository URL")).toBeTruthy();
    });

    const urlInput = rendered.container.querySelector('input[placeholder*="github.com"]') as HTMLInputElement;
    await fireEvent.input(urlInput, { target: { value: "https://github.com/test/repo" } });

    openDialogMock.mockResolvedValue("/parent");
    await fireEvent.click(rendered.getByText("Choose..."));

    await waitFor(() => {
      expect((rendered.getByText("Create") as HTMLButtonElement).disabled).toBe(false);
    });

    await fireEvent.click(rendered.getByText("Create"));

    await waitFor(() => {
      expect(cloneProgressHandler).not.toBeNull();
    });

    cloneProgressHandler!({ payload: { stage: "resolving", percent: 85 } });

    await waitFor(() => {
      expect(rendered.getByText("Resolving deltas")).toBeTruthy();
      expect(rendered.getByText("85%")).toBeTruthy();
    });
  });

  it("shows 'Cloning' for unknown stage", async () => {
    let cloneProgressHandler: ((event: { payload: any }) => void) | null = null;
    listenMock.mockImplementation(async (eventName: string, handler: any) => {
      if (eventName === "clone-progress") {
        cloneProgressHandler = handler;
      }
      return () => {};
    });

    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "create_project") return new Promise(() => {}); // never resolves
      return null;
    });

    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("New Project"));
    await waitFor(() => {
      expect(rendered.getByText("Repository URL")).toBeTruthy();
    });

    const urlInput = rendered.container.querySelector('input[placeholder*="github.com"]') as HTMLInputElement;
    await fireEvent.input(urlInput, { target: { value: "https://github.com/test/repo" } });

    openDialogMock.mockResolvedValue("/parent");
    await fireEvent.click(rendered.getByText("Choose..."));

    await waitFor(() => {
      expect((rendered.getByText("Create") as HTMLButtonElement).disabled).toBe(false);
    });

    await fireEvent.click(rendered.getByText("Create"));

    await waitFor(() => {
      expect(cloneProgressHandler).not.toBeNull();
    });

    cloneProgressHandler!({ payload: { stage: "other", percent: 10 } });

    await waitFor(() => {
      expect(rendered.getByText("Cloning")).toBeTruthy();
      expect(rendered.getByText("10%")).toBeTruthy();
    });
  });

  // --- MigrationModal onCompleted callback ---

  it("opens project after migration completes", async () => {
    const onOpen = vi.fn();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "probe_path") {
        return { kind: "migrationRequired", migrationSourceRoot: "/old/repo" };
      }
      if (cmd === "open_project") {
        return { action: "opened", info: { path: "/migrated/repo" } };
      }
      return null;
    });

    openDialogMock.mockResolvedValue("/old/repo");
    const rendered = await renderOpenProject({ onOpen });

    await fireEvent.click(rendered.getByText("Open Project..."));

    await waitFor(() => {
      expect(rendered.getByText("Migration Required")).toBeTruthy();
    });
  });

  // --- probeAndOpen normalizeOpenProjectError on catch path ---

  it("normalizes error from probeAndOpen catch path", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "get_recent_projects") return [];
      if (cmd === "probe_path")
        throw new Error("Not a git repository: /x");
      return null;
    });

    openDialogMock.mockResolvedValue("/x");
    const rendered = await renderOpenProject();

    await fireEvent.click(rendered.getByText("Open Project..."));

    await waitFor(() => {
      expect(rendered.getByText("Not a git repository.")).toBeTruthy();
    });
  });
});
