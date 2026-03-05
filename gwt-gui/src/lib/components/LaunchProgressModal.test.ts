import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";

async function renderModal(props: any) {
  const { default: LaunchProgressModal } = await import(
    "./LaunchProgressModal.svelte"
  );
  return render(LaunchProgressModal, { props });
}

describe("LaunchProgressModal", () => {
  beforeEach(() => {
    cleanup();
  });

  it("renders step markers in running state", async () => {
    const { container } = await renderModal({
      open: true,
      step: "validate",
      detail: "",
      status: "running",
      error: null,
      onCancel: vi.fn(),
      onClose: vi.fn(),
    });

    const marks = container.querySelectorAll(".mark");
    expect(marks.length).toBe(7);
    // "fetch" (idx 0) is before "validate" (idx 1) -> [x]
    expect(marks[0].textContent).toBe("[x]");
    // "validate" (idx 1) is the current step -> [>]
    expect(marks[1].textContent).toBe("[>]");
    // "paths" (idx 2) is after current -> [ ]
    expect(marks[2].textContent).toBe("[ ]");
  });

  it("shows error message in error state", async () => {
    const { container } = await renderModal({
      open: true,
      step: "create",
      detail: "",
      status: "error",
      error: "Worktree already exists.",
      onCancel: vi.fn(),
      onClose: vi.fn(),
    });

    const errorEl = container.querySelector(".error");
    expect(errorEl).not.toBeNull();
    expect(errorEl!.textContent).toBe("Worktree already exists.");

    // Close button should appear (not Cancel) in error state
    const closeBtn = container.querySelector("button.primary");
    expect(closeBtn).not.toBeNull();
    expect(closeBtn!.textContent).toContain("Close");
  });

  it("calls onCancel when Cancel button is clicked", async () => {
    const onCancel = vi.fn();
    const { container } = await renderModal({
      open: true,
      step: "fetch",
      detail: "",
      status: "running",
      error: null,
      onCancel,
      onClose: vi.fn(),
    });

    const cancelBtn = container.querySelector("button.secondary");
    expect(cancelBtn).not.toBeNull();
    await fireEvent.click(cancelBtn!);
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("calls onCancel on Escape key while running", async () => {
    const onCancel = vi.fn();
    const { container } = await renderModal({
      open: true,
      step: "fetch",
      detail: "",
      status: "running",
      error: null,
      onCancel,
      onClose: vi.fn(),
    });

    const overlay = container.querySelector(".overlay");
    expect(overlay).not.toBeNull();
    await fireEvent.keyDown(overlay!, { key: "Escape" });
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("calls onClose on Escape key when not running", async () => {
    const onCancel = vi.fn();
    const onClose = vi.fn();
    const { container } = await renderModal({
      open: true,
      step: "create",
      detail: "",
      status: "error",
      error: "Launch failed.",
      onCancel,
      onClose,
    });

    const overlay = container.querySelector(".overlay");
    expect(overlay).not.toBeNull();
    await fireEvent.keyDown(overlay!, { key: "Escape" });
    expect(onClose).toHaveBeenCalledOnce();
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("renders nothing when open is false", async () => {
    const { container } = await renderModal({
      open: false,
      step: "fetch",
      detail: "",
      status: "running",
      error: null,
      onCancel: vi.fn(),
      onClose: vi.fn(),
    });

    expect(container.querySelector(".overlay")).toBeNull();
    expect(container.querySelector(".dialog")).toBeNull();
  });

  it("shows detail text when running", async () => {
    const { container } = await renderModal({
      open: true,
      step: "deps",
      detail: "Installing packages...",
      status: "running",
      error: null,
      onCancel: vi.fn(),
      onClose: vi.fn(),
    });

    const detailEl = container.querySelector(".detail");
    expect(detailEl).not.toBeNull();
    expect(detailEl!.textContent).toBe("Installing packages...");
  });

  it("shows 'Use Existing Branch' button on E1004 error when onUseExisting is provided", async () => {
    const onUseExisting = vi.fn();
    const { container } = await renderModal({
      open: true,
      step: "create",
      detail: "",
      status: "error",
      error: "[E1004] Branch already exists: feature/refactoring",
      onCancel: vi.fn(),
      onClose: vi.fn(),
      onUseExisting,
    });

    const buttons = container.querySelectorAll("button");
    const useExistingBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Use Existing Branch"),
    );
    expect(useExistingBtn).not.toBeUndefined();
    expect(useExistingBtn!.classList.contains("primary")).toBe(true);

    // Close button should be secondary
    const closeBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Close"),
    );
    expect(closeBtn).not.toBeUndefined();
    expect(closeBtn!.classList.contains("secondary")).toBe(true);
  });

  it("does not show 'Use Existing Branch' button on non-E1004 errors", async () => {
    const onUseExisting = vi.fn();
    const { container } = await renderModal({
      open: true,
      step: "validate",
      detail: "",
      status: "error",
      error: "[E1001] Repository not found",
      onCancel: vi.fn(),
      onClose: vi.fn(),
      onUseExisting,
    });

    const buttons = container.querySelectorAll("button");
    const useExistingBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Use Existing Branch"),
    );
    expect(useExistingBtn).toBeUndefined();
  });

  it("calls onUseExisting when 'Use Existing Branch' button is clicked", async () => {
    const onUseExisting = vi.fn();
    const { container } = await renderModal({
      open: true,
      step: "create",
      detail: "",
      status: "error",
      error: "[E1004] Branch already exists: feature/refactoring",
      onCancel: vi.fn(),
      onClose: vi.fn(),
      onUseExisting,
    });

    const buttons = container.querySelectorAll("button");
    const useExistingBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Use Existing Branch"),
    );
    expect(useExistingBtn).not.toBeUndefined();
    await fireEvent.click(useExistingBtn!);
    expect(onUseExisting).toHaveBeenCalledOnce();
  });

  it("does not show 'Use Existing Branch' button on E1004 when onUseExisting is not provided", async () => {
    const { container } = await renderModal({
      open: true,
      step: "create",
      detail: "",
      status: "error",
      error: "[E1004] Branch already exists: feature/refactoring",
      onCancel: vi.fn(),
      onClose: vi.fn(),
    });

    const buttons = container.querySelectorAll("button");
    const useExistingBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Use Existing Branch"),
    );
    expect(useExistingBtn).toBeUndefined();
  });

  it("includes 'Registering skills' step in step list", async () => {
    const { container } = await renderModal({
      open: true,
      step: "skills",
      detail: "",
      status: "running",
      error: null,
      onCancel: vi.fn(),
      onClose: vi.fn(),
    });

    const texts = Array.from(container.querySelectorAll(".text")).map(
      (el) => el.textContent,
    );
    expect(texts).toContain("Registering skills");

    const marks = container.querySelectorAll(".mark");
    // skills is at index 5 (after create at 4), so steps 0-4 should be [x] and 5 should be [>]
    expect(marks[4].textContent).toBe("[x]");
    expect(marks[5].textContent).toBe("[>]");
    expect(marks[6].textContent).toBe("[ ]");
  });
});
