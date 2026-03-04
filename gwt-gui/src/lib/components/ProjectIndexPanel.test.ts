import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("../openExternalUrl", () => ({
  openExternalUrl: vi.fn(),
}));

async function renderProjectIndexPanel(projectPath = "/tmp/project") {
  const { default: ProjectIndexPanel } = await import("./ProjectIndexPanel.svelte");
  return render(ProjectIndexPanel, {
    props: {
      projectPath,
    },
  });
}

describe("ProjectIndexPanel", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_index_status_cmd") {
        return {
          indexed: true,
          totalFiles: 12911,
          dbSizeBytes: 82_087_321,
        };
      }

      if (command === "search_project_index_cmd") {
        return [];
      }

      if (command === "index_github_issues_cmd") {
        return {
          issuesIndexed: 0,
          durationMs: 1,
        };
      }

      if (command === "search_github_issues_cmd") {
        return [];
      }

      throw new Error(`unexpected invoke command: ${command}`);
    });
  });

  afterEach(() => {
    cleanup();
  });

  it("does not show no-results message before a files search is executed", async () => {
    const rendered = await renderProjectIndexPanel();

    await waitFor(() => {
      expect(rendered.getByText("12911 files indexed")).toBeTruthy();
    });

    const input = rendered.getByPlaceholderText("Search project files...") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "Git" } });

    expect(rendered.queryByText("No results found")).toBeNull();
    expect(invokeMock).not.toHaveBeenCalledWith(
      "search_project_index_cmd",
      expect.anything(),
    );
  });

  it("shows no-results message after a files search returns an empty list", async () => {
    const rendered = await renderProjectIndexPanel();

    await waitFor(() => {
      expect(rendered.getByText("12911 files indexed")).toBeTruthy();
    });

    const input = rendered.getByPlaceholderText("Search project files...") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "Git" } });
    await fireEvent.click(rendered.getByRole("button", { name: "Search" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("search_project_index_cmd", {
        projectRoot: "/tmp/project",
        query: "Git",
        nResults: 20,
      });
    });

    await waitFor(() => {
      expect(rendered.getByText("No results found")).toBeTruthy();
    });
  });

  it("hides stale no-results message when the query is edited after search", async () => {
    const rendered = await renderProjectIndexPanel();

    await waitFor(() => {
      expect(rendered.getByText("12911 files indexed")).toBeTruthy();
    });

    const input = rendered.getByPlaceholderText("Search project files...") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "Git" } });
    await fireEvent.click(rendered.getByRole("button", { name: "Search" }));

    await waitFor(() => {
      expect(rendered.getByText("No results found")).toBeTruthy();
    });

    await fireEvent.input(input, { target: { value: "Rust" } });
    expect(rendered.queryByText("No results found")).toBeNull();
  });
});
