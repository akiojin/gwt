import { describe, it, expect, vi, beforeEach } from "vitest";
import { render } from "@testing-library/svelte";
import type { PrStatusInfo, ReviewInfo, ReviewComment } from "../types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

async function renderSection(props: Record<string, unknown>) {
  const { default: PrStatusSection } = await import("./PrStatusSection.svelte");
  return render(PrStatusSection, { props });
}

function makePrDetail(overrides: Partial<PrStatusInfo> = {}): PrStatusInfo {
  return {
    number: 42,
    title: "Add feature X",
    state: "OPEN",
    url: "https://github.com/owner/repo/pull/42",
    mergeable: "MERGEABLE",
    author: "alice",
    baseBranch: "main",
    headBranch: "feature/x",
    labels: [],
    assignees: [],
    milestone: null,
    linkedIssues: [],
    checkSuites: [],
    reviews: [],
    reviewComments: [],
    changedFilesCount: 5,
    additions: 100,
    deletions: 20,
    ...overrides,
  };
}

describe("PrStatusSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows 'No PR' when prDetail is null", async () => {
    const { container } = await renderSection({ prDetail: null });
    expect(container.textContent).toContain("No PR");
  });

  it("shows 'Loading...' when loading is true", async () => {
    const { container } = await renderSection({ loading: true });
    expect(container.textContent).toContain("Loading...");
  });

  it("shows error message when error is provided", async () => {
    const { container } = await renderSection({ error: "Something went wrong" });
    expect(container.textContent).toContain("Something went wrong");
  });

  it("renders PR metadata correctly", async () => {
    const pr = makePrDetail({
      title: "Implement login",
      author: "bob",
      baseBranch: "main",
      headBranch: "feature/login",
    });
    const { container } = await renderSection({ prDetail: pr });

    expect(container.textContent).toContain("Implement login");
    expect(container.textContent).toContain("bob");
    expect(container.textContent).toContain("main");
    expect(container.textContent).toContain("feature/login");

    // Title should be a link
    const link = container.querySelector("a");
    expect(link).toBeTruthy();
    expect(link?.getAttribute("href")).toBe("https://github.com/owner/repo/pull/42");
    expect(link?.getAttribute("target")).toBe("_blank");
  });

  it("renders labels as pills", async () => {
    const pr = makePrDetail({ labels: ["bug", "urgent"] });
    const { container } = await renderSection({ prDetail: pr });

    const pills = container.querySelectorAll(".label-pill");
    expect(pills).toHaveLength(2);
    expect(pills[0].textContent).toContain("bug");
    expect(pills[1].textContent).toContain("urgent");
  });

  it("renders assignees", async () => {
    const pr = makePrDetail({ assignees: ["charlie", "dave"] });
    const { container } = await renderSection({ prDetail: pr });

    expect(container.textContent).toContain("charlie");
    expect(container.textContent).toContain("dave");
  });

  it("renders milestone when present", async () => {
    const pr = makePrDetail({ milestone: "v2.0" });
    const { container } = await renderSection({ prDetail: pr });
    expect(container.textContent).toContain("v2.0");
  });

  it("renders linked issues", async () => {
    const pr = makePrDetail({ linkedIssues: [10, 25] });
    const { container } = await renderSection({ prDetail: pr });
    expect(container.textContent).toContain("#10");
    expect(container.textContent).toContain("#25");
  });

  it("renders mergeable badge with correct class - MERGEABLE", async () => {
    const pr = makePrDetail({ mergeable: "MERGEABLE" });
    const { container } = await renderSection({ prDetail: pr });

    const badge = container.querySelector(".mergeable-badge");
    expect(badge).toBeTruthy();
    expect(badge?.classList.contains("mergeable")).toBe(true);
    expect(badge?.textContent).toContain("Mergeable");
  });

  it("renders mergeable badge with correct class - CONFLICTING", async () => {
    const pr = makePrDetail({ mergeable: "CONFLICTING" });
    const { container } = await renderSection({ prDetail: pr });

    const badge = container.querySelector(".mergeable-badge");
    expect(badge).toBeTruthy();
    expect(badge?.classList.contains("conflicting")).toBe(true);
    expect(badge?.textContent).toContain("Conflicting");
  });

  it("renders mergeable badge with correct class - UNKNOWN", async () => {
    const pr = makePrDetail({ mergeable: "UNKNOWN" });
    const { container } = await renderSection({ prDetail: pr });

    const badge = container.querySelector(".mergeable-badge");
    expect(badge).toBeTruthy();
    expect(badge?.classList.contains("unknown")).toBe(true);
    expect(badge?.textContent).toContain("Unknown");
  });

  it("renders reviews with correct state icons", async () => {
    const reviews: ReviewInfo[] = [
      { reviewer: "alice", state: "APPROVED" },
      { reviewer: "bob", state: "CHANGES_REQUESTED" },
      { reviewer: "charlie", state: "COMMENTED" },
      { reviewer: "dave", state: "PENDING" },
      { reviewer: "eve", state: "DISMISSED" },
    ];
    const pr = makePrDetail({ reviews });
    const { container } = await renderSection({ prDetail: pr });

    const reviewItems = container.querySelectorAll(".review-item");
    expect(reviewItems).toHaveLength(5);

    expect(container.textContent).toContain("alice");
    expect(container.textContent).toContain("bob");

    // Check state classes
    const stateElements = container.querySelectorAll(".review-state");
    expect(stateElements[0].classList.contains("approved")).toBe(true);
    expect(stateElements[1].classList.contains("changes_requested")).toBe(true);
    expect(stateElements[2].classList.contains("commented")).toBe(true);
    expect(stateElements[3].classList.contains("pending")).toBe(true);
    expect(stateElements[4].classList.contains("dismissed")).toBe(true);
  });

  it("does not render reviews section when no reviews", async () => {
    const pr = makePrDetail({ reviews: [] });
    const { container } = await renderSection({ prDetail: pr });

    const reviewsSection = container.querySelector(".reviews-section");
    expect(reviewsSection).toBeNull();
  });

  it("renders review comments with code snippets", async () => {
    const comments: ReviewComment[] = [
      {
        author: "alice",
        body: "Consider refactoring this function",
        filePath: "src/main.rs",
        line: 42,
        codeSnippet: "fn main() {\n  println!(\"hello\");\n}",
        createdAt: "2025-01-01T00:00:00Z",
      },
    ];
    const pr = makePrDetail({ reviewComments: comments });
    const { container } = await renderSection({ prDetail: pr });

    expect(container.textContent).toContain("alice");
    expect(container.textContent).toContain("Consider refactoring this function");
    expect(container.textContent).toContain("src/main.rs:42");

    const codeSnippet = container.querySelector(".code-snippet");
    expect(codeSnippet).toBeTruthy();
    expect(codeSnippet?.textContent).toContain("fn main()");
  });

  it("does not render comments section when no comments", async () => {
    const pr = makePrDetail({ reviewComments: [] });
    const { container } = await renderSection({ prDetail: pr });

    const commentsSection = container.querySelector(".comments-section");
    expect(commentsSection).toBeNull();
  });

  it("renders changes summary", async () => {
    const pr = makePrDetail({
      changedFilesCount: 12,
      additions: 350,
      deletions: 80,
    });
    const { container } = await renderSection({ prDetail: pr });

    expect(container.textContent).toContain("12 files changed");
    expect(container.textContent).toContain("+350");
    expect(container.textContent).toContain("-80");
  });
});
