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

vi.mock("@tauri-apps/api/core", () => ({
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

  it("continues sequential generation on history-updated events for the active project", async () => {
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
      if (cmd === "get_project_version_history" && args?.versionId === "unreleased") {
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
      if (cmd === "get_project_version_history" && args?.versionId === "v1.0.0") {
        return {
          status: "ok",
          version_id: "v1.0.0",
          label: "v1.0.0",
          range_from: null,
          range_to: "v1.0.0",
          commit_count: 1,
          summary_markdown: "## Done",
          changelog_markdown: null,
          error: null,
        };
      }
      return [];
    });

    await renderPanel({ projectPath: "/tmp/project" });
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_project_version_history", {
        projectPath: "/tmp/project",
        versionId: "unreleased",
      });
    });

    const before = invokeMock.mock.calls.filter(
      (call) => call[0] === "get_project_version_history" && call[1]?.versionId === "v1.0.0"
    ).length;

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

    const afterOtherPath = invokeMock.mock.calls.filter(
      (call) => call[0] === "get_project_version_history" && call[1]?.versionId === "v1.0.0"
    ).length;
    expect(afterOtherPath).toBe(before);

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
        summary_markdown: "## summary",
        changelog_markdown: null,
        error: null,
      },
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_project_version_history", {
        projectPath: "/tmp/project",
        versionId: "v1.0.0",
      });
    });
  });

});
