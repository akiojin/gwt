import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

async function renderDialog(props: any) {
  const { default: MergeDialog } = await import("./MergeDialog.svelte");
  return render(MergeDialog, { props });
}

describe("MergeDialog", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    cleanup();
  });

  afterEach(() => {
    cleanup();
  });

  it("renders dialog with PR number and title", async () => {
    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 42,
      prTitle: "Add login feature",
      onClose: vi.fn(),
      onMerged: vi.fn(),
    });

    expect(rendered.getByText("Merge PR #42")).toBeTruthy();
    expect(rendered.getByText("Add login feature")).toBeTruthy();
  });

  it("shows three merge method radio options", async () => {
    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onMerged: vi.fn(),
    });

    expect(rendered.getByText("Merge commit")).toBeTruthy();
    expect(rendered.getByText("Squash and merge")).toBeTruthy();
    expect(rendered.getByText("Rebase and merge")).toBeTruthy();
  });

  it("has squash selected by default", async () => {
    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onMerged: vi.fn(),
    });

    const radios = rendered.container.querySelectorAll('input[type="radio"]') as NodeListOf<HTMLInputElement>;
    const squashRadio = Array.from(radios).find((r) => r.value === "squash");
    expect(squashRadio?.checked).toBe(true);
  });

  it("has delete branch checkbox checked by default", async () => {
    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onMerged: vi.fn(),
    });

    const checkbox = rendered.container.querySelector('input[type="checkbox"]') as HTMLInputElement;
    expect(checkbox.checked).toBe(true);
  });

  it("shows commit message textarea pre-filled with PR title", async () => {
    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 5,
      prTitle: "My PR Title",
      onClose: vi.fn(),
      onMerged: vi.fn(),
    });

    const textarea = rendered.container.querySelector("textarea") as HTMLTextAreaElement;
    expect(textarea.value).toBe("My PR Title");
  });

  it("calls onClose when Cancel button is clicked", async () => {
    const onClose = vi.fn();
    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose,
      onMerged: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Cancel"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("calls onClose when backdrop is clicked", async () => {
    const onClose = vi.fn();
    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose,
      onMerged: vi.fn(),
    });

    const backdrop = rendered.container.querySelector(".dialog-backdrop") as HTMLElement;
    await fireEvent.click(backdrop);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("calls merge_pr invoke and onMerged on success", async () => {
    invokeMock.mockResolvedValue("merged");
    const onMerged = vi.fn();

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 42,
      prTitle: "Test PR",
      onClose: vi.fn(),
      onMerged,
    });

    await fireEvent.click(rendered.getByText("Merge"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("merge_pr", {
        projectPath: "/tmp/project",
        prNumber: 42,
        method: "squash",
        deleteBranch: true,
        commitMsg: "Test PR",
      });
      expect(onMerged).toHaveBeenCalledTimes(1);
    });
  });

  it("shows Merging... text while merging", async () => {
    invokeMock.mockImplementation(() => new Promise(() => {})); // never resolves

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onMerged: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Merge"));

    await waitFor(() => {
      expect(rendered.getByText("Merging...")).toBeTruthy();
    });
  });

  it("displays error message on merge failure", async () => {
    invokeMock.mockRejectedValue(new Error("Branch conflict"));

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onMerged: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Merge"));

    await waitFor(() => {
      expect(rendered.getByText("Branch conflict")).toBeTruthy();
    });
  });

  it("allows switching merge method to rebase", async () => {
    invokeMock.mockResolvedValue("merged");
    const onMerged = vi.fn();

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onMerged,
    });

    const radios = rendered.container.querySelectorAll('input[type="radio"]') as NodeListOf<HTMLInputElement>;
    const rebaseRadio = Array.from(radios).find((r) => r.value === "rebase");
    await fireEvent.click(rebaseRadio!);

    await fireEvent.click(rendered.getByText("Merge"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("merge_pr", expect.objectContaining({
        method: "rebase",
      }));
    });
  });

  it("disables Cancel button while merging", async () => {
    invokeMock.mockImplementation(() => new Promise(() => {}));

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onMerged: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Merge"));

    await waitFor(() => {
      const cancelBtn = rendered.getByText("Cancel") as HTMLButtonElement;
      expect(cancelBtn.disabled).toBe(true);
    });
  });

  it("handles string error in toErrorMessage", async () => {
    invokeMock.mockRejectedValue("raw string error");

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onMerged: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Merge"));

    await waitFor(() => {
      expect(rendered.getByText("raw string error")).toBeTruthy();
    });
  });

  it("handles non-standard error object in toErrorMessage", async () => {
    invokeMock.mockRejectedValue({ code: 500 });

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onMerged: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Merge"));

    await waitFor(() => {
      const errorEl = rendered.container.querySelector(".dialog-error");
      expect(errorEl).toBeTruthy();
    });
  });

  it("sends undefined commitMsg when message is whitespace only", async () => {
    invokeMock.mockResolvedValue("merged");
    const onMerged = vi.fn();

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onMerged,
    });

    const textarea = rendered.container.querySelector("textarea") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "   " } });

    await fireEvent.click(rendered.getByText("Merge"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("merge_pr", expect.objectContaining({
        commitMsg: undefined,
      }));
    });
  });

  it("does not call onClose when clicking inside the dialog (not backdrop)", async () => {
    const onClose = vi.fn();
    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose,
      onMerged: vi.fn(),
    });

    const dialog = rendered.container.querySelector(".dialog") as HTMLElement;
    await fireEvent.click(dialog);
    expect(onClose).not.toHaveBeenCalled();
  });
});
