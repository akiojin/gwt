import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
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
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_agents") return [];
      if (command === "get_stats") {
        return { global_agents: [], global_worktrees_created: 0, repos: [] };
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
});
