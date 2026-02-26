import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();
const listenMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

async function renderModal(props: any) {
  const { default: MigrationModal } = await import("./MigrationModal.svelte");
  return render(MigrationModal, { props });
}

describe("MigrationModal", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
    listenMock.mockResolvedValue(() => {});
    cleanup();
  });

  afterEach(() => {
    cleanup();
  });

  it("does not render when open is false", async () => {
    const rendered = await renderModal({
      open: false,
      sourceRoot: "/tmp/repo",
      onCompleted: vi.fn(),
    });

    expect(rendered.queryByText("Migration Required")).toBeNull();
  });

  it("renders dialog when open is true", async () => {
    const rendered = await renderModal({
      open: true,
      sourceRoot: "/tmp/repo",
      onCompleted: vi.fn(),
    });

    expect(rendered.getByText("Migration Required")).toBeTruthy();
    expect(rendered.getByText("/tmp/repo")).toBeTruthy();
  });

  it("shows all migration steps", async () => {
    const rendered = await renderModal({
      open: true,
      sourceRoot: "/tmp/repo",
      onCompleted: vi.fn(),
    });

    expect(rendered.getByText("Validating prerequisites")).toBeTruthy();
    expect(rendered.getByText("Creating backup")).toBeTruthy();
    expect(rendered.getByText("Creating bare repository")).toBeTruthy();
    expect(rendered.getByText("Migrating worktrees")).toBeTruthy();
    expect(rendered.getByText("Cleaning up")).toBeTruthy();
    expect(rendered.getByText("Completed")).toBeTruthy();
  });

  it("shows Migrate and Quit buttons", async () => {
    const rendered = await renderModal({
      open: true,
      sourceRoot: "/tmp/repo",
      onCompleted: vi.fn(),
    });

    expect(rendered.getByText("Migrate")).toBeTruthy();
    expect(rendered.getByText("Quit")).toBeTruthy();
  });

  it("calls start_migration_job on Migrate click", async () => {
    invokeMock.mockResolvedValue("job-123");

    const rendered = await renderModal({
      open: true,
      sourceRoot: "/tmp/repo",
      onCompleted: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Migrate"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("start_migration_job", { path: "/tmp/repo" });
    });
  });

  it("shows Migrating... while running", async () => {
    invokeMock.mockImplementation(() => new Promise(() => {})); // never resolves

    const rendered = await renderModal({
      open: true,
      sourceRoot: "/tmp/repo",
      onCompleted: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Migrate"));

    await waitFor(() => {
      expect(rendered.getByText("Migrating...")).toBeTruthy();
    });
  });

  it("disables Quit button while running", async () => {
    invokeMock.mockImplementation(() => new Promise(() => {}));

    const rendered = await renderModal({
      open: true,
      sourceRoot: "/tmp/repo",
      onCompleted: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Migrate"));

    await waitFor(() => {
      const quitBtn = rendered.getByText("Quit") as HTMLButtonElement;
      expect(quitBtn.disabled).toBe(true);
    });
  });

  it("shows error when start_migration_job fails", async () => {
    invokeMock.mockRejectedValue(new Error("permission denied"));

    const rendered = await renderModal({
      open: true,
      sourceRoot: "/tmp/repo",
      onCompleted: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Migrate"));

    await waitFor(() => {
      expect(rendered.getByText("Failed to start migration.")).toBeTruthy();
    });
  });

  it("shows Retry Migration button after error", async () => {
    invokeMock.mockRejectedValue(new Error("fail"));

    const rendered = await renderModal({
      open: true,
      sourceRoot: "/tmp/repo",
      onCompleted: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Migrate"));

    await waitFor(() => {
      expect(rendered.getByText("Retry Migration")).toBeTruthy();
    });
  });

  it("shows error when sourceRoot is empty", async () => {
    const rendered = await renderModal({
      open: true,
      sourceRoot: "  ",
      onCompleted: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Migrate"));

    await waitFor(() => {
      expect(rendered.getByText("Repository path is required.")).toBeTruthy();
    });
  });

  it("calls quit_app on Quit click", async () => {
    invokeMock.mockResolvedValue(undefined);

    const rendered = await renderModal({
      open: true,
      sourceRoot: "/tmp/repo",
      onCompleted: vi.fn(),
    });

    await fireEvent.click(rendered.getByText("Quit"));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("quit_app");
    });
  });

  it("calls onDismiss when quit_app fails", async () => {
    invokeMock.mockRejectedValue(new Error("not in tauri"));
    const onDismiss = vi.fn();

    const rendered = await renderModal({
      open: true,
      sourceRoot: "/tmp/repo",
      onCompleted: vi.fn(),
      onDismiss,
    });

    await fireEvent.click(rendered.getByText("Quit"));

    await waitFor(() => {
      expect(onDismiss).toHaveBeenCalledTimes(1);
    });
  });

  it("has proper aria attributes for accessibility", async () => {
    const rendered = await renderModal({
      open: true,
      sourceRoot: "/tmp/repo",
      onCompleted: vi.fn(),
    });

    const dialog = rendered.container.querySelector('[role="dialog"]');
    expect(dialog).toBeTruthy();
    expect(dialog?.getAttribute("aria-modal")).toBe("true");
    expect(dialog?.getAttribute("aria-label")).toBe("Migration Required");
  });
});
