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
});
