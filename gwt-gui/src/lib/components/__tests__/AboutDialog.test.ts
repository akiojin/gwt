import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  default: {
    invoke: invokeMock,
  },
}));

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn().mockResolvedValue("1.0.0"),
}));

async function renderAboutDialog(props: any) {
  const { default: AboutDialog } = await import("../AboutDialog.svelte");
  return render(AboutDialog, { props });
}

describe("AboutDialog", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
    Object.defineProperty(globalThis, "__TAURI_INTERNALS__", {
      value: { invoke: invokeMock },
      configurable: true,
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_agents") return [];
      if (command === "get_stats") {
        return { global: { agents: [], worktrees_created: 0 }, repos: [] };
      }
      return null;
    });
  });

  it("shows General tab active when initialTab is 'general'", async () => {
    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "general",
      onclose: vi.fn(),
    });

    const generalBtn = rendered.getByText("General");
    expect(generalBtn.classList.contains("active")).toBe(true);

    await rendered.findByText("gwt");
    await rendered.findByText("Git Worktree Manager");
  });

  it("shows System tab active when initialTab is 'system'", async () => {
    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "system",
      cpuUsage: 45,
      memUsed: 8589934592,
      memTotal: 17179869184,
      onclose: vi.fn(),
    });

    const systemBtn = rendered.getByText("System");
    expect(systemBtn.classList.contains("active")).toBe(true);

    await rendered.findByText("CPU");
    await rendered.findByText("Memory");
  });

  it("switches tabs on click", async () => {
    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "general",
      onclose: vi.fn(),
    });

    await rendered.findByText("gwt");

    const systemBtn = rendered.getByText("System");
    await fireEvent.click(systemBtn);

    expect(systemBtn.classList.contains("active")).toBe(true);
    await rendered.findByText("CPU");
  });

  it("calls onclose when Close button is clicked", async () => {
    const onclose = vi.fn();
    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "general",
      onclose,
    });

    const closeBtn = await rendered.findByText("Close");
    await fireEvent.click(closeBtn);

    expect(onclose).toHaveBeenCalledTimes(1);
  });

  it("does not render when open is false", async () => {
    const rendered = await renderAboutDialog({
      open: false,
      initialTab: "general",
      onclose: vi.fn(),
    });

    expect(rendered.queryByText("gwt")).toBeNull();
  });

  it("uses overlay click to close dialog", async () => {
    const onclose = vi.fn();
    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "general",
      onclose,
    });

    await rendered.findByText("gwt");
    await fireEvent.click(rendered.container.querySelector(".overlay") as HTMLElement);

    expect(onclose).toHaveBeenCalledTimes(1);
  });

  it("renders GPU details in System tab when gpuInfo is provided", async () => {
    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "system",
      cpuUsage: 22,
      memUsed: 8 * 1024 ** 3,
      memTotal: 16 * 1024 ** 3,
      gpuInfo: {
        name: "M2 Pro",
        usage_percent: 64,
        vram_total_bytes: 8 * 1024 ** 3,
        vram_used_bytes: 4 * 1024 ** 3,
      },
      onclose: vi.fn(),
    });

    await rendered.findByText("GPU");
    expect(rendered.getByText("M2 Pro")).toBeTruthy();
    expect(rendered.getByText("64%")).toBeTruthy();
    expect(rendered.getByText("VRAM: 4.0 / 8.0 GB")).toBeTruthy();
  });

  it("renders detected agents in General tab", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_agents") {
        return [
          { id: "codex", name: "Codex", available: true, authenticated: true, version: "1.2.3" },
          { id: "claude", name: "Claude", available: false, authenticated: false, version: "" },
        ];
      }
      if (command === "get_stats") {
        return { global: { agents: [], worktrees_created: 0 }, repos: [] };
      }
      return null;
    });

    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "general",
      onclose: vi.fn(),
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("detect_agents");
    });

    await rendered.findByText("Detected Agents");
    expect(rendered.getByText("Codex")).toBeTruthy();
    expect(rendered.getByText("1.2.3")).toBeTruthy();
    expect(rendered.getByText("Claude")).toBeTruthy();
    expect(rendered.getByText("not installed")).toBeTruthy();
  });

  it("renders statistics table and supports repository filter", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_agents") return [];
      if (command === "get_stats") {
        return {
          global: {
            agents: [
              { agent_id: "codex", model: "gpt-5", count: 3 },
              { agent_id: "claude", model: "sonnet", count: 1 },
            ],
            worktrees_created: 9,
          },
          repos: [
            {
              repo_path: "/tmp/repo-a",
              stats: {
                agents: [{ agent_id: "codex", model: "gpt-5", count: 1 }],
                worktrees_created: 2,
              },
            },
            {
              repo_path: "/tmp/repo-b",
              stats: {
                agents: [],
                worktrees_created: 0,
              },
            },
          ],
        };
      }
      return null;
    });

    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "statistics",
      onclose: vi.fn(),
    });

    await waitFor(() => {
      expect(rendered.getByText("All repositories")).toBeTruthy();
      expect(rendered.getByText("codex")).toBeTruthy();
      expect(rendered.getByText("gpt-5")).toBeTruthy();
      expect(rendered.getByText("Worktrees created:")).toBeTruthy();
    });

    const repoSelect = rendered.container.querySelector("#repo-filter") as HTMLSelectElement;
    await fireEvent.change(repoSelect, { target: { value: "/tmp/repo-b" } });

    await waitFor(() => {
      expect(rendered.getByText("No agent launches in this scope")).toBeTruthy();
    });
  });

});
