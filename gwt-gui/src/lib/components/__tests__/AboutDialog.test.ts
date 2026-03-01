import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
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

  it("renders GPU details in System tab when gpuInfos are provided", async () => {
    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "system",
      cpuUsage: 22,
      memUsed: 8 * 1024 ** 3,
      memTotal: 16 * 1024 ** 3,
      gpuInfos: [{
        name: "M2 Pro",
        usage_percent: 64,
        vram_total_bytes: 8 * 1024 ** 3,
        vram_used_bytes: 4 * 1024 ** 3,
      }],
      onclose: vi.fn(),
    });

    await rendered.findByText("GPU");
    expect(rendered.getByText("M2 Pro")).toBeTruthy();
    expect(rendered.getByText("64%")).toBeTruthy();
    expect(rendered.getByText("VRAM: 4.0 / 8.0 GB")).toBeTruthy();
  });

  it("renders multiple GPUs and supports static-only details", async () => {
    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "system",
      cpuUsage: 22,
      memUsed: 8 * 1024 ** 3,
      memTotal: 16 * 1024 ** 3,
      gpuInfos: [
        {
          name: "NVIDIA GeForce RTX 4090",
          usage_percent: 12,
          vram_total_bytes: 24 * 1024 ** 3,
          vram_used_bytes: 2 * 1024 ** 3,
        },
        {
          name: "Intel(R) UHD Graphics",
          usage_percent: null,
          vram_total_bytes: 1 * 1024 ** 3,
          vram_used_bytes: null,
        },
      ],
      onclose: vi.fn(),
    });

    await rendered.findByText("GPU 1");
    await rendered.findByText("GPU 2");
    expect(rendered.getByText("NVIDIA GeForce RTX 4090")).toBeTruthy();
    expect(rendered.getByText("Intel(R) UHD Graphics")).toBeTruthy();
    expect(rendered.getByText("VRAM: 2.0 / 24.0 GB")).toBeTruthy();
    expect(rendered.getByText("VRAM: 1.0 GB")).toBeTruthy();
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

  it("renders 'installed' for agent with available=true but no version", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_agents") {
        return [
          { id: "codex", name: "Codex", available: true, authenticated: true, version: "" },
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
      expect(rendered.getByText("Codex")).toBeTruthy();
      expect(rendered.getByText("installed")).toBeTruthy();
    });
  });

  it("handles loadAgents failure gracefully", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_agents") throw new Error("backend down");
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

    // Should render without agent list when load fails
    await waitFor(() => {
      expect(rendered.getByText("gwt")).toBeTruthy();
    });

    expect(rendered.queryByText("Detected Agents")).toBeNull();
  });

  it("handles loadStats failure gracefully on statistics tab", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_agents") return [];
      if (command === "get_stats") throw new Error("stats unavailable");
      return null;
    });

    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "statistics",
      onclose: vi.fn(),
    });

    await waitFor(() => {
      expect(rendered.getByText("No statistics yet")).toBeTruthy();
    });
  });

  it("does not close dialog when clicking inside the dialog area", async () => {
    const onclose = vi.fn();
    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "general",
      onclose,
    });

    await rendered.findByText("gwt");
    const dialogEl = rendered.container.querySelector(".about-dialog") as HTMLElement;
    await fireEvent.click(dialogEl);

    expect(onclose).not.toHaveBeenCalled();
  });

  it("shows 'No GPU detected' when gpuInfos is empty in system tab", async () => {
    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "system",
      cpuUsage: 10,
      memUsed: 4 * 1024 ** 3,
      memTotal: 16 * 1024 ** 3,
      gpuInfos: [],
      onclose: vi.fn(),
    });

    await rendered.findByText("No GPU detected");
  });

  it("shows 0% memory when memTotal is 0", async () => {
    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "system",
      cpuUsage: 50,
      memUsed: 0,
      memTotal: 0,
      gpuInfos: [],
      onclose: vi.fn(),
    });

    await rendered.findByText("CPU");
    // memPct should be 0 since memTotal=0
    expect(rendered.container.textContent).toContain("0%");
  });

  it("shows Loading statistics... while stats are loading", async () => {
    // Make get_stats hang so statsLoading stays true
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_agents") return [];
      if (command === "get_stats") return new Promise(() => {});
      return null;
    });

    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "statistics",
      onclose: vi.fn(),
    });

    await waitFor(() => {
      expect(rendered.getByText("Loading statistics...")).toBeTruthy();
    });
  });

  it("shows no agent launches when filtered repo has no agents", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_agents") return [];
      if (command === "get_stats") {
        return {
          global: {
            agents: [{ agent_id: "codex", model: "gpt-5", count: 2 }],
            worktrees_created: 5,
          },
          repos: [
            {
              repo_path: "/tmp/repo-a",
              stats: {
                agents: [{ agent_id: "codex", model: "gpt-5", count: 2 }],
                worktrees_created: 5,
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
      expect(rendered.getByText("codex")).toBeTruthy();
    });

    // Filter by a non-matching repo path (not in the list)
    const repoSelect = rendered.container.querySelector("#repo-filter") as HTMLSelectElement;
    // Select an existing repo that has agents
    await fireEvent.change(repoSelect, { target: { value: "/tmp/repo-a" } });

    await waitFor(() => {
      expect(rendered.getByText("codex")).toBeTruthy();
    });
  });

  it("renders GPU section with usage_percent but no VRAM", async () => {
    const rendered = await renderAboutDialog({
      open: true,
      initialTab: "system",
      cpuUsage: 10,
      memUsed: 4 * 1024 ** 3,
      memTotal: 16 * 1024 ** 3,
      gpuInfos: [
        {
          name: "Test GPU",
          usage_percent: 50,
          vram_total_bytes: null,
          vram_used_bytes: null,
        },
      ],
      onclose: vi.fn(),
    });

    await rendered.findByText("GPU");
    expect(rendered.getByText("Test GPU")).toBeTruthy();
    expect(rendered.getByText("50%")).toBeTruthy();
    // No VRAM display
    expect(rendered.container.textContent).not.toContain("VRAM");
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
