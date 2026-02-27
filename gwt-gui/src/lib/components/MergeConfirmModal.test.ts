import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import type { PrStatusInfo, WorkflowRunInfo, ReviewInfo } from "../types";

vi.mock("$lib/tauriInvoke", () => ({
  invoke: vi.fn(),
}));

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function renderModal(props: Record<string, unknown>) {
  const { default: MergeConfirmModal } = await import("./MergeConfirmModal.svelte");
  return render(MergeConfirmModal as any, { props });
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

describe("MergeConfirmModal", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders nothing when open is false", async () => {
    const { container } = await renderModal({
      open: false,
      prDetail: makePrDetail(),
      merging: false,
      onClose: vi.fn(),
      onConfirm: vi.fn(),
    });
    expect(container.querySelector(".modal-overlay")).toBeNull();
  });

  it("renders modal with PR info when open is true", async () => {
    const pr = makePrDetail({ number: 99, title: "Fix bug Y", headBranch: "fix/bug-y", baseBranch: "develop" });
    const { container } = await renderModal({
      open: true,
      prDetail: pr,
      merging: false,
      onClose: vi.fn(),
      onConfirm: vi.fn(),
    });
    expect(container.querySelector(".modal-overlay")).toBeTruthy();
    expect(container.textContent).toContain("#99");
    expect(container.textContent).toContain("Fix bug Y");
    expect(container.textContent).toContain("fix/bug-y");
    expect(container.textContent).toContain("develop");
  });

  it("calls onConfirm when Merge button is clicked", async () => {
    const onConfirm = vi.fn();
    const { container } = await renderModal({
      open: true,
      prDetail: makePrDetail(),
      merging: false,
      onClose: vi.fn(),
      onConfirm,
    });
    const mergeBtn = container.querySelector(".btn-merge") as HTMLElement;
    expect(mergeBtn).toBeTruthy();
    await fireEvent.click(mergeBtn);
    expect(onConfirm).toHaveBeenCalledOnce();
  });

  it("calls onClose when Cancel button is clicked", async () => {
    const onClose = vi.fn();
    const { container } = await renderModal({
      open: true,
      prDetail: makePrDetail(),
      merging: false,
      onClose,
      onConfirm: vi.fn(),
    });
    const cancelBtn = container.querySelector(".btn-cancel") as HTMLElement;
    expect(cancelBtn).toBeTruthy();
    await fireEvent.click(cancelBtn);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("calls onClose when Escape is pressed without moving focus into modal", async () => {
    const onClose = vi.fn();
    await renderModal({
      open: true,
      prDetail: makePrDetail(),
      merging: false,
      onClose,
      onConfirm: vi.fn(),
    });
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("disables Merge button and shows Merging... when merging is true", async () => {
    const { container } = await renderModal({
      open: true,
      prDetail: makePrDetail(),
      merging: true,
      onClose: vi.fn(),
      onConfirm: vi.fn(),
    });
    const mergeBtn = container.querySelector(".btn-merge") as HTMLButtonElement;
    expect(mergeBtn.disabled).toBe(true);
    expect(mergeBtn.textContent).toContain("Merging...");
  });

  it("shows check suite status icons when checkSuites are present", async () => {
    const checkSuites: WorkflowRunInfo[] = [
      { workflowName: "CI", runId: 1, status: "completed", conclusion: "success" },
      { workflowName: "Lint", runId: 2, status: "completed", conclusion: "failure" },
    ];
    const pr = makePrDetail({ checkSuites });
    const { container } = await renderModal({
      open: true,
      prDetail: pr,
      merging: false,
      onClose: vi.fn(),
      onConfirm: vi.fn(),
    });
    expect(container.textContent).toContain("CI");
    expect(container.textContent).toContain("Lint");
  });

  it("shows review status when reviews are present", async () => {
    const reviews: ReviewInfo[] = [
      { reviewer: "bob", state: "APPROVED" },
    ];
    const pr = makePrDetail({ reviews });
    const { container } = await renderModal({
      open: true,
      prDetail: pr,
      merging: false,
      onClose: vi.fn(),
      onConfirm: vi.fn(),
    });
    expect(container.textContent).toContain("bob");
  });

  it("renders all review state icons", async () => {
    // Exercise every branch of reviewStateIcon: APPROVED, CHANGES_REQUESTED, COMMENTED, PENDING, DISMISSED, default
    const reviews: ReviewInfo[] = [
      { reviewer: "alice", state: "APPROVED" },
      { reviewer: "bob", state: "CHANGES_REQUESTED" },
      { reviewer: "carol", state: "COMMENTED" },
      { reviewer: "dave", state: "PENDING" },
      { reviewer: "eve", state: "DISMISSED" },
    ];
    const pr = makePrDetail({ reviews });
    const { container } = await renderModal({
      open: true,
      prDetail: pr,
      merging: false,
      onClose: vi.fn(),
      onConfirm: vi.fn(),
    });

    expect(container.textContent).toContain("alice");
    expect(container.textContent).toContain("bob");
    expect(container.textContent).toContain("carol");
    expect(container.textContent).toContain("dave");
    expect(container.textContent).toContain("eve");

    // Verify review state icons are rendered
    const reviewItems = container.querySelectorAll(".merge-review-item");
    expect(reviewItems.length).toBe(5);

    // Check the review state classes
    const stateSpans = container.querySelectorAll(".review-state");
    expect(stateSpans.length).toBe(5);

    // Verify each state class is applied
    const classes = Array.from(stateSpans).map((el) => el.className);
    expect(classes.some((c) => c.includes("approved"))).toBe(true);
    expect(classes.some((c) => c.includes("changes_requested"))).toBe(true);
    expect(classes.some((c) => c.includes("commented"))).toBe(true);
    expect(classes.some((c) => c.includes("pending"))).toBe(true);
    expect(classes.some((c) => c.includes("dismissed"))).toBe(true);
  });

  it("renders unknown review state with fallback icon", async () => {
    // Exercise the default branch in reviewStateIcon
    const reviews = [
      { reviewer: "unknown-user", state: "UNKNOWN_STATE" as any },
    ];
    const pr = makePrDetail({ reviews });
    const { container } = await renderModal({
      open: true,
      prDetail: pr,
      merging: false,
      onClose: vi.fn(),
      onConfirm: vi.fn(),
    });

    expect(container.textContent).toContain("unknown-user");
    expect(container.textContent).toContain("?");
  });

  it("does not render checks section when checkSuites is empty", async () => {
    const pr = makePrDetail({ checkSuites: [] });
    const { container } = await renderModal({
      open: true,
      prDetail: pr,
      merging: false,
      onClose: vi.fn(),
      onConfirm: vi.fn(),
    });

    expect(container.querySelector(".merge-checks")).toBeNull();
  });

  it("does not render reviews section when reviews is empty", async () => {
    const pr = makePrDetail({ reviews: [] });
    const { container } = await renderModal({
      open: true,
      prDetail: pr,
      merging: false,
      onClose: vi.fn(),
      onConfirm: vi.fn(),
    });

    expect(container.querySelector(".merge-reviews")).toBeNull();
  });

  it("does not call onClose for non-Escape keys", async () => {
    const onClose = vi.fn();
    await renderModal({
      open: true,
      prDetail: makePrDetail(),
      merging: false,
      onClose,
      onConfirm: vi.fn(),
    });

    await fireEvent.keyDown(window, { key: "Enter" });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("does not call onClose on Escape when modal is not open", async () => {
    const onClose = vi.fn();
    await renderModal({
      open: false,
      prDetail: makePrDetail(),
      merging: false,
      onClose,
      onConfirm: vi.fn(),
    });

    await fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("does not call onClose on Escape when prDetail is null", async () => {
    const onClose = vi.fn();
    await renderModal({
      open: true,
      prDetail: null,
      merging: false,
      onClose,
      onConfirm: vi.fn(),
    });

    await fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("calls onClose when overlay is clicked", async () => {
    const onClose = vi.fn();
    const { container } = await renderModal({
      open: true,
      prDetail: makePrDetail(),
      merging: false,
      onClose,
      onConfirm: vi.fn(),
    });

    const overlay = container.querySelector(".modal-overlay") as HTMLElement;
    expect(overlay).toBeTruthy();
    await fireEvent.click(overlay);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("does not call onClose when dialog body is clicked (stopPropagation)", async () => {
    const onClose = vi.fn();
    const { container } = await renderModal({
      open: true,
      prDetail: makePrDetail(),
      merging: false,
      onClose,
      onConfirm: vi.fn(),
    });

    const dialog = container.querySelector(".merge-dialog") as HTMLElement;
    expect(dialog).toBeTruthy();
    await fireEvent.click(dialog);
    expect(onClose).not.toHaveBeenCalled();
  });

  it("calls onClose when close button (x) is clicked", async () => {
    const onClose = vi.fn();
    const { container } = await renderModal({
      open: true,
      prDetail: makePrDetail(),
      merging: false,
      onClose,
      onConfirm: vi.fn(),
    });

    const closeBtn = container.querySelector(".close-btn") as HTMLElement;
    expect(closeBtn).toBeTruthy();
    await fireEvent.click(closeBtn);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("re-renders from open to closed to exercise teardown", async () => {
    const { default: MergeConfirmModal } = await import("./MergeConfirmModal.svelte");
    const reviews: ReviewInfo[] = [
      { reviewer: "alice", state: "APPROVED" },
      { reviewer: "bob", state: "CHANGES_REQUESTED" },
    ];
    const checkSuites: WorkflowRunInfo[] = [
      { workflowName: "CI", runId: 1, status: "completed", conclusion: "success" },
    ];
    const rendered = render(MergeConfirmModal as any, {
      props: {
        open: true,
        prDetail: makePrDetail({ reviews, checkSuites }),
        merging: false,
        onClose: vi.fn(),
        onConfirm: vi.fn(),
      },
    });

    expect(rendered.container.querySelector(".modal-overlay")).toBeTruthy();

    // Close the modal
    await rendered.rerender({
      open: false,
      prDetail: null,
      merging: false,
      onClose: vi.fn(),
      onConfirm: vi.fn(),
    });

    expect(rendered.container.querySelector(".modal-overlay")).toBeNull();
  });

  it("unmounts with full content to exercise cleanup branches", async () => {
    const reviews: ReviewInfo[] = [
      { reviewer: "alice", state: "APPROVED" },
    ];
    const checkSuites: WorkflowRunInfo[] = [
      { workflowName: "CI", runId: 1, status: "completed", conclusion: "success" },
    ];
    const rendered = await renderModal({
      open: true,
      prDetail: makePrDetail({ reviews, checkSuites }),
      merging: false,
      onClose: vi.fn(),
      onConfirm: vi.fn(),
    });

    expect(rendered.container.querySelector(".modal-overlay")).toBeTruthy();
    rendered.unmount();
  });
});
