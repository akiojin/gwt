import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, waitFor, cleanup } from "@testing-library/svelte";
import type { ProjectModeState } from "../types";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
}));

const baseState: ProjectModeState = {
  messages: [],
  ai_ready: true,
  ai_error: null,
  last_error: null,
  is_waiting: false,
  session_name: "Project Mode",
  llm_call_count: 0,
  estimated_tokens: 0,
};

async function renderPanel(
  initialOverride?: Partial<ProjectModeState>,
  sendOverride?: Partial<ProjectModeState>
) {
  invokeMock.mockImplementation(async (command: string) => {
    if (command === "get_project_mode_state_cmd") {
      return { ...baseState, ...initialOverride };
    }
    if (command === "send_project_mode_message_cmd") {
      return { ...baseState, ...sendOverride };
    }
    return baseState;
  });

  const { default: ProjectModePanel } = await import("./ProjectModePanel.svelte");
  return render(ProjectModePanel);
}

function countInvokeCalls(name: string): number {
  return invokeMock.mock.calls.filter((c) => c[0] === name).length;
}

describe("ProjectModePanel", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
  });

  it("does not send on Enter during IME composition", async () => {
    const rendered = await renderPanel();

    await waitFor(() => {
      expect(countInvokeCalls("get_project_mode_state_cmd")).toBe(1);
    });

    const textarea = rendered.getByPlaceholderText(
      "Type a task and press Enter..."
    ) as HTMLTextAreaElement;

    await fireEvent.input(textarea, { target: { value: "日本語入力" } });
    await fireEvent.compositionStart(textarea);
    await fireEvent.keyDown(textarea, { key: "Enter", isComposing: true });

    expect(countInvokeCalls("send_project_mode_message_cmd")).toBe(0);

    await fireEvent.compositionEnd(textarea);
    await fireEvent.keyDown(textarea, { key: "Enter" });

    expect(countInvokeCalls("send_project_mode_message_cmd")).toBe(0);

    await new Promise((r) => setTimeout(r, 0));
    await fireEvent.keyDown(textarea, { key: "Enter" });

    await waitFor(() => {
      expect(countInvokeCalls("send_project_mode_message_cmd")).toBe(1);
    });
  });

  it("shows spinner while request is waiting", async () => {
    const rendered = await renderPanel({}, { is_waiting: true });

    await waitFor(() => {
      expect(countInvokeCalls("get_project_mode_state_cmd")).toBe(1);
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

  it("auto-scrolls to latest message", async () => {
    const rendered = await renderPanel(
      {},
      {
        messages: [
          {
            role: "assistant",
            content: "latest",
            timestamp: Date.now(),
          },
        ],
      }
    );

    await waitFor(() => {
      expect(countInvokeCalls("get_project_mode_state_cmd")).toBe(1);
    });

    const chat = rendered.container.querySelector(".agent-chat") as HTMLDivElement;
    let lastScrollTop = 0;
    Object.defineProperty(chat, "scrollHeight", {
      value: 200,
      configurable: true,
    });
    Object.defineProperty(chat, "scrollTop", {
      get() {
        return lastScrollTop;
      },
      set(value) {
        lastScrollTop = value;
      },
      configurable: true,
    });

    const textarea = rendered.getByPlaceholderText(
      "Type a task and press Enter..."
    ) as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "scroll" } });

    const button = rendered.getByRole("button", { name: "Send" });
    await fireEvent.click(button);

    await waitFor(() => {
      expect(lastScrollTop).toBe(200);
    });
  });

  it("shows custom session/lead metadata in header", async () => {
    const rendered = await renderPanel({
      session_name: "Sprint Planning",
      lead_status: "running",
      project_mode_session_id: "pm-123",
      llm_call_count: 7,
      estimated_tokens: 2048,
    });

    await waitFor(() => {
      expect(rendered.getByText("Sprint Planning")).toBeTruthy();
      expect(rendered.getByText("Lead: running")).toBeTruthy();
      expect(rendered.getByText("Session: pm-123")).toBeTruthy();
      expect(rendered.getByText("LLM: 7")).toBeTruthy();
      expect(rendered.getByText("Tokens: 2048")).toBeTruthy();
    });
  });

  it("does not send on Shift+Enter and ignores blank input", async () => {
    const rendered = await renderPanel();

    await waitFor(() => {
      expect(countInvokeCalls("get_project_mode_state_cmd")).toBe(1);
    });

    const textarea = rendered.getByPlaceholderText(
      "Type a task and press Enter..."
    ) as HTMLTextAreaElement;

    await fireEvent.input(textarea, { target: { value: "   " } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(countInvokeCalls("send_project_mode_message_cmd")).toBe(0);

    await fireEvent.input(textarea, { target: { value: "keep newline" } });
    await fireEvent.keyDown(textarea, { key: "Enter", shiftKey: true });
    expect(countInvokeCalls("send_project_mode_message_cmd")).toBe(0);
  });

  it("renders fallback AI warning and stringified error when send fails with object message", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_project_mode_state_cmd") {
        return {
          ...baseState,
          ai_ready: false,
          ai_error: null,
        };
      }
      if (command === "send_project_mode_message_cmd") {
        throw { message: 123 };
      }
      return baseState;
    });

    const { default: ProjectModePanel } = await import("./ProjectModePanel.svelte");
    const rendered = render(ProjectModePanel);

    await waitFor(() => {
      expect(rendered.getByText("AI settings are required.")).toBeTruthy();
    });

    const textarea = rendered.getByPlaceholderText(
      "Type a task and press Enter..."
    ) as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "send" } });
    await fireEvent.click(rendered.getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(rendered.getByText("[object Object]")).toBeTruthy();
    });
  });

  it("formats null error as Unknown error when initial state fetch fails", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_project_mode_state_cmd") {
        throw null;
      }
      return baseState;
    });

    const { default: ProjectModePanel } = await import("./ProjectModePanel.svelte");
    const rendered = render(ProjectModePanel);

    await waitFor(() => {
      expect(rendered.getByText("Unknown error")).toBeTruthy();
    });
  });

  it("dispatches spec issue event once per issue number", async () => {
    const dispatchSpy = vi.spyOn(window, "dispatchEvent");
    const rendered = await renderPanel(
      {
        active_spec_issue_number: 42,
        active_spec_id: null,
        active_spec_issue_url: null,
      },
      {
        active_spec_issue_number: 42,
        active_spec_id: null,
        active_spec_issue_url: null,
      }
    );

    await waitFor(() => {
      expect(dispatchSpy).toHaveBeenCalledTimes(1);
    });

    const specEvent = dispatchSpy.mock.calls.find(
      (call) =>
        call[0] instanceof CustomEvent &&
        call[0].type === "gwt-project-mode-open-spec-issue"
    )?.[0] as CustomEvent | undefined;

    expect(specEvent?.detail).toEqual({
      issueNumber: 42,
      specId: null,
      issueUrl: null,
    });

    const textarea = rendered.getByPlaceholderText(
      "Type a task and press Enter..."
    ) as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "trigger refresh" } });
    await fireEvent.click(rendered.getByRole("button", { name: "Send" }));

    await waitFor(() => {
      expect(countInvokeCalls("send_project_mode_message_cmd")).toBe(1);
    });
    expect(dispatchSpy).toHaveBeenCalledTimes(1);
  });
});
