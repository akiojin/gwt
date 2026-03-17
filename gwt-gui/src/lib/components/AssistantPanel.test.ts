import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  createEvent,
  fireEvent,
  render,
  waitFor,
} from "@testing-library/svelte";
import { tick } from "svelte";
import type { AssistantState } from "../types";

const invokeMock = vi.fn();
const listenMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

const assistantStateFixture = {
  messages: [],
  aiReady: true,
  isThinking: false,
  sessionId: "session-main",
  llmCallCount: 0,
  estimatedTokens: 0,
};

const dashboardFixture = {
  panes: [],
  git: {
    branch: "main",
    uncommittedCount: 0,
    unpushedCount: 0,
  },
};

let initialAssistantState: AssistantState;
let sendMessageImpl: (args?: { input?: string }) => Promise<AssistantState>;

async function renderAssistantPanel() {
  const { default: AssistantPanel } = await import("./AssistantPanel.svelte");
  return render(AssistantPanel);
}

function getAssistantSendCalls() {
  return invokeMock.mock.calls.filter(([command]) => command === "assistant_send_message");
}

describe("AssistantPanel", () => {
  const originalRequestAnimationFrame = globalThis.requestAnimationFrame;
  const originalScrollIntoView = HTMLElement.prototype.scrollIntoView;

  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();

    listenMock.mockResolvedValue(() => {});
    initialAssistantState = structuredClone(assistantStateFixture);
    sendMessageImpl = async (args?: { input?: string }) => ({
      ...structuredClone(assistantStateFixture),
      messages: [
        {
          role: "user",
          kind: "text",
          content: args?.input ?? "",
          timestamp: Date.now(),
        },
      ],
    });

    invokeMock.mockImplementation(async (command: string, args?: { input?: string }) => {
      if (command === "assistant_get_state") {
        return structuredClone(initialAssistantState);
      }
      if (command === "assistant_get_dashboard") {
        return structuredClone(dashboardFixture);
      }
      if (command === "assistant_send_message") {
        return sendMessageImpl(args);
      }
      if (command === "assistant_start") {
        return undefined;
      }
      throw new Error(`Unexpected invoke command: ${command}`);
    });

    Object.defineProperty(globalThis, "requestAnimationFrame", {
      configurable: true,
      value: (callback: FrameRequestCallback) => {
        callback(0);
        return 1;
      },
    });
    HTMLElement.prototype.scrollIntoView = vi.fn();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    Object.defineProperty(globalThis, "requestAnimationFrame", {
      configurable: true,
      value: originalRequestAnimationFrame,
    });
    HTMLElement.prototype.scrollIntoView = originalScrollIntoView;
  });

  it("does not send when Enter is used to confirm IME composition", async () => {
    const rendered = await renderAssistantPanel();
    const textarea = rendered.getByPlaceholderText("Type a message...") as HTMLTextAreaElement;

    await waitFor(() => {
      expect(textarea.disabled).toBe(false);
    });

    await fireEvent.input(textarea, { target: { value: "こんにちは" } });
    await fireEvent.compositionStart(textarea);
    await fireEvent.compositionEnd(textarea);
    await tick();

    const event = createEvent.keyDown(textarea, {
      key: "Enter",
      code: "Enter",
      keyCode: 13,
      which: 13,
    });
    Object.defineProperty(event, "isComposing", {
      configurable: true,
      value: true,
    });

    await fireEvent(textarea, event);

    expect(getAssistantSendCalls()).toHaveLength(0);
  });

  it("does not send when the keydown falls back to IME keyCode 229", async () => {
    const rendered = await renderAssistantPanel();
    const textarea = rendered.getByPlaceholderText("Type a message...") as HTMLTextAreaElement;

    await waitFor(() => {
      expect(textarea.disabled).toBe(false);
    });

    await fireEvent.input(textarea, { target: { value: "変換中" } });
    await fireEvent.compositionStart(textarea);
    await fireEvent.compositionEnd(textarea);
    await tick();

    const event = createEvent.keyDown(textarea, {
      key: "Enter",
      code: "Enter",
      which: 229,
    });
    Object.defineProperty(event, "keyCode", {
      configurable: true,
      value: 229,
    });

    await fireEvent(textarea, event);

    expect(getAssistantSendCalls()).toHaveLength(0);
  });

  it("sends on plain Enter after composition is complete", async () => {
    const rendered = await renderAssistantPanel();
    const textarea = rendered.getByPlaceholderText("Type a message...") as HTMLTextAreaElement;

    await waitFor(() => {
      expect(textarea.disabled).toBe(false);
    });

    await fireEvent.input(textarea, { target: { value: "hello" } });
    await fireEvent.keyDown(textarea, {
      key: "Enter",
      code: "Enter",
      keyCode: 13,
      which: 13,
    });

    await waitFor(() => {
      expect(getAssistantSendCalls()).toHaveLength(1);
    });

    expect(getAssistantSendCalls()[0]).toEqual([
      "assistant_send_message",
      { input: "hello" },
    ]);
  });

  it("does not send on Shift+Enter", async () => {
    const rendered = await renderAssistantPanel();
    const textarea = rendered.getByPlaceholderText("Type a message...") as HTMLTextAreaElement;

    await waitFor(() => {
      expect(textarea.disabled).toBe(false);
    });

    await fireEvent.input(textarea, { target: { value: "hello" } });
    await fireEvent.keyDown(textarea, {
      key: "Enter",
      code: "Enter",
      keyCode: 13,
      which: 13,
      shiftKey: true,
    });

    expect(getAssistantSendCalls()).toHaveLength(0);
  });

  it("preserves line breaks in rendered message content", async () => {
    initialAssistantState = {
      ...structuredClone(assistantStateFixture),
      messages: [
        {
          role: "assistant",
          kind: "text",
          content: "line 1\nline 2",
          timestamp: 1,
        },
      ],
    };

    const rendered = await renderAssistantPanel();

    const content = await waitFor(() => {
      const el = rendered.container.querySelector(".message-content") as HTMLElement | null;
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });

    expect(content.textContent?.replace(/^\s+/, "")).toBe("line 1\nline 2");
    const source = await import("./AssistantPanel.svelte?raw");
    expect(source.default).toContain("white-space: pre-wrap;");
  });

  it("shows the user message immediately while assistant_send_message is pending", async () => {
    let resolveSend: ((state: AssistantState) => void) | undefined;
    sendMessageImpl = () =>
      new Promise<AssistantState>((resolve) => {
        resolveSend = resolve;
      });

    const rendered = await renderAssistantPanel();
    const textarea = rendered.getByPlaceholderText("Type a message...") as HTMLTextAreaElement;

    await waitFor(() => {
      expect(textarea.disabled).toBe(false);
    });

    await fireEvent.input(textarea, { target: { value: "first line\nsecond line" } });
    await fireEvent.keyDown(textarea, {
      key: "Enter",
      code: "Enter",
      keyCode: 13,
      which: 13,
    });

    await waitFor(() => {
      const message = rendered.container.querySelector(".message.user .message-content");
      expect(message?.textContent?.replace(/^\s+/, "")).toBe("first line\nsecond line");
    });
    expect(rendered.getByText("Thinking...")).toBeTruthy();
    expect(textarea.value).toBe("");

    resolveSend?.({
      ...structuredClone(assistantStateFixture),
      messages: [
        {
          role: "user",
          kind: "text",
          content: "first line\nsecond line",
          timestamp: 1,
        },
        {
          role: "assistant",
          kind: "text",
          content: "done",
          timestamp: 2,
        },
      ],
    });

    await waitFor(() => {
      expect(rendered.getByText("done")).toBeTruthy();
    });
  });

  it("rolls back the optimistic message and restores the input on send failure", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    sendMessageImpl = async () => {
      throw new Error("send failed");
    };

    const rendered = await renderAssistantPanel();
    const textarea = rendered.getByPlaceholderText("Type a message...") as HTMLTextAreaElement;

    await waitFor(() => {
      expect(textarea.disabled).toBe(false);
    });

    await fireEvent.input(textarea, { target: { value: "retry me" } });
    await fireEvent.keyDown(textarea, {
      key: "Enter",
      code: "Enter",
      keyCode: 13,
      which: 13,
    });

    await waitFor(() => {
      expect(textarea.value).toBe("retry me");
    });
    expect(rendered.container.querySelector(".message.user")).toBeNull();
    expect(rendered.queryByText("Thinking...")).toBeNull();
  });
});
