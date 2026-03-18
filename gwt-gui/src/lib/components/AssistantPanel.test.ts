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
const eventHandlers = new Map<string, (event: { payload: unknown }) => void>();

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
  startupStatus: "ready",
  startupSummaryReady: true,
  startupFailureKind: null,
  startupFailureDetail: null,
  startupRecoveryHints: [],
  blockers: [],
  recommendedNextActions: [],
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

async function renderAssistantPanel(
  props: {
    isActive?: boolean;
    projectPath?: string | null;
    onOpenSettings?: () => void;
  } = {},
) {
  const { default: AssistantPanel } = await import("./AssistantPanel.svelte");
  return render(AssistantPanel, { props });
}

async function waitForTextarea(rendered: Awaited<ReturnType<typeof renderAssistantPanel>>) {
  const textarea = rendered.getByPlaceholderText("Type a message...") as HTMLTextAreaElement;
  await waitFor(() => {
    expect(textarea.disabled).toBe(false);
  });
  return textarea;
}

async function sendWithEnter(textarea: HTMLTextAreaElement, value: string) {
  const baseline = getAssistantSendCalls().length;
  await fireEvent.input(textarea, { target: { value } });
  textarea.setSelectionRange(textarea.value.length, textarea.value.length);
  await fireEvent.keyDown(textarea, {
    key: "Enter",
    code: "Enter",
    keyCode: 13,
    which: 13,
  });
  await waitFor(() => {
    expect(getAssistantSendCalls()).toHaveLength(baseline + 1);
  });
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
    eventHandlers.clear();

    listenMock.mockImplementation(async (eventName: string, handler: unknown) => {
      if (typeof eventName === "string" && typeof handler === "function") {
        eventHandlers.set(eventName, handler as (event: { payload: unknown }) => void);
      }
      return () => {
        eventHandlers.delete(eventName);
      };
    });
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

  it("disables the composer while the assistant is thinking", async () => {
    initialAssistantState = {
      ...structuredClone(assistantStateFixture),
      isThinking: true,
      sessionId: "session-main",
      messages: [
        {
          role: "assistant",
          kind: "text",
          content: "Checking startup analysis cache...",
          timestamp: 1,
        },
      ],
    };

    const rendered = await renderAssistantPanel();
    const textarea = rendered.getByPlaceholderText("Type a message...") as HTMLTextAreaElement;
    const button = rendered.getByText("Send") as HTMLButtonElement;

    await waitFor(() => {
      expect(textarea.disabled).toBe(true);
      expect(button.disabled).toBe(true);
      expect(rendered.getByText("Checking startup analysis cache...")).toBeTruthy();
      expect(rendered.getByText("Thinking...")).toBeTruthy();
    });
  });

  it("shows the current goal and recommended next actions in the dashboard strip", async () => {
    initialAssistantState = {
      ...structuredClone(assistantStateFixture),
      workingGoal: "#1636 Assistant Mode",
      goalConfidence: "high",
      currentStatus: "monitoring",
      recommendedNextActions: [
        "ブランチ `feature/issue-1636` で agent を起動して作業を再開する",
      ],
    };

    const rendered = await renderAssistantPanel();

    await waitFor(() => {
      expect(rendered.getByTestId("assistant-goal-strip")).toBeTruthy();
      expect(rendered.getByText("#1636 Assistant Mode")).toBeTruthy();
      expect(rendered.getByText("Monitoring")).toBeTruthy();
      expect(
        rendered.getByText("ブランチ `feature/issue-1636` で agent を起動して作業を再開する")
      ).toBeTruthy();
    });
  });

  it("renders goal confirmation state with blockers", async () => {
    initialAssistantState = {
      ...structuredClone(assistantStateFixture),
      currentStatus: "awaiting_goal_confirmation",
      blockers: [
        "README / CLAUDE.md / 現在の branch から、着手中のゴールを一意に特定できません。",
      ],
      recommendedNextActions: [
        "現在のゴールを一文で確認し、必要なら issue または README に明記する",
      ],
      messages: [
        {
          role: "assistant",
          kind: "text",
          content: "## Assistant PM Update\n現在の作業ゴールを一文で確認してください。",
          timestamp: 1,
        },
      ],
    };

    const rendered = await renderAssistantPanel();

    await waitFor(() => {
      expect(rendered.getByText("Needs Goal")).toBeTruthy();
      expect(
        rendered.getByText(
          "README / CLAUDE.md / 現在の branch から、着手中のゴールを一意に特定できません。"
        )
      ).toBeTruthy();
      expect(rendered.getByText("現在の作業ゴールを一文で確認してください。")).toBeTruthy();
    });
  });

  it("preserves line breaks in rendered message content", async () => {
    initialAssistantState = {
      ...structuredClone(assistantStateFixture),
      messages: [
        {
          role: "user",
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

  it("renders assistant markdown output as formatted content", async () => {
    initialAssistantState = {
      ...structuredClone(assistantStateFixture),
      messages: [
        {
          role: "assistant",
          kind: "text",
          content: "## Summary\n- item one\n- item two",
          timestamp: 1,
        },
      ],
    };

    const rendered = await renderAssistantPanel();

    await waitFor(() => {
      expect(rendered.container.querySelector(".message.assistant h2")?.textContent).toBe(
        "Summary"
      );
      expect(rendered.container.querySelectorAll(".message.assistant li")).toHaveLength(2);
    });
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

  it("navigates sent user input history with ArrowUp and ArrowDown, then restores the draft", async () => {
    const rendered = await renderAssistantPanel();
    const textarea = await waitForTextarea(rendered);

    await sendWithEnter(textarea, "first");
    await sendWithEnter(textarea, "second");

    await fireEvent.input(textarea, { target: { value: "draft" } });
    textarea.setSelectionRange(textarea.value.length, textarea.value.length);

    await fireEvent.keyDown(textarea, { key: "ArrowUp", code: "ArrowUp" });
    await waitFor(() => {
      expect(textarea.value).toBe("second");
    });

    textarea.setSelectionRange(textarea.value.length, textarea.value.length);
    await fireEvent.keyDown(textarea, { key: "ArrowUp", code: "ArrowUp" });
    await waitFor(() => {
      expect(textarea.value).toBe("first");
    });

    textarea.setSelectionRange(textarea.value.length, textarea.value.length);
    await fireEvent.keyDown(textarea, { key: "ArrowDown", code: "ArrowDown" });
    await waitFor(() => {
      expect(textarea.value).toBe("second");
    });

    textarea.setSelectionRange(textarea.value.length, textarea.value.length);
    await fireEvent.keyDown(textarea, { key: "ArrowDown", code: "ArrowDown" });
    await waitFor(() => {
      expect(textarea.value).toBe("draft");
    });
  });

  it("does not navigate history with ArrowUp when the caret is not on the first line", async () => {
    const rendered = await renderAssistantPanel();
    const textarea = await waitForTextarea(rendered);

    await sendWithEnter(textarea, "previous");
    await fireEvent.input(textarea, { target: { value: "line 1\nline 2" } });

    const secondLineOffset = textarea.value.indexOf("\n") + 1;
    textarea.setSelectionRange(secondLineOffset, secondLineOffset);
    await fireEvent.keyDown(textarea, { key: "ArrowUp", code: "ArrowUp" });

    expect(textarea.value).toBe("line 1\nline 2");
  });

  it("does not leave multiline history with ArrowDown before the caret reaches the last line", async () => {
    const rendered = await renderAssistantPanel();
    const textarea = await waitForTextarea(rendered);

    await sendWithEnter(textarea, "older\nentry");
    await fireEvent.input(textarea, { target: { value: "draft" } });
    textarea.setSelectionRange(textarea.value.length, textarea.value.length);

    await fireEvent.keyDown(textarea, { key: "ArrowUp", code: "ArrowUp" });
    await waitFor(() => {
      expect(textarea.value).toBe("older\nentry");
    });

    textarea.setSelectionRange(2, 2);
    await fireEvent.keyDown(textarea, { key: "ArrowDown", code: "ArrowDown" });

    expect(textarea.value).toBe("older\nentry");
  });

  it("does not navigate history when the input text is selected", async () => {
    const rendered = await renderAssistantPanel();
    const textarea = await waitForTextarea(rendered);

    await sendWithEnter(textarea, "previous");
    await fireEvent.input(textarea, { target: { value: "draft" } });
    textarea.setSelectionRange(0, textarea.value.length);

    await fireEvent.keyDown(textarea, { key: "ArrowUp", code: "ArrowUp" });

    expect(textarea.value).toBe("draft");
  });

  it("reloads the dashboard when the Assistant tab becomes active again", async () => {
    const rendered = await renderAssistantPanel({
      isActive: true,
      projectPath: "/tmp/project",
    });

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter(([command]) => command === "assistant_get_dashboard"),
      ).toHaveLength(1);
    });

    await rendered.rerender({ isActive: false, projectPath: "/tmp/project" });
    expect(
      invokeMock.mock.calls.filter(([command]) => command === "assistant_get_dashboard"),
    ).toHaveLength(1);

    await rendered.rerender({ isActive: true, projectPath: "/tmp/project" });
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter(([command]) => command === "assistant_get_dashboard"),
      ).toHaveLength(2);
    });
  });

  it("reloads the dashboard when launch and close events arrive", async () => {
    await renderAssistantPanel({
      isActive: false,
      projectPath: "/tmp/project",
    });

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter(([command]) => command === "assistant_get_dashboard"),
      ).toHaveLength(1);
    });

    eventHandlers.get("launch-finished")?.({ payload: { paneId: "pane-1" } });
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter(([command]) => command === "assistant_get_dashboard"),
      ).toHaveLength(2);
    });

    eventHandlers.get("terminal-closed")?.({ payload: { pane_id: "pane-1" } });
    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter(([command]) => command === "assistant_get_dashboard"),
      ).toHaveLength(3);
    });
  });

  it("shows a startup recovery panel for failed autonomous startup", async () => {
    initialAssistantState = {
      ...structuredClone(assistantStateFixture),
      sessionId: "session-main",
      startupStatus: "failed",
      startupFailureKind: "resource_guard",
      startupFailureDetail:
        'Failed to load model "qwen/qwen3.5-35b-a3b" due to insufficient system resources.',
      startupRecoveryHints: [
        "Choose a smaller model in Settings.",
        "Switch to a remote inference endpoint if available.",
      ],
    };

    const rendered = await renderAssistantPanel();

    await waitFor(() => {
      expect(rendered.getByTestId("assistant-startup-recovery")).toBeTruthy();
      expect(rendered.getByText("Selected model is too heavy for this machine")).toBeTruthy();
      expect(rendered.getByText("Choose a smaller model in Settings.")).toBeTruthy();
      expect(rendered.getByText("Open Settings")).toBeTruthy();
      expect(rendered.getByText("Retry")).toBeTruthy();
    });
  });

  it("retries assistant startup from the recovery panel", async () => {
    initialAssistantState = {
      ...structuredClone(assistantStateFixture),
      sessionId: "session-main",
      startupStatus: "failed",
      startupFailureKind: "llm_error",
      startupFailureDetail: "LLM call failed",
      startupRecoveryHints: ["Retry the Assistant startup."],
    };

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
        return {
          ...structuredClone(assistantStateFixture),
          startupStatus: "ready",
          startupSummaryReady: true,
          messages: [
            {
              role: "assistant",
              kind: "text",
              content: "Recovered",
              timestamp: 1,
            },
          ],
        };
      }
      throw new Error(`Unexpected invoke command: ${command}`);
    });

    const rendered = await renderAssistantPanel();

    await waitFor(() => {
      expect(rendered.getByTestId("assistant-startup-recovery")).toBeTruthy();
    });
    await fireEvent.click(rendered.getByText("Retry"));

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter(([command]) => command === "assistant_start"),
      ).toHaveLength(1);
      expect(rendered.getByText("Recovered")).toBeTruthy();
    });
  });

  it("opens settings from the recovery panel", async () => {
    const onOpenSettings = vi.fn();
    initialAssistantState = {
      ...structuredClone(assistantStateFixture),
      sessionId: "session-main",
      startupStatus: "failed",
      startupFailureKind: "ai_not_configured",
      startupFailureDetail: "AI is not configured.",
      startupRecoveryHints: ["Open Settings and configure the active AI profile."],
    };

    const rendered = await renderAssistantPanel({ onOpenSettings });

    await waitFor(() => {
      expect(rendered.getByTestId("assistant-startup-recovery")).toBeTruthy();
    });
    await fireEvent.click(rendered.getByText("Open Settings"));

    expect(onOpenSettings).toHaveBeenCalledTimes(1);
  });

  it("auto-retries failed startup after settings are saved while the tab is active", async () => {
    initialAssistantState = {
      ...structuredClone(assistantStateFixture),
      sessionId: "session-main",
      startupStatus: "failed",
      startupFailureKind: "resource_guard",
      startupFailureDetail: "insufficient system resources",
      startupRecoveryHints: ["Choose a smaller model in Settings."],
    };

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
        return {
          ...structuredClone(assistantStateFixture),
          startupStatus: "ready",
          startupSummaryReady: true,
          messages: [],
        };
      }
      throw new Error(`Unexpected invoke command: ${command}`);
    });

    await renderAssistantPanel({
      isActive: true,
      projectPath: "/tmp/project",
    });

    await waitFor(() => {
      expect(document.querySelector("[data-testid='assistant-startup-recovery']")).toBeTruthy();
    });
    window.dispatchEvent(new CustomEvent("gwt-settings-updated"));

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter(([command]) => command === "assistant_start"),
      ).toHaveLength(1);
    });
  });

  it("shows autonomous startup progress while assistant_start is pending", async () => {
    let resolveStart: (() => void) | undefined;
    initialAssistantState = {
      ...structuredClone(assistantStateFixture),
      sessionId: null,
      startupStatus: "idle",
      startupSummaryReady: false,
    };

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
        return new Promise<AssistantState>((resolve) => {
          resolveStart = () => {
            initialAssistantState = {
              ...structuredClone(assistantStateFixture),
              messages: [
                {
                  role: "assistant",
                  kind: "text",
                  content: "Current status\n- branch: main",
                  timestamp: Date.now(),
                },
              ],
            };
            resolve(structuredClone(initialAssistantState));
          };
        });
      }
      throw new Error(`Unexpected invoke command: ${command}`);
    });

    const rendered = await renderAssistantPanel({
      isActive: true,
      projectPath: "/tmp/project",
    });

    await waitFor(() => {
      expect(rendered.getByText("Analyzing project...")).toBeTruthy();
    });

    resolveStart?.();

    await waitFor(() => {
      expect(rendered.container.querySelector(".message.assistant p")?.textContent).toBe(
        "Current status"
      );
      expect(rendered.container.querySelector(".message.assistant li")?.textContent).toBe(
        "branch: main"
      );
    });
  });
});
