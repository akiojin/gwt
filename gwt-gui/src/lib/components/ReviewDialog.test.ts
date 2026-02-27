import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

async function renderDialog(props: any) {
  const { default: ReviewDialog } = await import("./ReviewDialog.svelte");
  return render(ReviewDialog, { props });
}

describe("ReviewDialog", () => {
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
      prNumber: 99,
      prTitle: "Fix spacing",
      onClose: vi.fn(),
      onReviewed: vi.fn(),
    });

    expect(rendered.getByText("Review PR #99")).toBeTruthy();
    expect(rendered.getByText("Fix spacing")).toBeTruthy();
  });

  it("shows three review action radio options", async () => {
    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onReviewed: vi.fn(),
    });

    const radios = rendered.container.querySelectorAll('input[type="radio"]') as NodeListOf<HTMLInputElement>;
    expect(radios).toHaveLength(3);
    expect(radios[0].value).toBe("approve");
    expect(radios[1].value).toBe("request-changes");
    expect(radios[2].value).toBe("comment");
  });

  it("has approve selected by default", async () => {
    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onReviewed: vi.fn(),
    });

    const radios = rendered.container.querySelectorAll('input[type="radio"]') as NodeListOf<HTMLInputElement>;
    const approveRadio = Array.from(radios).find((r) => r.value === "approve");
    expect(approveRadio?.checked).toBe(true);
  });

  it("shows comment textarea with placeholder", async () => {
    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onReviewed: vi.fn(),
    });

    const textarea = rendered.container.querySelector("textarea") as HTMLTextAreaElement;
    expect(textarea).toBeTruthy();
    expect(textarea.placeholder).toBe("Leave a comment...");
  });

  it("calls onClose when Cancel button is clicked", async () => {
    const onClose = vi.fn();
    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose,
      onReviewed: vi.fn(),
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
      onReviewed: vi.fn(),
    });

    const backdrop = rendered.container.querySelector(".dialog-backdrop") as HTMLElement;
    await fireEvent.click(backdrop);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("calls review_pr invoke and onReviewed on success", async () => {
    invokeMock.mockResolvedValue("reviewed");
    const onReviewed = vi.fn();

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 42,
      prTitle: "Test PR",
      onClose: vi.fn(),
      onReviewed,
    });

    await fireEvent.click(rendered.getByText("Submit Review"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("review_pr", {
        projectPath: "/tmp/project",
        prNumber: 42,
        action: "approve",
        body: undefined,
      });
      expect(onReviewed).toHaveBeenCalledTimes(1);
    });
  });

  it("sends body text with review", async () => {
    invokeMock.mockResolvedValue("reviewed");
    const onReviewed = vi.fn();

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 10,
      prTitle: "Test",
      onClose: vi.fn(),
      onReviewed,
    });

    const textarea = rendered.container.querySelector("textarea") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "LGTM!" } });

    await fireEvent.click(rendered.getByText("Submit Review"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("review_pr", expect.objectContaining({
        body: "LGTM!",
      }));
    });
  });

  it("shows Submitting... text while submitting", async () => {
    invokeMock.mockImplementation(() => new Promise(() => {}));

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onReviewed: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Submit Review"));

    await waitFor(() => {
      expect(rendered.getByText("Submitting...")).toBeTruthy();
    });
  });

  it("displays error message on submit failure", async () => {
    invokeMock.mockRejectedValue(new Error("Permission denied"));

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onReviewed: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Submit Review"));

    await waitFor(() => {
      expect(rendered.getByText("Permission denied")).toBeTruthy();
    });
  });

  it("allows switching to request-changes action", async () => {
    invokeMock.mockResolvedValue("reviewed");

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onReviewed: vi.fn(),
    });

    const radios = rendered.container.querySelectorAll('input[type="radio"]') as NodeListOf<HTMLInputElement>;
    const requestChangesRadio = Array.from(radios).find((r) => r.value === "request-changes");
    await fireEvent.click(requestChangesRadio!);

    await fireEvent.click(rendered.getByText("Submit Review"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("review_pr", expect.objectContaining({
        action: "request-changes",
      }));
    });
  });

  it("disables Cancel button while submitting", async () => {
    invokeMock.mockImplementation(() => new Promise(() => {}));

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onReviewed: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Submit Review"));

    await waitFor(() => {
      const cancelBtn = rendered.getByText("Cancel") as HTMLButtonElement;
      expect(cancelBtn.disabled).toBe(true);
    });
  });

  it("handles string error in toErrorMessage", async () => {
    invokeMock.mockRejectedValue("plain string error");

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onReviewed: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Submit Review"));

    await waitFor(() => {
      expect(rendered.getByText("plain string error")).toBeTruthy();
    });
  });

  it("handles non-standard error object in toErrorMessage", async () => {
    invokeMock.mockRejectedValue({ code: 42 });

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onReviewed: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Submit Review"));

    await waitFor(() => {
      const errorEl = rendered.container.querySelector(".dialog-error");
      expect(errorEl).toBeTruthy();
    });
  });

  it("sends undefined body when body is whitespace only", async () => {
    invokeMock.mockResolvedValue("reviewed");
    const onReviewed = vi.fn();

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 10,
      prTitle: "Test",
      onClose: vi.fn(),
      onReviewed,
    });

    const textarea = rendered.container.querySelector("textarea") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "   " } });

    await fireEvent.click(rendered.getByText("Submit Review"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("review_pr", expect.objectContaining({
        body: undefined,
      }));
    });
  });

  it("handles error object with non-string message in toErrorMessage", async () => {
    invokeMock.mockRejectedValue({ message: 42 });

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onReviewed: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Submit Review"));

    await waitFor(() => {
      const errorEl = rendered.container.querySelector(".dialog-error");
      expect(errorEl).toBeTruthy();
      // Falls through to String(err) since msg is not a string
      expect(errorEl?.textContent).toContain("[object Object]");
    });
  });

  it("allows switching to comment action", async () => {
    invokeMock.mockResolvedValue("reviewed");

    const rendered = await renderDialog({
      projectPath: "/tmp/project",
      prNumber: 1,
      prTitle: "Test",
      onClose: vi.fn(),
      onReviewed: vi.fn(),
    });

    const radios = rendered.container.querySelectorAll('input[type="radio"]') as NodeListOf<HTMLInputElement>;
    const commentRadio = Array.from(radios).find((r) => r.value === "comment");
    await fireEvent.click(commentRadio!);

    await fireEvent.click(rendered.getByText("Submit Review"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("review_pr", expect.objectContaining({
        action: "comment",
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
      onReviewed: vi.fn(),
    });

    const dialog = rendered.container.querySelector(".dialog") as HTMLElement;
    await fireEvent.click(dialog);
    expect(onClose).not.toHaveBeenCalled();
  });
});
