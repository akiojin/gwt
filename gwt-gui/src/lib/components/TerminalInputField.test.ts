import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import TerminalInputField from "./TerminalInputField.svelte";

// Mock Tauri invoke
vi.mock("$lib/tauriInvoke", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
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
    const buttons = container.querySelectorAll("button.action-btn");
    expect(buttons.length).toBeGreaterThanOrEqual(2);
  });

  it("renders attach button", () => {
    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "claude", active: true },
    });
    const attachBtn = container.querySelector('button[title="Attach image"]');
    expect(attachBtn).toBeTruthy();
  });

  it("disables send button when input is empty", () => {
    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "claude", active: true },
    });
    const sendBtn = container.querySelector('button[title="Send (Ctrl+Enter)"]');
    expect(sendBtn).toBeTruthy();
    expect((sendBtn as HTMLButtonElement).disabled).toBe(true);
  });

  it("does not show image thumbnails when no images attached", () => {
    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "claude", active: true },
    });
    const thumbnails = container.querySelector(".image-thumbnails");
    expect(thumbnails).toBeNull();
  });

  it("has correct structure for terminal input field", () => {
    const { container } = render(TerminalInputField, {
      props: { paneId: "test-pane", agentId: "claude", active: true },
    });
    const field = container.querySelector(".terminal-input-field");
    expect(field).toBeTruthy();
    const inputRow = container.querySelector(".input-row");
    expect(inputRow).toBeTruthy();
  });
});
