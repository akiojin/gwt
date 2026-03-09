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

  it("shows a Python installation notice instead of a raw status error", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_index_status_cmd") {
        throw new Error(
          "Get index status task failed: Chroma helper failed (status=exit code: 9009): Python",
        );
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

    const rendered = await renderProjectIndexPanel();

    await waitFor(() => {
      expect(rendered.getByText("Project Index requires Python 3.11+")).toBeTruthy();
    });

    expect(
      rendered.getByText(/Install Python 3\.11 or later, then reopen Project Index\./),
    ).toBeTruthy();
    expect(rendered.queryByText(/Failed to load index status:/)).toBeNull();
  });

  it("shows a Python installation notice instead of a raw search error", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_index_status_cmd") {
        return {
          indexed: false,
          totalFiles: 0,
          dbSizeBytes: 0,
        };
      }

      if (command === "search_project_index_cmd") {
        throw new Error(
          "Search project index task failed: Python runtime not found (checked python3.13/python3.12/python3.11/python3/py/python)",
        );
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

    const rendered = await renderProjectIndexPanel();

    const input = rendered.getByPlaceholderText("Search project files...") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "Git" } });
    await fireEvent.click(rendered.getByRole("button", { name: "Search" }));

    await waitFor(() => {
      expect(rendered.getByText("Project Index requires Python 3.11+")).toBeTruthy();
    });

    expect(rendered.queryByText(/Search project index task failed:/)).toBeNull();
    expect(rendered.queryByText("No results found")).toBeNull();
  });
});
