import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("../openExternalUrl", () => ({
  openExternalUrl: vi.fn(),
}));

async function renderPanel(props?: any) {
  const { default: PrListPanel } = await import("./PrListPanel.svelte");
  return render(PrListPanel, {
    props: {
      projectPath: "/tmp/project",
      onSwitchToWorktree: vi.fn(),
      ...props,
    },
  });
}

describe("PrListPanel", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    cleanup();
  });

  afterEach(() => {
    cleanup();
  });

  it("shows gh CLI unavailable message when not authenticated", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: false, authenticated: false };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText(/GitHub CLI.*not available/i)).toBeTruthy();
    });
  });

  it("shows Pull Requests header", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: false, authenticated: false };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Pull Requests")).toBeTruthy();
    });
  });

  it("renders state filter buttons (Open, Closed, Merged)", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: false, authenticated: false };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Open")).toBeTruthy();
      expect(rendered.getByText("Closed")).toBeTruthy();
      expect(rendered.getByText("Merged")).toBeTruthy();
    });
  });

  it("has Open filter active by default", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: false, authenticated: false };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      const openBtn = rendered.getByText("Open");
      expect(openBtn.classList.contains("active")).toBe(true);
    });
  });

  it("shows search input with correct placeholder", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: false, authenticated: false };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      const input = rendered.container.querySelector('input[placeholder="Search pull requests..."]');
      expect(input).toBeTruthy();
    });
  });

  it("has a refresh button", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: false, authenticated: false };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      const refreshBtn = rendered.container.querySelector('[title="Refresh"]');
      expect(refreshBtn).toBeTruthy();
    });
  });

  it("shows loading state while fetching PRs", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: true, authenticated: true };
      if (cmd === "fetch_github_user") return { login: "alice" };
      if (cmd === "fetch_pr_list") return new Promise(() => {}); // never resolves
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText("Loading pull requests...")).toBeTruthy();
    });
  });

  it("calls check_gh_cli_status on mount", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: false, authenticated: false };
      return null;
    });

    await renderPanel();

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("check_gh_cli_status", {
        projectPath: "/tmp/project",
      });
    });
  });

  it("does not show PR list when gh CLI check fails", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") throw new Error("Failed");
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      expect(rendered.getByText(/GitHub CLI.*not available/i)).toBeTruthy();
    });
  });

  it("renders section element with pr-list-panel class", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: false, authenticated: false };
      return null;
    });

    const rendered = await renderPanel();

    expect(rendered.container.querySelector(".pr-list-panel")).toBeTruthy();
  });

  it("renders header with plp-header class", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: false, authenticated: false };
      return null;
    });

    const rendered = await renderPanel();

    expect(rendered.container.querySelector(".plp-header")).toBeTruthy();
  });

  it("has Closed filter not active by default", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "check_gh_cli_status") return { available: false, authenticated: false };
      return null;
    });

    const rendered = await renderPanel();

    await waitFor(() => {
      const closedBtn = rendered.getByText("Closed");
      expect(closedBtn.classList.contains("active")).toBe(false);
      const mergedBtn = rendered.getByText("Merged");
      expect(mergedBtn.classList.contains("active")).toBe(false);
    });
  });
});
