import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();
const openDialogMock = vi.fn();
const listenMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: (...args: unknown[]) => openDialogMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
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
    await new Promise((r) => setTimeout(r, 0));

    invokeMock.mockResolvedValue([
      { path: "/home/user/myproject", lastOpened: "2026-01-01T00:00:00Z" },
      { path: "/home/user/other-repo", lastOpened: "2026-01-02T00:00:00Z" },
    ]);

    const rendered = await renderOpenProject();

    await waitFor(() => {
      expect(rendered.getByText("/home/user/myproject")).toBeTruthy();
    });

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
      expect(onOpen).toHaveBeenCalledWith("/home/user/project");
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
});
