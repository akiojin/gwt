import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

async function renderPanel(props: any) {
  const { default: IssueSpecPanel } = await import("./IssueSpecPanel.svelte");
  return render(IssueSpecPanel, { props });
}

const detailFixture = {
  number: 42,
  title: "Implement auth feature",
  url: "https://github.com/test/repo/issues/42",
  updatedAt: "2026-01-15T10:00:00Z",
  specId: "SPEC-abc123",
  etag: "W/\"etag-value\"",
  body: "Full issue body",
  sections: {
    spec: "Specification content",
    plan: "Plan content",
    tasks: "Task list",
    tdd: "TDD approach",
    research: "Research notes",
    dataModel: "Data model description",
    quickstart: "Quickstart instructions",
    contracts: "API contracts",
    checklists: "Review checklists",
  },
};

describe("IssueSpecPanel", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    cleanup();
  });

  afterEach(() => {
    cleanup();
  });

  it("shows loading state initially", async () => {
    invokeMock.mockImplementation(() => new Promise(() => {})); // never resolves

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      issueNumber: 42,
    });

    expect(rendered.getByText("Loading issue spec...")).toBeTruthy();
  });

  it("renders issue spec header with issue number", async () => {
    invokeMock.mockResolvedValue(detailFixture);

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      issueNumber: 42,
    });

    expect(rendered.getByText("Issue Spec")).toBeTruthy();
    expect(rendered.getByText("#42")).toBeTruthy();
  });

  it("renders specId in header when provided", async () => {
    invokeMock.mockResolvedValue(detailFixture);

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      issueNumber: 42,
      specId: "SPEC-xyz",
    });

    await waitFor(() => {
      expect(rendered.getByText("SPEC-xyz")).toBeTruthy();
    });
  });

  it("calls get_spec_issue_detail_cmd with correct args", async () => {
    invokeMock.mockResolvedValue(detailFixture);

    await renderPanel({
      projectPath: "/tmp/project",
      issueNumber: 42,
    });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_spec_issue_detail_cmd", {
        projectPath: "/tmp/project",
        issueNumber: 42,
      });
    });
  });

  it("shows error message on failure", async () => {
    invokeMock.mockRejectedValue(new Error("Not found"));

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      issueNumber: 99,
    });

    await waitFor(() => {
      expect(rendered.getByText("Not found")).toBeTruthy();
    });
  });

  it("shows _TODO_ for empty sections and renders all 9 headings", async () => {
    invokeMock.mockResolvedValue({
      ...detailFixture,
      sections: {
        spec: "",
        plan: "  ",
        tasks: "",
        tdd: "",
        research: "",
        dataModel: "",
        quickstart: "",
        contracts: "",
        checklists: "",
      },
    });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      issueNumber: 42,
    });

    await waitFor(() => {
      expect(rendered.getByText("Implement auth feature")).toBeTruthy();
    });

    // All 9 section headings are present
    const headings = rendered.container.querySelectorAll("article.section h3");
    const headingTexts = Array.from(headings).map((h) => h.textContent);
    expect(headingTexts).toEqual([
      "Spec", "Plan", "Tasks", "TDD", "Research",
      "Data Model", "Quickstart", "Contracts", "Checklists",
    ]);

    const todoElements = rendered.getAllByText("_TODO_");
    expect(todoElements.length).toBeGreaterThanOrEqual(9);
  });

  it("renders metadata row with link, updated date, and etag", async () => {
    invokeMock.mockResolvedValue(detailFixture);

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      issueNumber: 42,
    });

    await waitFor(() => {
      expect(rendered.getByText("Open on GitHub")).toBeTruthy();
    });

    const link = rendered.getByText("Open on GitHub") as HTMLAnchorElement;
    expect(link.getAttribute("href")).toBe("https://github.com/test/repo/issues/42");
    expect(link.getAttribute("target")).toBe("_blank");

    expect(rendered.getByText(/Updated:/)).toBeTruthy();
    expect(rendered.getByText(/ETag:/)).toBeTruthy();
  });

  it("shows Issue not found when detail is null", async () => {
    invokeMock.mockResolvedValue(null);

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      issueNumber: 42,
    });

    await waitFor(() => {
      expect(rendered.getByText("Issue not found.")).toBeTruthy();
    });
  });

  it("handles string error from invoke", async () => {
    invokeMock.mockRejectedValue("Some string error");

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      issueNumber: 42,
    });

    await waitFor(() => {
      expect(rendered.getByText("Some string error")).toBeTruthy();
    });
  });

  it("handles non-standard error from invoke", async () => {
    invokeMock.mockRejectedValue({ code: 404 });

    const rendered = await renderPanel({
      projectPath: "/tmp/project",
      issueNumber: 42,
    });

    await waitFor(() => {
      const errorEl = rendered.container.querySelector(".issue-spec-error");
      expect(errorEl).toBeTruthy();
    });
  });
});
