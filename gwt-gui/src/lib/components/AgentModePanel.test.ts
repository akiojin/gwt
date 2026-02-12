import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, waitFor, cleanup } from "@testing-library/svelte";
import type { AgentModeState } from "../types";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

const baseState: AgentModeState = {
  messages: [],
  ai_ready: true,
  ai_error: null,
  last_error: null,
  is_waiting: false,
  session_name: "Agent Mode",
  llm_call_count: 0,
  estimated_tokens: 0,
};

async function renderPanel(
  initialOverride?: Partial<AgentModeState>,
  sendOverride?: Partial<AgentModeState>
) {
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "get_agent_mode_state_cmd") {
      return { ...baseState, ...initialOverride };
    }
    if (command === "send_agent_mode_message") {
      return { ...baseState, ...sendOverride };
    }
    return baseState;
  });

  const { default: AgentModePanel } = await import("./AgentModePanel.svelte");
  return render(AgentModePanel);
}

function countInvokeCalls(name: string): number {
  return invokeMock.mock.calls.filter((c) => c[0] === name).length;
}

describe("AgentModePanel", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
  });

  it("does not send on Enter during IME composition", async () => {
    const rendered = await renderPanel();

    await waitFor(() => {
      expect(countInvokeCalls("get_agent_mode_state_cmd")).toBe(1);
    });

    const textarea = rendered.getByPlaceholderText(
      "Type a task and press Enter..."
    ) as HTMLTextAreaElement;

    await fireEvent.input(textarea, { target: { value: "日本語入力" } });
    await fireEvent.compositionStart(textarea);
    await fireEvent.keyDown(textarea, { key: "Enter", isComposing: true });

    expect(countInvokeCalls("send_agent_mode_message")).toBe(0);

    await fireEvent.compositionEnd(textarea);
    await fireEvent.keyDown(textarea, { key: "Enter" });

    await waitFor(() => {
      expect(countInvokeCalls("send_agent_mode_message")).toBe(1);
    });
  });

  it("shows spinner while request is waiting", async () => {
    const rendered = await renderPanel({}, { is_waiting: true });

    await waitFor(() => {
      expect(countInvokeCalls("get_agent_mode_state_cmd")).toBe(1);
    });

    const textarea = rendered.getByPlaceholderText(
      "Type a task and press Enter..."
    ) as HTMLTextAreaElement;

    await fireEvent.input(textarea, { target: { value: "status" } });

    const button = rendered.getByRole("button", { name: "Send" });
    await fireEvent.click(button);

    await waitFor(() => {
      expect(rendered.container.querySelector(".spinner")).toBeTruthy();
    });

    expect((button as HTMLButtonElement).disabled).toBe(true);
  });

  it("renders chat bubbles for messages", async () => {
    const rendered = await renderPanel({
      messages: [
        {
          role: "user",
          content: "hello",
          timestamp: Date.now(),
        },
        {
          role: "assistant",
          content: "hi",
          timestamp: Date.now() + 1,
        },
      ],
    });

    await waitFor(() => {
      expect(rendered.getByText("hello")).toBeTruthy();
    });

    expect(
      rendered.container.querySelector(".agent-message.user .agent-bubble")
    ).toBeTruthy();
    expect(
      rendered.container.querySelector(".agent-message.assistant .agent-bubble")
    ).toBeTruthy();
  });
});
