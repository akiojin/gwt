import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import TerminalInputField from "./TerminalInputField.svelte";

const mockInvoke = vi.fn().mockResolvedValue(undefined);

// Mock Tauri invoke
vi.mock("$lib/tauriInvoke", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// Mock dialog
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue(null),
}));

describe("TerminalInputField", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
  });

  it("renders textarea with placeholder", () => {
    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "claude", active: true },
    });
    const textarea = container.querySelector("textarea");
    expect(textarea).toBeTruthy();
    expect(textarea?.placeholder).toContain("Ctrl+Enter");
  });

  it("renders send and stop buttons", () => {
    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "claude", active: true },
    });
    const sendBtn = container.querySelector('button[title="Send (Ctrl+Enter)"]');
    const stopBtn = container.querySelector('button[title="Stop (Escape)"]');
    expect(sendBtn).toBeTruthy();
    expect(stopBtn).toBeTruthy();
  });

  it("renders voice and attach buttons", () => {
    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "claude", active: true },
    });
    const voiceBtn = container.querySelector('button[title="Voice input"]');
    const attachBtn = container.querySelector('button[title="Attach image"]');
    expect(voiceBtn).toBeTruthy();
    expect(attachBtn).toBeTruthy();
  });

  it("disables send button when input is empty", () => {
    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "claude", active: true },
    });
    const sendBtn = container.querySelector('button[title="Send (Ctrl+Enter)"]') as HTMLButtonElement;
    expect(sendBtn.disabled).toBe(true);
  });

  it("does not show image thumbnails when no images attached", () => {
    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "claude", active: true },
    });
    const thumbnails = container.querySelector(".image-thumbnails");
    expect(thumbnails).toBeNull();
  });

  it("sends text on Ctrl+Enter", async () => {
    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "claude", active: true },
    });
    const textarea = container.querySelector("textarea")!;

    // Type text
    await fireEvent.input(textarea, { target: { value: "hello" } });

    // Press Ctrl+Enter
    await fireEvent.keyDown(textarea, { key: "Enter", ctrlKey: true });

    // Allow async send to complete
    await vi.waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("write_terminal", {
        paneId: "test-pane",
        data: expect.arrayContaining([104, 101, 108, 108, 111]), // "hello" bytes
      });
    });
  });

  it("sends interrupt on Escape", async () => {
    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "claude", active: true },
    });
    const textarea = container.querySelector("textarea")!;

    await fireEvent.keyDown(textarea, { key: "Escape" });

    await vi.waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("write_terminal", {
        paneId: "test-pane",
        data: [0x1b], // ESC
      });
    });
  });

  it("does not send on Enter during IME composition", async () => {
    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "claude", active: true },
    });
    const textarea = container.querySelector("textarea")!;

    await fireEvent.input(textarea, { target: { value: "test" } });

    // Simulate IME composition
    await fireEvent.compositionStart(textarea);
    await fireEvent.keyDown(textarea, { key: "Enter", ctrlKey: true, isComposing: true });

    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("works with unknown agentId using default profile", async () => {
    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "some-unknown-agent", active: true },
    });
    const textarea = container.querySelector("textarea")!;

    await fireEvent.input(textarea, { target: { value: "hello" } });
    await fireEvent.keyDown(textarea, { key: "Escape" });

    await vi.waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("write_terminal", {
        paneId: "test-pane",
        data: [0x1b], // Default profile interrupt = ESC
      });
    });
  });

  it("calls onFocusTerminal on Tab key", async () => {
    const onFocusTerminal = vi.fn();
    const { container } = render(TerminalInputField, {
      props: {
        paneId: "test-pane",
        agentId: "claude",
        active: true,
        onFocusTerminal,
      },
    });
    const textarea = container.querySelector("textarea")!;

    await fireEvent.keyDown(textarea, { key: "Tab" });

    // Tab fallback should call onFocusTerminal
    await vi.waitFor(() => {
      expect(onFocusTerminal).toHaveBeenCalled();
    });
  });

  it("dispatches voice toggle event on mic button click", async () => {
    const handler = vi.fn();
    window.addEventListener("gwt-voice-toggle", handler);

    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "claude", active: true },
    });
    const voiceBtn = container.querySelector('button[title="Voice input"]')!;

    await fireEvent.click(voiceBtn);
    expect(handler).toHaveBeenCalled();

    window.removeEventListener("gwt-voice-toggle", handler);
  });
});
