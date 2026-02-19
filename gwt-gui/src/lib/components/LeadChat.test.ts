import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, cleanup } from "@testing-library/svelte";
import type { LeadMessage } from "../types";

function makeMsg(
  overrides: Partial<LeadMessage> & { content: string }
): LeadMessage {
  return {
    role: "assistant",
    kind: "message",
    timestamp: Date.now(),
    ...overrides,
  };
}

async function renderChat(props: {
  messages?: LeadMessage[];
  isWaiting?: boolean;
  onSend?: (text: string) => void;
}) {
  const { default: LeadChat } = await import("./LeadChat.svelte");
  return render(LeadChat, {
    props: {
      messages: props.messages ?? [],
      isWaiting: props.isWaiting ?? false,
      onSend: props.onSend ?? vi.fn(),
    },
  });
}

describe("LeadChat", () => {
  beforeEach(() => {
    cleanup();
  });

  it("shows empty state placeholder when no messages", async () => {
    const rendered = await renderChat({});
    expect(rendered.getByText("Start a conversation...")).toBeTruthy();
  });

  it("renders user messages right-aligned", async () => {
    const rendered = await renderChat({
      messages: [makeMsg({ role: "user", content: "Hello lead" })],
    });
    expect(rendered.getByText("Hello lead")).toBeTruthy();
    const bubble = rendered.container.querySelector(
      ".lead-message.user"
    );
    expect(bubble).toBeTruthy();
  });

  it("renders assistant messages left-aligned", async () => {
    const rendered = await renderChat({
      messages: [makeMsg({ role: "assistant", content: "I will help" })],
    });
    expect(rendered.getByText("I will help")).toBeTruthy();
    const bubble = rendered.container.querySelector(
      ".lead-message.assistant"
    );
    expect(bubble).toBeTruthy();
  });

  it("shows kind badges for thought, action, observation, error, progress", async () => {
    const kinds = [
      "thought",
      "action",
      "observation",
      "error",
      "progress",
    ] as const;
    const messages = kinds.map((kind, i) =>
      makeMsg({ kind, content: `msg-${kind}`, timestamp: Date.now() + i })
    );
    const rendered = await renderChat({ messages });

    for (const kind of kinds) {
      expect(rendered.getByText(`msg-${kind}`)).toBeTruthy();
      const badge = rendered.container.querySelector(
        `.lead-message.${kind} .lead-kind-badge`
      );
      expect(badge).toBeTruthy();
      expect(badge!.textContent?.trim().toLowerCase()).toContain(kind);
    }
  });

  it("hides kind badge for plain assistant messages", async () => {
    const rendered = await renderChat({
      messages: [makeMsg({ role: "assistant", kind: "message", content: "plain" })],
    });
    const badge = rendered.container.querySelector(
      ".lead-message.assistant .lead-kind-badge"
    );
    // For kind=message, the badge should either not exist or be hidden
    const msgEl = rendered.container.querySelector(".lead-message.assistant");
    expect(msgEl).toBeTruthy();
    // The badge should not be visible for kind=message on assistant role
    if (badge) {
      expect(badge.textContent?.trim()).toBe("");
    }
  });

  it("renders input area with placeholder", async () => {
    const rendered = await renderChat({});
    const textarea = rendered.getByPlaceholderText(
      "Type a message and press Enter..."
    ) as HTMLTextAreaElement;
    expect(textarea).toBeTruthy();
  });

  it("shows spinner when isWaiting is true", async () => {
    const rendered = await renderChat({ isWaiting: true });
    expect(rendered.container.querySelector(".spinner")).toBeTruthy();
    const btn = rendered.container.querySelector(
      ".send-btn"
    ) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("disables textarea when isWaiting is true", async () => {
    const rendered = await renderChat({ isWaiting: true });
    const textarea = rendered.getByPlaceholderText(
      "Type a message and press Enter..."
    ) as HTMLTextAreaElement;
    expect(textarea.disabled).toBe(true);
  });

  it("calls onSend when Enter is pressed (not Shift+Enter)", async () => {
    const onSend = vi.fn();
    const rendered = await renderChat({ onSend });
    const textarea = rendered.getByPlaceholderText(
      "Type a message and press Enter..."
    ) as HTMLTextAreaElement;

    await fireEvent.input(textarea, { target: { value: "test message" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).toHaveBeenCalledWith("test message");
  });

  it("does not call onSend on Shift+Enter (allows newline)", async () => {
    const onSend = vi.fn();
    const rendered = await renderChat({ onSend });
    const textarea = rendered.getByPlaceholderText(
      "Type a message and press Enter..."
    ) as HTMLTextAreaElement;

    await fireEvent.input(textarea, { target: { value: "line1" } });
    await fireEvent.keyDown(textarea, { key: "Enter", shiftKey: true });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("does not send on Enter during IME composition", async () => {
    const onSend = vi.fn();
    const rendered = await renderChat({ onSend });
    const textarea = rendered.getByPlaceholderText(
      "Type a message and press Enter..."
    ) as HTMLTextAreaElement;

    await fireEvent.input(textarea, { target: { value: "日本語" } });
    await fireEvent.compositionStart(textarea);
    await fireEvent.keyDown(textarea, { key: "Enter", isComposing: true });

    expect(onSend).not.toHaveBeenCalled();

    await fireEvent.compositionEnd(textarea);
    // Immediately after compositionEnd, Enter should still be suppressed
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(onSend).not.toHaveBeenCalled();

    // After microtask, Enter should work
    await new Promise((r) => setTimeout(r, 0));
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("日本語");
  });

  it("clears input after sending", async () => {
    const onSend = vi.fn();
    const rendered = await renderChat({ onSend });
    const textarea = rendered.getByPlaceholderText(
      "Type a message and press Enter..."
    ) as HTMLTextAreaElement;

    await fireEvent.input(textarea, { target: { value: "clear me" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).toHaveBeenCalledWith("clear me");
    expect(textarea.value).toBe("");
  });

  it("does not send empty messages", async () => {
    const onSend = vi.fn();
    const rendered = await renderChat({ onSend });
    const textarea = rendered.getByPlaceholderText(
      "Type a message and press Enter..."
    ) as HTMLTextAreaElement;

    await fireEvent.input(textarea, { target: { value: "   " } });
    await fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("displays timestamps on messages", async () => {
    const ts = 1700000000000; // 2023-11-14T22:13:20.000Z
    const rendered = await renderChat({
      messages: [makeMsg({ content: "timestamped", timestamp: ts })],
    });
    // Timestamp should be rendered somewhere in the message
    const timeEl = rendered.container.querySelector(".lead-timestamp");
    expect(timeEl).toBeTruthy();
    expect(timeEl!.textContent).toBeTruthy();
  });

  it("calls onSend when send button is clicked", async () => {
    const onSend = vi.fn();
    const rendered = await renderChat({ onSend });
    const textarea = rendered.getByPlaceholderText(
      "Type a message and press Enter..."
    ) as HTMLTextAreaElement;

    await fireEvent.input(textarea, { target: { value: "via button" } });
    const btn = rendered.container.querySelector(".send-btn") as HTMLButtonElement;
    await fireEvent.click(btn);

    expect(onSend).toHaveBeenCalledWith("via button");
  });
});
