import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor } from "@testing-library/svelte";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

async function renderPanel(props: any) {
  const { default: Panel } = await import("./VersionHistoryPanel.svelte");
  return render(Panel, { props });
}

describe("VersionHistoryPanel", () => {
  beforeEach(() => {
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
});
