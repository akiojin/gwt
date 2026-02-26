import { describe, it, expect, vi, beforeEach } from "vitest";
import { render } from "@testing-library/svelte";
import { fireEvent } from "@testing-library/svelte";
import type { PrStatusInfo, ReviewInfo, ReviewComment, WorkflowRunInfo } from "../types";

vi.mock("$lib/tauriInvoke", () => ({
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

  it("shows update error while keeping PR detail visible", async () => {
    const pr = makePrDetail({ mergeStateStatus: "BEHIND" });
    const { container } = await renderSection({
      prDetail: pr,
      updateError: "Failed to update branch: 403 Forbidden",
    });

    expect(container.textContent).toContain("Failed to update branch: 403 Forbidden");
    expect(container.textContent).toContain("Add feature X");
    expect(container.querySelector(".update-branch-btn")).toBeTruthy();
    expect(container.querySelector(".pr-status-error")).toBeNull();
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

  it("renders merged badge when PR state is MERGED even if mergeable is UNKNOWN", async () => {
    const pr = makePrDetail({ state: "MERGED", mergeable: "UNKNOWN" });
    const { container } = await renderSection({ prDetail: pr });

    const badge = container.querySelector(".mergeable-badge");
    expect(badge).toBeTruthy();
    expect(badge?.classList.contains("merged")).toBe(true);
    expect(badge?.textContent).toContain("Merged");
    expect(badge?.textContent).not.toContain("Unknown");
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

  // --- T007: Checks section tests ---

  describe("Checks section", () => {
    it("does not render checks section when checkSuites is empty", async () => {
      const pr = makePrDetail({ checkSuites: [] });
      const { container } = await renderSection({ prDetail: pr });

      const checksSection = container.querySelector(".checks-section");
      expect(checksSection).toBeNull();
    });

    it("renders checks section with workflow runs", async () => {
      const checkSuites: WorkflowRunInfo[] = [
        { workflowName: "CI", runId: 1, status: "completed", conclusion: "success" },
        { workflowName: "Lint", runId: 2, status: "completed", conclusion: "failure" },
      ];
      const pr = makePrDetail({ checkSuites });
      const { container } = await renderSection({ prDetail: pr });

      const checksSection = container.querySelector(".checks-section");
      expect(checksSection).toBeTruthy();
      // Shows count in collapsed toggle
      expect(checksSection?.textContent).toContain("Checks (2)");

      // Expand to see individual workflow names
      const toggleBtn = container.querySelector(".checks-toggle") as HTMLElement;
      await fireEvent.click(toggleBtn);
      expect(checksSection?.textContent).toContain("CI");
      expect(checksSection?.textContent).toContain("Lint");
    });

    it("renders checks section collapsed by default with summary count", async () => {
      const checkSuites: WorkflowRunInfo[] = [
        { workflowName: "CI", runId: 1, status: "completed", conclusion: "success" },
        { workflowName: "Lint", runId: 2, status: "completed", conclusion: "failure" },
      ];
      const pr = makePrDetail({ checkSuites });
      const { container } = await renderSection({ prDetail: pr });

      // Should have a toggle button
      const toggleBtn = container.querySelector(".checks-toggle");
      expect(toggleBtn).toBeTruthy();

      // Individual checks should be hidden by default
      const checksList = container.querySelector(".checks-list");
      expect(checksList).toBeNull();
    });

    it("expands checks list when toggle is clicked", async () => {
      const checkSuites: WorkflowRunInfo[] = [
        { workflowName: "CI", runId: 1, status: "completed", conclusion: "success" },
      ];
      const pr = makePrDetail({ checkSuites });
      const { container } = await renderSection({ prDetail: pr });

      const toggleBtn = container.querySelector(".checks-toggle") as HTMLElement;
      expect(toggleBtn).toBeTruthy();

      await fireEvent.click(toggleBtn);

      const checksList = container.querySelector(".checks-list");
      expect(checksList).toBeTruthy();
      expect(checksList?.textContent).toContain("CI");
    });

    it("renders status icon and class for each workflow run", async () => {
      const checkSuites: WorkflowRunInfo[] = [
        { workflowName: "CI", runId: 1, status: "completed", conclusion: "success" },
        { workflowName: "Lint", runId: 2, status: "completed", conclusion: "failure" },
        { workflowName: "Deploy", runId: 3, status: "in_progress", conclusion: null },
      ];
      const pr = makePrDetail({ checkSuites });
      const { container } = await renderSection({ prDetail: pr });

      // Expand checks
      const toggleBtn = container.querySelector(".checks-toggle") as HTMLElement;
      await fireEvent.click(toggleBtn);

      const checkItems = container.querySelectorAll(".check-item");
      expect(checkItems).toHaveLength(3);

      // Check status icons are present
      const icons = container.querySelectorAll(".check-status");
      expect(icons).toHaveLength(3);
      expect(icons[0].classList.contains("pass")).toBe(true);
      expect(icons[1].classList.contains("fail")).toBe(true);
      expect(icons[2].classList.contains("running")).toBe(true);
    });

    it("calls onOpenCiLog with WorkflowRunInfo when a check item is clicked", async () => {
      const onOpenCiLog = vi.fn();
      const run: WorkflowRunInfo = { workflowName: "CI", runId: 123, status: "completed", conclusion: "failure" };
      const pr = makePrDetail({ checkSuites: [run] });
      const { container } = await renderSection({ prDetail: pr, onOpenCiLog });

      // Expand checks
      const toggleBtn = container.querySelector(".checks-toggle") as HTMLElement;
      await fireEvent.click(toggleBtn);

      const checkItem = container.querySelector(".check-item") as HTMLElement;
      await fireEvent.click(checkItem);

      expect(onOpenCiLog).toHaveBeenCalledOnce();
      expect(onOpenCiLog).toHaveBeenCalledWith(run);
    });

    it("shows conclusion text for each workflow run", async () => {
      const checkSuites: WorkflowRunInfo[] = [
        { workflowName: "CI", runId: 1, status: "completed", conclusion: "success" },
        { workflowName: "Lint", runId: 2, status: "in_progress", conclusion: null },
      ];
      const pr = makePrDetail({ checkSuites });
      const { container } = await renderSection({ prDetail: pr });

      const toggleBtn = container.querySelector(".checks-toggle") as HTMLElement;
      await fireEvent.click(toggleBtn);

      const conclusions = container.querySelectorAll(".check-conclusion");
      expect(conclusions).toHaveLength(2);
      expect(conclusions[0].textContent).toContain("Success");
      expect(conclusions[1].textContent).toContain("Running");
    });

    it("shows 'No checks' when checkSuites is empty and prDetail exists", async () => {
      const pr = makePrDetail({ checkSuites: [] });
      const { container } = await renderSection({ prDetail: pr });

      expect(container.textContent).toContain("No checks");
    });
  });

  // --- T013: isRequired badge ---

  describe("isRequired badge", () => {
    it("shows 'required' badge when isRequired is true", async () => {
      const checkSuites: WorkflowRunInfo[] = [
        { workflowName: "CI", runId: 1, status: "completed", conclusion: "success", isRequired: true },
        { workflowName: "Optional", runId: 2, status: "completed", conclusion: "success", isRequired: false },
      ];
      const pr = makePrDetail({ checkSuites });
      const { container } = await renderSection({ prDetail: pr });

      // Expand
      const toggleBtn = container.querySelector(".checks-toggle") as HTMLElement;
      await fireEvent.click(toggleBtn);

      const requiredBadges = container.querySelectorAll(".required-badge");
      expect(requiredBadges).toHaveLength(1);
      expect(requiredBadges[0].textContent).toContain("required");
    });
  });

  // --- T015: Merge meta row with mergeStateStatus + Update Branch button ---

  describe("Merge meta row with mergeStateStatus", () => {
    it("displays 'Behind base' badge when mergeStateStatus is BEHIND", async () => {
      const pr = makePrDetail({ mergeStateStatus: "BEHIND" });
      const { container } = await renderSection({ prDetail: pr });

      const stateStatusBadge = container.querySelector(".merge-state-badge");
      expect(stateStatusBadge).toBeTruthy();
      expect(stateStatusBadge?.textContent).toContain("Behind base");
      expect(stateStatusBadge?.classList.contains("behind")).toBe(true);
    });

    it("displays 'Blocked' badge when mergeStateStatus is BLOCKED", async () => {
      const pr = makePrDetail({ mergeStateStatus: "BLOCKED" });
      const { container } = await renderSection({ prDetail: pr });

      const stateStatusBadge = container.querySelector(".merge-state-badge");
      expect(stateStatusBadge).toBeTruthy();
      expect(stateStatusBadge?.textContent).toContain("Blocked");
      expect(stateStatusBadge?.classList.contains("blocked")).toBe(true);
    });

    it("shows only 'Blocked' when mergeStateStatus is BLOCKED and mergeable is MERGEABLE", async () => {
      const pr = makePrDetail({ mergeable: "MERGEABLE", mergeStateStatus: "BLOCKED" });
      const { container } = await renderSection({ prDetail: pr });

      const mergeableBadge = container.querySelector(".mergeable-badge");
      expect(mergeableBadge).toBeNull();
      const stateStatusBadge = container.querySelector(".merge-state-badge");
      expect(stateStatusBadge).toBeTruthy();
      expect(stateStatusBadge?.textContent).toContain("Blocked");
    });

    it("displays 'Conflicts' badge when mergeStateStatus is DIRTY", async () => {
      const pr = makePrDetail({ mergeStateStatus: "DIRTY" });
      const { container } = await renderSection({ prDetail: pr });

      const stateStatusBadge = container.querySelector(".merge-state-badge");
      expect(stateStatusBadge).toBeTruthy();
      expect(stateStatusBadge?.textContent).toContain("Conflicts");
      expect(stateStatusBadge?.classList.contains("blocked")).toBe(true);
    });

    it("does not display 'Conflicts' badge when mergeable is CONFLICTING", async () => {
      const pr = makePrDetail({ mergeable: "CONFLICTING", mergeStateStatus: "DIRTY" });
      const { container } = await renderSection({ prDetail: pr });

      const stateStatusBadge = container.querySelector(".merge-state-badge");
      expect(stateStatusBadge).toBeNull();
      const mergeableBadge = container.querySelector(".mergeable-badge");
      expect(mergeableBadge?.textContent).toContain("Conflicting");
    });

    it("displays 'Draft' badge when mergeStateStatus is DRAFT", async () => {
      const pr = makePrDetail({ mergeStateStatus: "DRAFT" });
      const { container } = await renderSection({ prDetail: pr });

      const stateStatusBadge = container.querySelector(".merge-state-badge");
      expect(stateStatusBadge).toBeTruthy();
      expect(stateStatusBadge?.textContent).toContain("Draft");
      expect(stateStatusBadge?.classList.contains("neutral")).toBe(true);
    });

    it("displays 'Unstable' badge when mergeStateStatus is UNSTABLE", async () => {
      const pr = makePrDetail({ mergeStateStatus: "UNSTABLE" });
      const { container } = await renderSection({ prDetail: pr });

      const stateStatusBadge = container.querySelector(".merge-state-badge");
      expect(stateStatusBadge).toBeTruthy();
      expect(stateStatusBadge?.textContent).toContain("Unstable");
      expect(stateStatusBadge?.classList.contains("unstable")).toBe(true);
    });

    it("does not display badge when mergeStateStatus is CLEAN", async () => {
      const pr = makePrDetail({ mergeStateStatus: "CLEAN" });
      const { container } = await renderSection({ prDetail: pr });

      const stateStatusBadge = container.querySelector(".merge-state-badge");
      expect(stateStatusBadge).toBeNull();
    });

    it("does not display badge when mergeStateStatus is HAS_HOOKS", async () => {
      const pr = makePrDetail({ mergeStateStatus: "HAS_HOOKS" });
      const { container } = await renderSection({ prDetail: pr });

      const stateStatusBadge = container.querySelector(".merge-state-badge");
      expect(stateStatusBadge).toBeNull();
    });

    it("does not display badge when mergeStateStatus is UNKNOWN", async () => {
      const pr = makePrDetail({ mergeStateStatus: "UNKNOWN" });
      const { container } = await renderSection({ prDetail: pr });

      const stateStatusBadge = container.querySelector(".merge-state-badge");
      expect(stateStatusBadge).toBeNull();
    });

    it("does not display badge when mergeStateStatus is null", async () => {
      const pr = makePrDetail({ mergeStateStatus: null });
      const { container } = await renderSection({ prDetail: pr });

      const stateStatusBadge = container.querySelector(".merge-state-badge");
      expect(stateStatusBadge).toBeNull();
    });

    it("does not display badge when mergeStateStatus is undefined", async () => {
      const pr = makePrDetail();
      const { container } = await renderSection({ prDetail: pr });

      const stateStatusBadge = container.querySelector(".merge-state-badge");
      expect(stateStatusBadge).toBeNull();
    });

    it("shows Update Branch button when mergeStateStatus is BEHIND", async () => {
      const pr = makePrDetail({ mergeStateStatus: "BEHIND" });
      const { container } = await renderSection({ prDetail: pr });

      const updateBtn = container.querySelector(".update-branch-btn");
      expect(updateBtn).toBeTruthy();
      expect(updateBtn?.textContent).toContain("Update Branch");
    });

    it("does not show Update Branch button when mergeStateStatus is CLEAN", async () => {
      const pr = makePrDetail({ mergeStateStatus: "CLEAN" });
      const { container } = await renderSection({ prDetail: pr });

      const updateBtn = container.querySelector(".update-branch-btn");
      expect(updateBtn).toBeNull();
    });

    it("calls onUpdateBranch when Update Branch button is clicked", async () => {
      const onUpdateBranch = vi.fn().mockResolvedValue(undefined);
      const pr = makePrDetail({ mergeStateStatus: "BEHIND" });
      const { container } = await renderSection({ prDetail: pr, onUpdateBranch });

      const updateBtn = container.querySelector(".update-branch-btn") as HTMLElement;
      await fireEvent.click(updateBtn);

      expect(onUpdateBranch).toHaveBeenCalledOnce();
    });

    it("disables Update Branch button when updatingBranch is true", async () => {
      const onUpdateBranch = vi.fn().mockResolvedValue(undefined);
      const pr = makePrDetail({ mergeStateStatus: "BEHIND" });
      const { container } = await renderSection({
        prDetail: pr,
        onUpdateBranch,
        updatingBranch: true,
      });

      const updateBtn = container.querySelector(".update-branch-btn") as HTMLButtonElement;
      expect(updateBtn.disabled).toBe(true);
      expect(updateBtn.textContent).toContain("Updating");
    });

    it("does not show mergeStateStatus badge when PR state is MERGED", async () => {
      const pr = makePrDetail({ state: "MERGED", mergeStateStatus: "BEHIND" });
      const { container } = await renderSection({ prDetail: pr });

      const stateStatusBadge = container.querySelector(".merge-state-badge");
      expect(stateStatusBadge).toBeNull();
    });

    it("does not show Update Branch button when PR state is MERGED", async () => {
      const pr = makePrDetail({ state: "MERGED", mergeStateStatus: "BEHIND" });
      const { container } = await renderSection({ prDetail: pr });

      const updateBtn = container.querySelector(".update-branch-btn");
      expect(updateBtn).toBeNull();
    });
  });

  // --- SPEC-merge-pr: Mergeable badge button tests ---

  describe("Mergeable badge button (SPEC-merge-pr)", () => {
    it("renders button when state=OPEN, mergeable=MERGEABLE, and onMerge is provided", async () => {
      const onMerge = vi.fn();
      const pr = makePrDetail({ state: "OPEN", mergeable: "MERGEABLE" });
      const { container } = await renderSection({ prDetail: pr, onMerge });

      const btn = container.querySelector(".mergeable-badge-btn");
      expect(btn).toBeTruthy();
      expect(btn?.tagName).toBe("BUTTON");
      expect(btn?.textContent).toContain("Mergeable");
    });

    it("renders span (not button) when state=CLOSED", async () => {
      const onMerge = vi.fn();
      const pr = makePrDetail({ state: "CLOSED", mergeable: "MERGEABLE" });
      const { container } = await renderSection({ prDetail: pr, onMerge });

      const btn = container.querySelector(".mergeable-badge-btn");
      expect(btn).toBeNull();
      const span = container.querySelector(".mergeable-badge");
      expect(span).toBeTruthy();
    });

    it("renders span when mergeable=CONFLICTING", async () => {
      const onMerge = vi.fn();
      const pr = makePrDetail({ state: "OPEN", mergeable: "CONFLICTING" });
      const { container } = await renderSection({ prDetail: pr, onMerge });

      const btn = container.querySelector(".mergeable-badge-btn");
      expect(btn).toBeNull();
    });

    it("renders span when mergeable=UNKNOWN", async () => {
      const onMerge = vi.fn();
      const pr = makePrDetail({ state: "OPEN", mergeable: "UNKNOWN" });
      const { container } = await renderSection({ prDetail: pr, onMerge });

      const btn = container.querySelector(".mergeable-badge-btn");
      expect(btn).toBeNull();
    });

    it("renders span when onMerge is not provided even if OPEN+MERGEABLE", async () => {
      const pr = makePrDetail({ state: "OPEN", mergeable: "MERGEABLE" });
      const { container } = await renderSection({ prDetail: pr });

      const btn = container.querySelector(".mergeable-badge-btn");
      expect(btn).toBeNull();
      const span = container.querySelector(".mergeable-badge");
      expect(span).toBeTruthy();
    });

    it("calls onMerge when button is clicked", async () => {
      const onMerge = vi.fn();
      const pr = makePrDetail({ state: "OPEN", mergeable: "MERGEABLE" });
      const { container } = await renderSection({ prDetail: pr, onMerge });

      const btn = container.querySelector(".mergeable-badge-btn") as HTMLElement;
      await fireEvent.click(btn);
      expect(onMerge).toHaveBeenCalledOnce();
    });

    it("shows disabled button with Merging... when merging=true", async () => {
      const onMerge = vi.fn();
      const pr = makePrDetail({ state: "OPEN", mergeable: "MERGEABLE" });
      const { container } = await renderSection({ prDetail: pr, onMerge, merging: true });

      const btn = container.querySelector(".mergeable-badge-btn") as HTMLButtonElement;
      expect(btn).toBeTruthy();
      expect(btn.disabled).toBe(true);
      expect(btn.textContent).toContain("Merging...");
    });

    it("renders button when BEHIND but MERGEABLE", async () => {
      const onMerge = vi.fn();
      const pr = makePrDetail({ state: "OPEN", mergeable: "MERGEABLE", mergeStateStatus: "BEHIND" });
      const { container } = await renderSection({ prDetail: pr, onMerge });

      const btn = container.querySelector(".mergeable-badge-btn");
      expect(btn).toBeTruthy();
    });

    it("does not render button when state=MERGED", async () => {
      const onMerge = vi.fn();
      const pr = makePrDetail({ state: "MERGED", mergeable: "UNKNOWN" });
      const { container } = await renderSection({ prDetail: pr, onMerge });

      const btn = container.querySelector(".mergeable-badge-btn");
      expect(btn).toBeNull();
    });
  });

  // --- T007: retrying prop – pulse animation & merge button control ---

  describe("retrying prop", () => {
    it("applies .pulse class to mergeable-badge when retrying=true", async () => {
      const onMerge = vi.fn();
      const pr = makePrDetail({ state: "OPEN", mergeable: "MERGEABLE" });
      const { container } = await renderSection({ prDetail: pr, onMerge, retrying: true });

      const badge = container.querySelector(".mergeable-badge");
      expect(badge).toBeTruthy();
      expect(badge?.classList.contains("pulse")).toBe(true);
    });

    it("disables merge button when retrying=true", async () => {
      const onMerge = vi.fn();
      const pr = makePrDetail({ state: "OPEN", mergeable: "MERGEABLE" });
      const { container } = await renderSection({ prDetail: pr, onMerge, retrying: true });

      const btn = container.querySelector(".mergeable-badge-btn") as HTMLButtonElement;
      expect(btn).toBeTruthy();
      expect(btn.disabled).toBe(true);
    });

    it("shows 'Checking merge status...' text when retrying=true", async () => {
      const onMerge = vi.fn();
      const pr = makePrDetail({ state: "OPEN", mergeable: "MERGEABLE" });
      const { container } = await renderSection({ prDetail: pr, onMerge, retrying: true });

      const btn = container.querySelector(".mergeable-badge-btn");
      expect(btn?.textContent).toContain("Checking merge status...");
    });

    it("does not apply .pulse class when retrying=false (default)", async () => {
      const onMerge = vi.fn();
      const pr = makePrDetail({ state: "OPEN", mergeable: "MERGEABLE" });
      const { container } = await renderSection({ prDetail: pr, onMerge });

      const badge = container.querySelector(".mergeable-badge");
      expect(badge).toBeTruthy();
      expect(badge?.classList.contains("pulse")).toBe(false);
    });

    it("applies .pulse class to span badge when retrying=true and not merge-clickable", async () => {
      const pr = makePrDetail({ state: "OPEN", mergeable: "UNKNOWN" });
      const { container } = await renderSection({ prDetail: pr, retrying: true });

      const badge = container.querySelector(".mergeable-badge");
      expect(badge).toBeTruthy();
      expect(badge?.classList.contains("pulse")).toBe(true);
    });
  });
});
