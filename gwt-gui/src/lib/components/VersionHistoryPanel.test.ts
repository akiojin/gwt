import { describe, it, expect, vi, beforeEach } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";

const invokeMock = vi.fn();
type VersionHistoryUpdatedPayload = {
  projectPath: string;
  versionId: string;
  result: Record<string, any>;
};
const eventListeners = new Map<string, Set<(event: { payload: VersionHistoryUpdatedPayload }) => void>>();
const listenMock = vi.fn(
  async (
    eventName: string,
    handler: (event: { payload: VersionHistoryUpdatedPayload }) => void
  ) => {
    let bucket = eventListeners.get(eventName);
    if (!bucket) {
      bucket = new Set();
      eventListeners.set(eventName, bucket);
    }
    bucket.add(handler);
    return () => {
      bucket?.delete(handler);
      if (bucket && bucket.size === 0) eventListeners.delete(eventName);
    };
  }
);

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

async function emitVersionHistoryUpdated(payload: VersionHistoryUpdatedPayload) {
  const handlers = Array.from(eventListeners.get("project-version-history-updated") ?? []);
  for (const handler of handlers) {
    await handler({ payload });
  }
}

async function renderPanel(props: any) {
  const { default: Panel } = await import("./VersionHistoryPanel.svelte");
  return render(Panel, { props });
}

describe("VersionHistoryPanel", () => {
  beforeEach(() => {
    cleanup();
    eventListeners.clear();
    listenMock.mockClear();
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "list_project_versions") {
        return {
          items: [
            {
              id: "unreleased",
              label: "Unreleased (HEAD)",
              range_from: "v1.0.0",
              range_to: "HEAD",
              commit_count: 2,
            },
            {
              id: "v1.0.0",
              label: "v1.0.0",
              range_from: null,
              range_to: "v1.0.0",
              commit_count: 5,
            },
          ],
        };
      }
      if (cmd === "get_project_version_history") {
        const versionId = String(args?.versionId ?? "");
        return {
          status: "ok",
          version_id: versionId,
          label: versionId || "unknown",
          range_from: null,
          range_to: versionId === "unreleased" ? "HEAD" : versionId,
          commit_count: 1,
          summary_markdown: "## Summary\nSomething.\n\n## Highlights\n- One",
          changelog_markdown: "### Features\n- A",
          error: null,
        };
      }
      return [];
    });
  });

  it("loads versions and requests per-version history", async () => {
    const rendered = await renderPanel({ projectPath: "/tmp/project" });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("list_project_versions", {
        projectPath: "/tmp/project",
        limit: 10,
      });
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_project_version_history", {
        projectPath: "/tmp/project",
        versionId: "unreleased",
      });
    });

    await waitFor(() => {
      expect(rendered.container.querySelector(".vh-markdown h2")).toBeTruthy();
      expect(rendered.container.querySelectorAll(".vh-markdown li")).toHaveLength(2);
    });
  });

  it("shows empty state when no version tags exist", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_project_versions") {
        return { items: [] };
      }
      return [];
    });

    const rendered = await renderPanel({ projectPath: "/tmp/project" });
    await waitFor(() => {
      expect(rendered.getByText("No version tags found. Showing Unreleased only.")).toBeTruthy();
    });
    expect(invokeMock).not.toHaveBeenCalledWith("get_project_version_history", expect.anything());
  });

  it("shows load error when listing versions fails", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "list_project_versions") {
        throw new Error("list failed");
      }
      return [];
    });

    const rendered = await renderPanel({ projectPath: "/tmp/project" });
    await waitFor(() => {
      expect(rendered.getByText("Failed to load versions: list failed")).toBeTruthy();
    });
  });

  it("renders disabled/error statuses and toggles card expansion", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "list_project_versions") {
        return {
          items: [
            {
              id: "unreleased",
              label: "Unreleased (HEAD)",
              range_from: "v1.0.0",
              range_to: "HEAD",
              commit_count: 2,
            },
            {
              id: "v1.0.0",
              label: "v1.0.0",
              range_from: null,
              range_to: "v1.0.0",
              commit_count: 1,
            },
          ],
        };
      }
      if (cmd === "get_project_version_history") {
        if (args?.versionId === "unreleased") {
          return {
            status: "disabled",
            version_id: "unreleased",
            label: "Unreleased (HEAD)",
            range_from: "v1.0.0",
            range_to: "HEAD",
            commit_count: 2,
            summary_markdown: null,
            changelog_markdown: null,
            error: null,
          };
        }
        return {
          status: "error",
          version_id: "v1.0.0",
          label: "v1.0.0",
          range_from: null,
          range_to: "v1.0.0",
          commit_count: 1,
          summary_markdown: null,
          changelog_markdown: null,
          error: "generation failed",
        };
      }
      return [];
    });

    const rendered = await renderPanel({ projectPath: "/tmp/project" });
    await waitFor(() => {
      expect(rendered.getByText("ai disabled")).toBeTruthy();
      expect(rendered.getByText("error")).toBeTruthy();
    });
    expect(
      rendered.getByText("AI is not configured. Enable AI settings in Settings to use Version History.")
    ).toBeTruthy();

    await waitFor(() => {
      expect(rendered.container.querySelectorAll(".vh-card-header").length).toBe(2);
    });
    const headers = rendered.container.querySelectorAll(".vh-card-header");
    await fireEvent.click(headers[1] as HTMLElement);
    expect(rendered.getByText("generation failed")).toBeTruthy();
    await fireEvent.click(headers[1] as HTMLElement);
    await waitFor(() => {
      expect(rendered.queryByText("generation failed")).toBeNull();
    });
  });

  it("updates result via event listener and ignores events for other projects", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "list_project_versions") {
        return {
          items: [
            {
              id: "unreleased",
              label: "Unreleased (HEAD)",
              range_from: "v1.0.0",
              range_to: "HEAD",
              commit_count: 2,
            },
          ],
        };
      }
      if (cmd === "get_project_version_history") {
        return {
          status: "generating",
          version_id: "unreleased",
          label: "Unreleased (HEAD)",
          range_from: "v1.0.0",
          range_to: "HEAD",
          commit_count: 2,
          summary_markdown: null,
          changelog_markdown: null,
          error: null,
        };
      }
      return [];
    });

    const rendered = await renderPanel({ projectPath: "/tmp/project" });
    await waitFor(() => {
      expect(rendered.container.querySelector(".vh-status.gen")).toBeTruthy();
    });

    // Event for a different project should be ignored.
    await emitVersionHistoryUpdated({
      projectPath: "/tmp/other",
      versionId: "unreleased",
      result: {
        status: "ok",
        version_id: "unreleased",
        label: "Unreleased (HEAD)",
        range_from: "v1.0.0",
        range_to: "HEAD",
        commit_count: 2,
        summary_markdown: "ignored",
        changelog_markdown: null,
        error: null,
      },
    });
    // Should still show "generating" (event for wrong project was ignored).
    expect(rendered.container.querySelector(".vh-status.gen")).toBeTruthy();

    // Event for the correct project should update the result.
    await emitVersionHistoryUpdated({
      projectPath: "/tmp/project",
      versionId: "unreleased",
      result: {
        status: "ok",
        version_id: "unreleased",
        label: "Unreleased (HEAD)",
        range_from: "v1.0.0",
        range_to: "HEAD",
        commit_count: 2,
        summary_markdown: "## Summary\nDone",
        changelog_markdown: null,
        error: null,
      },
    });

    await waitFor(() => {
      expect(rendered.container.querySelector(".vh-status.ok")).toBeTruthy();
    });
  });

  it("calls get_project_version_history for all versions in parallel", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "list_project_versions") {
        return {
          items: [
            {
              id: "unreleased",
              label: "Unreleased (HEAD)",
              range_from: "v1.0.0",
              range_to: "HEAD",
              commit_count: 2,
            },
            {
              id: "v1.0.0",
              label: "v1.0.0",
              range_from: null,
              range_to: "v1.0.0",
              commit_count: 5,
            },
          ],
        };
      }
      if (cmd === "get_project_version_history") {
        const versionId = String(args?.versionId ?? "");
        // Return immediately with "ok" status.
        return {
          status: "ok",
          version_id: versionId,
          label: versionId,
          range_from: null,
          range_to: versionId === "unreleased" ? "HEAD" : versionId,
          commit_count: 1,
          summary_markdown: "## Summary\nOK",
          changelog_markdown: "### Features\n- ok",
          error: null,
        };
      }
      return [];
    });

    await renderPanel({ projectPath: "/tmp/project" });

    // Wait for all get_project_version_history calls.
    await waitFor(() => {
      const historyCalls = invokeMock.mock.calls.filter(
        (c: any[]) => c[0] === "get_project_version_history"
      );
      expect(historyCalls).toHaveLength(2);
    });

    // Both versions should have been requested (in any order, since parallel).
    const historyVersionIds = invokeMock.mock.calls
      .filter((c: any[]) => c[0] === "get_project_version_history")
      .map((c: any[]) => c[1]?.versionId);
    expect(historyVersionIds).toContain("unreleased");
    expect(historyVersionIds).toContain("v1.0.0");
  });

  it("updates each version status individually as results arrive", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "list_project_versions") {
        return {
          items: [
            {
              id: "unreleased",
              label: "Unreleased (HEAD)",
              range_from: "v1.0.0",
              range_to: "HEAD",
              commit_count: 2,
            },
            {
              id: "v1.0.0",
              label: "v1.0.0",
              range_from: null,
              range_to: "v1.0.0",
              commit_count: 5,
            },
          ],
        };
      }
      if (cmd === "get_project_version_history") {
        const versionId = String(args?.versionId ?? "");
        // Return "generating" status for all, so we can simulate staggered completion.
        return {
          status: "generating",
          version_id: versionId,
          label: versionId,
          range_from: null,
          range_to: versionId === "unreleased" ? "HEAD" : versionId,
          commit_count: 1,
          summary_markdown: null,
          changelog_markdown: null,
          error: null,
        };
      }
      return [];
    });

    const rendered = await renderPanel({ projectPath: "/tmp/project" });

    // Wait for both calls to complete.
    await waitFor(() => {
      const historyCalls = invokeMock.mock.calls.filter(
        (c: any[]) => c[0] === "get_project_version_history"
      );
      expect(historyCalls).toHaveLength(2);
    });

    // Both should show "generating" status.
    await waitFor(() => {
      const genBadges = rendered.container.querySelectorAll(".vh-status.gen");
      expect(genBadges.length).toBe(2);
    });

    // Now simulate only v1.0.0 completing via event.
    await emitVersionHistoryUpdated({
      projectPath: "/tmp/project",
      versionId: "v1.0.0",
      result: {
        status: "ok",
        version_id: "v1.0.0",
        label: "v1.0.0",
        range_from: null,
        range_to: "v1.0.0",
        commit_count: 5,
        summary_markdown: "## Summary\nDone",
        changelog_markdown: "### Features\n- done",
        error: null,
      },
    });

    // v1.0.0 should now show "ok", but unreleased should still be "generating".
    await waitFor(() => {
      const okBadges = rendered.container.querySelectorAll(".vh-status.ok");
      const genBadges = rendered.container.querySelectorAll(".vh-status.gen");
      expect(okBadges.length).toBe(1);
      expect(genBadges.length).toBe(1);
    });
  });

  it("keeps completed status when a stale generating response arrives later", async () => {
    let resolveHistory!: (value: any) => void;
    const historyPromise = new Promise<any>((resolve) => {
      resolveHistory = resolve;
    });

    invokeMock.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "list_project_versions") {
        return {
          items: [
            {
              id: "unreleased",
              label: "Unreleased (HEAD)",
              range_from: "v1.0.0",
              range_to: "HEAD",
              commit_count: 2,
            },
          ],
        };
      }
      if (cmd === "get_project_version_history") {
        return historyPromise;
      }
      return [];
    });

    const rendered = await renderPanel({ projectPath: "/tmp/project" });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_project_version_history", {
        projectPath: "/tmp/project",
        versionId: "unreleased",
      });
    });

    // Background event completes first.
    await emitVersionHistoryUpdated({
      projectPath: "/tmp/project",
      versionId: "unreleased",
      result: {
        status: "ok",
        version_id: "unreleased",
        label: "Unreleased (HEAD)",
        range_from: "v1.0.0",
        range_to: "HEAD",
        commit_count: 2,
        summary_markdown: "## Summary\nDone",
        changelog_markdown: "### Features\n- done",
        error: null,
      },
    });

    await waitFor(() => {
      expect(rendered.container.querySelector(".vh-status.ok")).toBeTruthy();
    });

    // Then the stale direct invoke response resolves with "generating".
    resolveHistory({
      status: "generating",
      version_id: "unreleased",
      label: "Unreleased (HEAD)",
      range_from: "v1.0.0",
      range_to: "HEAD",
      commit_count: 2,
      summary_markdown: null,
      changelog_markdown: "### Features\n- done",
      error: null,
    });

    await waitFor(() => {
      expect(rendered.container.querySelector(".vh-status.ok")).toBeTruthy();
      expect(rendered.container.querySelector(".vh-status.gen")).toBeNull();
    });
  });

  it("displays changelog immediately while AI summary is generating", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "list_project_versions") {
        return {
          items: [
            {
              id: "unreleased",
              label: "Unreleased (HEAD)",
              range_from: "v1.0.0",
              range_to: "HEAD",
              commit_count: 3,
            },
          ],
        };
      }
      if (cmd === "get_project_version_history") {
        return {
          status: "generating",
          version_id: "unreleased",
          label: "Unreleased (HEAD)",
          range_from: "v1.0.0",
          range_to: "HEAD",
          commit_count: 3,
          summary_markdown: null,
          changelog_markdown: "### Features\n- added new feature",
          error: null,
        };
      }
      return [];
    });

    const rendered = await renderPanel({ projectPath: "/tmp/project" });

    // Should show "Generating summary..." note
    await waitFor(() => {
      expect(rendered.getByText("Generating summary...")).toBeTruthy();
    });

    // Changelog should be displayed even during generating state
    await waitFor(() => {
      const h3s = rendered.container.querySelectorAll("h3");
      const changelogH3 = Array.from(h3s).find((h: Element) => h.textContent === "Changelog");
      expect(changelogH3).toBeTruthy();
    });

    // Summary heading should NOT be present (summary_markdown is null)
    const h3s = rendered.container.querySelectorAll("h3");
    const summaryH3 = Array.from(h3s).find((h: Element) => h.textContent === "Summary");
    expect(summaryH3).toBeUndefined();
  });

});
