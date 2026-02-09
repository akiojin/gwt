import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor } from "@testing-library/svelte";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

async function renderSidebar(props: any) {
  const { default: Sidebar } = await import("./Sidebar.svelte");
  return render(Sidebar, { props });
}

describe("Sidebar", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue([]);
  });

  it("does not re-fetch local branches when refreshKey is unchanged", async () => {
    const onBranchSelect = vi.fn();

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect,
      refreshKey: 0,
    });

    await waitFor(() => {
      expect(invokeMock.mock.calls.length).toBeGreaterThan(0);
    });

    const firstCallCount = invokeMock.mock.calls.length;

    // Rerender with the same key should not trigger a re-fetch.
    await rendered.rerender({ refreshKey: 0 });

    await new Promise((r) => setTimeout(r, 50));
    expect(invokeMock.mock.calls.length).toBe(firstCallCount);
  });

  it("re-fetches local branches when refreshKey changes", async () => {
    const onBranchSelect = vi.fn();

    const rendered = await renderSidebar({
      projectPath: "/tmp/project",
      onBranchSelect,
      refreshKey: 0,
    });

    await waitFor(() => {
      expect(invokeMock.mock.calls.length).toBeGreaterThan(0);
    });

    const firstCallCount = invokeMock.mock.calls.length;

    // Changing refreshKey should trigger a re-fetch.
    await rendered.rerender({
      refreshKey: 1,
    });

    await waitFor(() => {
      expect(invokeMock.mock.calls.length).toBe(firstCallCount + 1);
    });
  });
});
