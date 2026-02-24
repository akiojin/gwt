import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, waitFor, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();
vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
}));

const collectSystemInfoMock = vi.fn();
const collectRecentLogsMock = vi.fn();
vi.mock("$lib/diagnostics", () => ({
  collectSystemInfo: (...args: unknown[]) => collectSystemInfoMock(...args),
  collectRecentLogs: (...args: unknown[]) => collectRecentLogsMock(...args),
}));

const collectScreenTextMock = vi.fn();
vi.mock("$lib/screenCapture", () => ({
  collectScreenText: (...args: unknown[]) => collectScreenTextMock(...args),
}));

const openExternalUrlMock = vi.fn();
vi.mock("$lib/openExternalUrl", () => ({
  openExternalUrl: (...args: unknown[]) => openExternalUrlMock(...args),
}));

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function renderReportDialog(props: any) {
  const { default: ReportDialog } = await import("./ReportDialog.svelte");
  return render(ReportDialog, { props });
}

const sampleError = {
  severity: "error" as const,
  code: "E1001",
  message: "something went wrong",
  command: "open_project",
  category: "Git",
  suggestions: ["Try again", "Check path"],
  timestamp: "2026-01-01T00:00:00.000Z",
};

describe("ReportDialog", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
    collectSystemInfoMock.mockReset();
    collectRecentLogsMock.mockReset();
    collectScreenTextMock.mockReset();
    openExternalUrlMock.mockReset();

    collectSystemInfoMock.mockResolvedValue("macOS 15.2, M4 Pro");
    collectRecentLogsMock.mockResolvedValue("LOG: test entry");
    collectScreenTextMock.mockReturnValue("=== GWT Screen Capture ===");
  });

  it("renders Bug Report tab by default when mode is bug", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose: vi.fn(),
    });

    expect(rendered.getByText("Bug Report")).toBeTruthy();
    expect(rendered.getByText("Feature Request")).toBeTruthy();
    expect(rendered.getByLabelText("Title")).toBeTruthy();
    expect(rendered.getByLabelText("Steps to Reproduce")).toBeTruthy();
    expect(rendered.getByLabelText("Expected Result")).toBeTruthy();
    expect(rendered.getByLabelText("Actual Result")).toBeTruthy();
  });

  it("renders Feature Request tab when mode is feature", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "feature",
      onclose: vi.fn(),
    });

    // Feature tab should be active
    expect(rendered.getByLabelText("Description")).toBeTruthy();
    expect(rendered.getByLabelText("Use Case")).toBeTruthy();
    expect(rendered.getByLabelText("Expected Benefit")).toBeTruthy();
  });

  it("switches tabs when clicking tab buttons", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose: vi.fn(),
    });

    // Bug tab visible initially
    expect(rendered.getByLabelText("Steps to Reproduce")).toBeTruthy();

    // Switch to Feature Request
    await fireEvent.click(rendered.getByText("Feature Request"));
    expect(rendered.getByLabelText("Description")).toBeTruthy();

    // Switch back to Bug Report
    await fireEvent.click(rendered.getByText("Bug Report"));
    expect(rendered.getByLabelText("Steps to Reproduce")).toBeTruthy();
  });

  it("keeps dialog visible and switches tab when mode changes while open", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });

    const onclose = vi.fn();
    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose,
    });

    expect(rendered.getByText("Bug Report")).toBeTruthy();

    await rendered.rerender({
      open: true,
      mode: "feature",
      onclose,
    });

    await waitFor(() => {
      expect(rendered.getByLabelText("Description")).toBeTruthy();
    });
  });

  it("shows error details section when prefillError is provided", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      prefillError: sampleError,
      onclose: vi.fn(),
    });

    // Error Details collapsible header should be present
    const header = rendered.getByText("Error Details");
    expect(header).toBeTruthy();

    // Expand error details
    await fireEvent.click(header);
    expect(rendered.getAllByText("E1001").length).toBeGreaterThanOrEqual(1);
    expect(rendered.getByText("open_project")).toBeTruthy();
  });

  it("shows validation message when submitting with empty title", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_report_target") {
        return { owner: "akiojin", repo: "gwt", display: "akiojin/gwt" };
      }
      return {};
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Submit"));

    await waitFor(() => {
      expect(rendered.getByText("Please enter a title.")).toBeTruthy();
    });
  });

  it("submits bug report via create_github_issue and calls onsuccess", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_report_target") {
        return { owner: "akiojin", repo: "gwt", display: "akiojin/gwt" };
      }
      if (command === "create_github_issue") {
        return { url: "https://github.com/akiojin/gwt/issues/42", number: 42 };
      }
      return {};
    });

    const onsuccess = vi.fn();
    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose: vi.fn(),
      onsuccess,
    });

    const titleInput = rendered.getByLabelText("Title") as HTMLInputElement;
    await fireEvent.input(titleInput, { target: { value: "Test bug" } });

    await fireEvent.click(rendered.getByText("Submit"));

    await waitFor(() => {
      expect(onsuccess).toHaveBeenCalledTimes(1);
      expect(onsuccess).toHaveBeenCalledWith({
        url: "https://github.com/akiojin/gwt/issues/42",
        number: 42,
      });
    });

    expect(invokeMock).toHaveBeenCalledWith(
      "create_github_issue",
      expect.objectContaining({
        owner: "akiojin",
        repo: "gwt",
        title: "Test bug",
        labels: ["bug"],
      }),
    );
  });

  it("shows fallback actions on submit failure", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_report_target") {
        return { owner: "akiojin", repo: "gwt", display: "akiojin/gwt" };
      }
      if (command === "create_github_issue") {
        throw new Error("gh not found");
      }
      return {};
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose: vi.fn(),
    });

    const titleInput = rendered.getByLabelText("Title") as HTMLInputElement;
    await fireEvent.input(titleInput, { target: { value: "Failing bug" } });
    await fireEvent.click(rendered.getByText("Submit"));

    await waitFor(() => {
      expect(rendered.getByText("Copy to Clipboard")).toBeTruthy();
      expect(rendered.getByText("Open in Browser")).toBeTruthy();
    });
  });

  it("opens browser URL on Open in Browser click", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_report_target") {
        return { owner: "akiojin", repo: "gwt", display: "akiojin/gwt" };
      }
      if (command === "create_github_issue") {
        throw new Error("gh not found");
      }
      return {};
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose: vi.fn(),
    });

    const titleInput = rendered.getByLabelText("Title") as HTMLInputElement;
    await fireEvent.input(titleInput, { target: { value: "Browser test" } });
    await fireEvent.click(rendered.getByText("Submit"));

    await waitFor(() => {
      expect(rendered.getByText("Open in Browser")).toBeTruthy();
    });

    await fireEvent.click(rendered.getByText("Open in Browser"));
    expect(openExternalUrlMock).toHaveBeenCalledTimes(1);
    const url = openExternalUrlMock.mock.calls[0][0] as string;
    expect(url).toContain("github.com/akiojin/gwt/issues/new");
  });

  it("toggles preview showing generated markdown", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose: vi.fn(),
    });

    const titleInput = rendered.getByLabelText("Title") as HTMLInputElement;
    await fireEvent.input(titleInput, { target: { value: "Preview test" } });

    // Show preview
    await fireEvent.click(rendered.getByText("Preview"));
    expect(rendered.getByText("Preview", { selector: "h3" })).toBeTruthy();

    // Hide preview
    await fireEvent.click(rendered.getByText("Hide Preview"));
    expect(rendered.queryByText("Preview", { selector: "h3" })).toBeNull();
  });

  it("calls onclose when Cancel is clicked", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });
    const onclose = vi.fn();

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose,
    });

    await fireEvent.click(rendered.getByText("Cancel"));
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  it("calls onclose when clicking backdrop", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });
    const onclose = vi.fn();

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose,
    });

    const overlay = rendered.container.querySelector(
      ".report-overlay",
    ) as HTMLDivElement;
    expect(overlay).toBeTruthy();
    await fireEvent.click(overlay);
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  it("calls onclose on Escape key", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });
    const onclose = vi.fn();

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose,
    });

    const overlay = rendered.container.querySelector(
      ".report-overlay",
    ) as HTMLDivElement;
    expect(overlay).toBeTruthy();
    await fireEvent.keyDown(overlay, { key: "Escape" });
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  it("detects working repo and adds to target dropdown", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_report_target") {
        return { owner: "myorg", repo: "myapp", display: "myorg/myapp" };
      }
      return {};
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      projectPath: "/tmp/project",
      onclose: vi.fn(),
    });

    await waitFor(() => {
      const select = rendered.getByLabelText("Repository") as HTMLSelectElement;
      const options = select.querySelectorAll("option");
      expect(options.length).toBe(2);
      expect(options[0].textContent).toBe("akiojin/gwt");
      expect(options[1].textContent).toBe("myorg/myapp");
    });
  });

  it("shows only akiojin/gwt when detected repo matches default", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      projectPath: "/tmp/project",
      onclose: vi.fn(),
    });

    await waitFor(() => {
      const select = rendered.getByLabelText("Repository") as HTMLSelectElement;
      const options = select.querySelectorAll("option");
      expect(options.length).toBe(1);
      expect(options[0].textContent).toBe("akiojin/gwt");
    });
  });

  it("captures terminal text when Capture Terminal Text is clicked", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_report_target") {
        return { owner: "akiojin", repo: "gwt", display: "akiojin/gwt" };
      }
      if (command === "capture_screen_text") {
        return "Terminal output line 1\nTerminal output line 2";
      }
      return {};
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Capture Terminal Text"));

    await waitFor(() => {
      expect(rendered.getByText("Recapture Terminal Text")).toBeTruthy();
    });
  });

  it("submits feature request with enhancement label and calls onsuccess", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "detect_report_target") {
        return { owner: "akiojin", repo: "gwt", display: "akiojin/gwt" };
      }
      if (command === "create_github_issue") {
        return { url: "https://github.com/akiojin/gwt/issues/99", number: 99 };
      }
      return {};
    });

    const onsuccess = vi.fn();
    const rendered = await renderReportDialog({
      open: true,
      mode: "feature",
      onclose: vi.fn(),
      onsuccess,
    });

    const titleInput = rendered.getByLabelText("Title") as HTMLInputElement;
    await fireEvent.input(titleInput, { target: { value: "New feature" } });
    await fireEvent.click(rendered.getByText("Submit"));

    await waitFor(() => {
      expect(onsuccess).toHaveBeenCalledTimes(1);
      expect(onsuccess).toHaveBeenCalledWith({
        url: "https://github.com/akiojin/gwt/issues/99",
        number: 99,
      });
    });

    expect(invokeMock).toHaveBeenCalledWith(
      "create_github_issue",
      expect.objectContaining({
        labels: ["enhancement"],
      }),
    );
  });

  it("shows diagnostic checkboxes in bug report tab", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose: vi.fn(),
    });

    expect(rendered.getByText("System Info")).toBeTruthy();
    expect(rendered.getByText("Application Logs")).toBeTruthy();
    expect(rendered.getByText("Screen Capture (text)")).toBeTruthy();
  });

  it("passes current branch and active tab into screen capture", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      screenCaptureBranch: "feature/error-reporting",
      screenCaptureActiveTab: "Terminal (feature/error-reporting)",
      onclose: vi.fn(),
    });

    const labels = rendered.container.querySelectorAll(".checkbox-label");
    const captureLabel = Array.from(labels).find((l) =>
      l.textContent?.includes("Screen Capture (text)"),
    );
    const captureCheckbox = captureLabel?.querySelector(
      "input[type='checkbox']",
    ) as HTMLInputElement;
    expect(captureCheckbox).toBeTruthy();

    await fireEvent.click(captureCheckbox);

    await waitFor(() => {
      expect(collectScreenTextMock).toHaveBeenCalledWith({
        branch: "feature/error-reporting",
        activeTab: "Terminal (feature/error-reporting)",
      });
    });
  });

  it("renders preview as editable textarea", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose: vi.fn(),
    });

    const titleInput = rendered.getByLabelText("Title") as HTMLInputElement;
    await fireEvent.input(titleInput, {
      target: { value: "Editable preview" },
    });
    await fireEvent.click(rendered.getByText("Preview"));

    const previewArea = rendered.container.querySelector(
      ".preview-content",
    ) as HTMLTextAreaElement;
    expect(previewArea).toBeTruthy();
    expect(previewArea.tagName).toBe("TEXTAREA");
    expect(previewArea.value).toContain("Bug Report");
  });

  it("disables Submit button when title is empty", async () => {
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose: vi.fn(),
    });

    const submitBtn = rendered.getByText("Submit") as HTMLButtonElement;
    expect(submitBtn.disabled).toBe(true);

    const titleInput = rendered.getByLabelText("Title") as HTMLInputElement;
    await fireEvent.input(titleInput, { target: { value: "Some title" } });
    expect(submitBtn.disabled).toBe(false);
  });

  it("disables Application Logs checkbox when no logs available", async () => {
    collectRecentLogsMock.mockResolvedValue("");
    invokeMock.mockResolvedValue({
      owner: "akiojin",
      repo: "gwt",
      display: "akiojin/gwt",
    });

    const rendered = await renderReportDialog({
      open: true,
      mode: "bug",
      onclose: vi.fn(),
    });

    // Click the Application Logs checkbox to trigger the effect
    const labels = rendered.container.querySelectorAll(".checkbox-label");
    const logsLabel = Array.from(labels).find((l) =>
      l.textContent?.includes("Application Logs"),
    );
    const logsCheckbox = logsLabel?.querySelector(
      "input[type='checkbox']",
    ) as HTMLInputElement;
    expect(logsCheckbox).toBeTruthy();
    await fireEvent.click(logsCheckbox);

    // Wait for the effect to detect empty logs and disable the checkbox
    await waitFor(() => {
      expect(logsLabel?.textContent).toContain("No logs available");
      const updatedCheckbox = logsLabel?.querySelector(
        "input[type='checkbox']",
      ) as HTMLInputElement;
      expect(updatedCheckbox.disabled).toBe(true);
    });
  });
});
