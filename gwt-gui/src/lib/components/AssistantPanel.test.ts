import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  createEvent,
  fireEvent,
  render,
  waitFor,
} from "@testing-library/svelte";
import { tick } from "svelte";

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
    invokeMock.mockImplementation(async (command: string, args?: { input?: string }) => {
      if (command === "assistant_get_state") {
        return structuredClone(assistantStateFixture);
      }
      if (command === "assistant_get_dashboard") {
        return structuredClone(dashboardFixture);
      }
      if (command === "assistant_send_message") {
        return {
          ...structuredClone(assistantStateFixture),
          messages: [
            {
              role: "user",
              kind: "text",
              content: args?.input ?? "",
              timestamp: Date.now(),
            },
          ],
        };
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
});
