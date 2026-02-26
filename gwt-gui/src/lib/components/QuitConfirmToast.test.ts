import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, fireEvent, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();
type TauriEventHandler = (event: { payload: any }) => void;
const eventListeners = new Map<string, Set<TauriEventHandler>>();
const listenMock = vi.fn(async (eventName: string, handler: TauriEventHandler) => {
  let bucket = eventListeners.get(eventName);
  if (!bucket) {
    bucket = new Set();
    eventListeners.set(eventName, bucket);
  }
  bucket.add(handler);
  return () => {
    bucket?.delete(handler);
    if (bucket && bucket.size === 0) eventListeners.delete(eventName);
  };
});

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

async function emitTauriEvent(eventName: string, payload: any = null) {
  const handlers = Array.from(eventListeners.get(eventName) ?? []);
  for (const handler of handlers) {
    await handler({ payload });
  }
}

async function renderQuitConfirmToast() {
  const { default: QuitConfirmToast } = await import("./QuitConfirmToast.svelte");
  return render(QuitConfirmToast);
}

function countInvokeCalls(name: string): number {
  return invokeMock.mock.calls.filter((c) => c[0] === name).length;
}

describe("QuitConfirmToast", () => {
  beforeEach(() => {
    cleanup();
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    listenMock.mockClear();
    eventListeners.clear();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders nothing when quit-confirm-show event has not been emitted", async () => {
    const rendered = await renderQuitConfirmToast();

    await waitFor(() => {
      expect(rendered.queryByText("Press ⌘Q again to quit")).toBeNull();
    });
  });

  it("shows toast when quit-confirm-show event is emitted", async () => {
    const rendered = await renderQuitConfirmToast();

    await emitTauriEvent("quit-confirm-show");

    await waitFor(() => {
      expect(rendered.getByText("Press ⌘Q again to quit")).toBeTruthy();
    });
  });

  it("hides toast after 3 seconds timeout", async () => {
    vi.useFakeTimers();

    const rendered = await renderQuitConfirmToast();

    await emitTauriEvent("quit-confirm-show");

    await waitFor(() => {
      expect(rendered.getByText("Press ⌘Q again to quit")).toBeTruthy();
    });

    vi.advanceTimersByTime(3000);

    await waitFor(() => {
      expect(rendered.queryByText("Press ⌘Q again to quit")).toBeNull();
    });

    expect(countInvokeCalls("cancel_quit_confirm")).toBe(1);
  });

  it("hides toast on mousedown and calls cancel_quit_confirm", async () => {
    const rendered = await renderQuitConfirmToast();

    await emitTauriEvent("quit-confirm-show");

    await waitFor(() => {
      expect(rendered.getByText("Press ⌘Q again to quit")).toBeTruthy();
    });

    await fireEvent.mouseDown(document);

    await waitFor(() => {
      expect(rendered.queryByText("Press ⌘Q again to quit")).toBeNull();
    });

    expect(countInvokeCalls("cancel_quit_confirm")).toBe(1);
  });

  it("hides toast on keydown (non Cmd+Q) and calls cancel_quit_confirm", async () => {
    const rendered = await renderQuitConfirmToast();

    await emitTauriEvent("quit-confirm-show");

    await waitFor(() => {
      expect(rendered.getByText("Press ⌘Q again to quit")).toBeTruthy();
    });

    await fireEvent.keyDown(document, { key: "a" });

    await waitFor(() => {
      expect(rendered.queryByText("Press ⌘Q again to quit")).toBeNull();
    });

    expect(countInvokeCalls("cancel_quit_confirm")).toBe(1);
  });

  it("has fade-in animation class when visible", async () => {
    const rendered = await renderQuitConfirmToast();

    await emitTauriEvent("quit-confirm-show");

    await waitFor(() => {
      const toast = rendered.getByTestId("quit-confirm-toast");
      expect(toast).toBeTruthy();
      expect(toast.classList.contains("fade-in")).toBe(true);
    });
  });
});
