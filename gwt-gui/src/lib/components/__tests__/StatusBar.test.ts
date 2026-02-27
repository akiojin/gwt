import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, cleanup } from "@testing-library/svelte";
import type { AgentInfo } from "../../types";

const tauriInvokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => tauriInvokeMock(...args),
}));

type StatusBarProps = {
  projectPath: string;
  currentBranch?: string;
  terminalCount?: number;
  osEnvReady?: boolean;
  voiceInputEnabled?: boolean;
  voiceInputListening?: boolean;
  voiceInputPreparing?: boolean;
  voiceInputSupported?: boolean;
  voiceInputAvailable?: boolean;
  voiceInputAvailabilityReason?: string | null;
  voiceInputError?: string | null;
};

async function renderStatusBar(props: StatusBarProps) {
  const { default: StatusBar } = await import("../StatusBar.svelte");
  return render(StatusBar, { props });
}

describe("StatusBar", () => {
  beforeEach(() => {
    tauriInvokeMock.mockReset();
    cleanup();
  });

  afterEach(() => {
    cleanup();
  });

  // --- Branch display ---
  it("displays the current branch name", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      currentBranch: "main",
    });

    await waitFor(() => {
      expect(rendered.getByText("main")).toBeTruthy();
    });
  });

  it("displays '---' when no branch is set", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      currentBranch: "",
    });

    await waitFor(() => {
      expect(rendered.getByText("---")).toBeTruthy();
    });
  });

  // --- Terminal count ---
  it("displays terminal count when terminals exist", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      terminalCount: 3,
    });

    await waitFor(() => {
      expect(rendered.getByText("[3 terminals]")).toBeTruthy();
    });
  });

  it("shows singular 'terminal' for count of 1", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      terminalCount: 1,
    });

    await waitFor(() => {
      expect(rendered.getByText("[1 terminal]")).toBeTruthy();
    });
  });

  it("hides terminal count when 0", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      terminalCount: 0,
    });

    expect(rendered.queryByText(/terminal/)).toBeNull();
  });

  // --- Project path ---
  it("displays the project path", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/home/user/my-project",
    });

    await waitFor(() => {
      expect(rendered.getByText("/home/user/my-project")).toBeTruthy();
    });
  });

  // --- osEnvReady loading ---
  it("shows 'Loading environment...' when osEnvReady is false", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      osEnvReady: false,
    });

    await waitFor(() => {
      expect(rendered.getByText("Loading environment...")).toBeTruthy();
    });
  });

  it("shows 'Agents: waiting' when osEnvReady is false", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      osEnvReady: false,
    });

    await waitFor(() => {
      expect(rendered.getByText("Agents: waiting")).toBeTruthy();
    });
  });

  // --- Agent detection ---
  it("shows agent statuses after detection", async () => {
    // Flush pending microtasks from previous test's $effect cleanup
    await new Promise((r) => setTimeout(r, 0));

    const agents: AgentInfo[] = [
      { id: "claude", name: "Claude", version: "1.2.0", available: true, authenticated: true },
      { id: "codex", name: "Codex", version: "", available: true, authenticated: true },
      { id: "gemini", name: "Gemini", version: "", available: false, authenticated: false },
      { id: "opencode", name: "OpenCode", version: "0.5.0", available: true, authenticated: true },
    ];

    tauriInvokeMock.mockResolvedValue(agents);

    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      osEnvReady: true,
    });

    await waitFor(() => {
      expect(rendered.getByText("Claude:1.2.0")).toBeTruthy();
    });

    expect(rendered.getByText("Codex:installed")).toBeTruthy();
    expect(rendered.getByText("Gemini:not installed")).toBeTruthy();
    expect(rendered.getByText("OpenCode:0.5.0")).toBeTruthy();
  });

  it("shows 'not installed' for agents not in the returned list", async () => {
    const agents: AgentInfo[] = [
      { id: "claude", name: "Claude", version: "1.0.0", available: true, authenticated: true },
    ];

    tauriInvokeMock.mockResolvedValue(agents);

    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      osEnvReady: true,
    });

    await waitFor(() => {
      expect(rendered.getByText("Claude:1.0.0")).toBeTruthy();
    });

    expect(rendered.getByText("Codex:not installed")).toBeTruthy();
    expect(rendered.getByText("Gemini:not installed")).toBeTruthy();
    expect(rendered.getByText("OpenCode:not installed")).toBeTruthy();
  });

  it("shows 'Agents: ...' while loading agents", async () => {
    // detect_agents never resolves
    tauriInvokeMock.mockReturnValue(new Promise(() => {}));

    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      osEnvReady: true,
    });

    await waitFor(() => {
      expect(rendered.getByText("Agents: ...")).toBeTruthy();
    });
  });

  it("handles agent detection error gracefully", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("network error"));

    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      osEnvReady: true,
    });

    await waitFor(() => {
      // After error, agents = [] so all agents show 'not installed'
      expect(rendered.getByText("Claude:not installed")).toBeTruthy();
      expect(rendered.getByText("Codex:not installed")).toBeTruthy();
    });
  });

  it("shows warn class for agent with bunx version", async () => {
    const agents: AgentInfo[] = [
      { id: "claude", name: "Claude", version: "bunx", available: true, authenticated: true },
    ];

    tauriInvokeMock.mockResolvedValue(agents);

    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      osEnvReady: true,
    });

    await waitFor(() => {
      expect(rendered.getByText("Claude:bunx")).toBeTruthy();
    });

    const claudeSpan = rendered.getByText("Claude:bunx");
    expect(claudeSpan.classList.contains("warn")).toBe(true);
  });

  it("shows warn class for agent with npx version", async () => {
    const agents: AgentInfo[] = [
      { id: "codex", name: "Codex", version: "npx", available: true, authenticated: true },
    ];

    tauriInvokeMock.mockResolvedValue(agents);

    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      osEnvReady: true,
    });

    await waitFor(() => {
      expect(rendered.getByText("Codex:npx")).toBeTruthy();
    });

    const codexSpan = rendered.getByText("Codex:npx");
    expect(codexSpan.classList.contains("warn")).toBe(true);
  });

  // --- Voice status ---
  it("shows 'Voice: off' when voiceInputEnabled is false", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputSupported: true,
      voiceInputAvailable: true,
      voiceInputEnabled: false,
    });

    await waitFor(() => {
      expect(rendered.getByText("Voice: off")).toBeTruthy();
    });
  });

  it("shows 'Voice: listening' when actively listening", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputSupported: true,
      voiceInputAvailable: true,
      voiceInputEnabled: true,
      voiceInputListening: true,
    });

    await waitFor(() => {
      expect(rendered.getByText("Voice: listening")).toBeTruthy();
    });
  });

  it("shows 'Voice: preparing' when preparing", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputSupported: true,
      voiceInputAvailable: true,
      voiceInputEnabled: true,
      voiceInputPreparing: true,
    });

    await waitFor(() => {
      expect(rendered.getByText("Voice: preparing")).toBeTruthy();
    });
  });

  it("shows 'Voice: error' when there is a voice error", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputSupported: true,
      voiceInputAvailable: true,
      voiceInputEnabled: true,
      voiceInputError: "microphone failed",
    });

    await waitFor(() => {
      expect(rendered.getByText("Voice: error")).toBeTruthy();
    });
  });

  it("shows 'Voice: idle' when enabled but not listening or preparing", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputSupported: true,
      voiceInputAvailable: true,
      voiceInputEnabled: true,
      voiceInputListening: false,
      voiceInputPreparing: false,
    });

    await waitFor(() => {
      expect(rendered.getByText("Voice: idle")).toBeTruthy();
    });
  });

  it("shows 'Voice: backend unavailable' when not supported", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputSupported: false,
    });

    await waitFor(() => {
      expect(rendered.getByText("Voice: backend unavailable")).toBeTruthy();
    });
  });

  it("shows 'Voice: unavailable' when not available", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputSupported: true,
      voiceInputAvailable: false,
    });

    await waitFor(() => {
      expect(rendered.getByText("Voice: unavailable")).toBeTruthy();
    });
  });

  it("sets voice error as title attribute", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputSupported: true,
      voiceInputAvailable: true,
      voiceInputEnabled: true,
      voiceInputError: "microphone permission denied",
    });

    await waitFor(() => {
      const voiceSpan = rendered.getByText("Voice: error");
      expect(voiceSpan.getAttribute("title")).toBe("microphone permission denied");
    });
  });

  it("sets availability reason as title attribute when no error", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputSupported: true,
      voiceInputAvailable: false,
      voiceInputAvailabilityReason: "No GPU detected",
    });

    await waitFor(() => {
      const voiceSpan = rendered.getByText("Voice: unavailable");
      expect(voiceSpan.getAttribute("title")).toBe("No GPU detected");
    });
  });

  // --- Voice status CSS classes ---
  it("applies 'ok' class when voice is listening", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputSupported: true,
      voiceInputAvailable: true,
      voiceInputEnabled: true,
      voiceInputListening: true,
    });

    await waitFor(() => {
      const voiceSpan = rendered.getByText("Voice: listening");
      expect(voiceSpan.classList.contains("ok")).toBe(true);
    });
  });

  it("applies 'muted' class when voice is off", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputSupported: true,
      voiceInputAvailable: true,
      voiceInputEnabled: false,
    });

    await waitFor(() => {
      const voiceSpan = rendered.getByText("Voice: off");
      expect(voiceSpan.classList.contains("muted")).toBe(true);
    });
  });

  it("applies 'bad' class when voice is not supported", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputSupported: false,
    });

    await waitFor(() => {
      const voiceSpan = rendered.getByText("Voice: backend unavailable");
      expect(voiceSpan.classList.contains("bad")).toBe(true);
    });
  });

  it("applies 'warn' class when voice has error", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("fail"));
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputSupported: true,
      voiceInputAvailable: true,
      voiceInputEnabled: true,
      voiceInputError: "some error",
    });

    await waitFor(() => {
      const voiceSpan = rendered.getByText("Voice: error");
      expect(voiceSpan.classList.contains("warn")).toBe(true);
    });
  });

  // --- statusbar helpers (also tested separately) ---
  // These are tested separately in statusBarHelpers.test.ts; keeping the original tests below.
});

// Keep the original helper tests intact
import { renderBar, usageColorClass, formatMemory } from "../statusBarHelpers";

describe("StatusBar helpers", () => {
  describe("renderBar", () => {
    it("renders 50% as [||||    ]", () => {
      expect(renderBar(50)).toBe("[||||    ]");
    });

    it("renders 0% as [        ]", () => {
      expect(renderBar(0)).toBe("[        ]");
    });

    it("renders 100% as [||||||||]", () => {
      expect(renderBar(100)).toBe("[||||||||]");
    });

    it("renders 25% as [||      ]", () => {
      expect(renderBar(25)).toBe("[||      ]");
    });
  });

  describe("usageColorClass", () => {
    it("returns 'ok' for usage below 70%", () => {
      expect(usageColorClass(0)).toBe("ok");
      expect(usageColorClass(50)).toBe("ok");
      expect(usageColorClass(69)).toBe("ok");
    });

    it("returns 'warn' for 70-89%", () => {
      expect(usageColorClass(70)).toBe("warn");
      expect(usageColorClass(75)).toBe("warn");
      expect(usageColorClass(89)).toBe("warn");
    });

    it("returns 'bad' for 90% and above", () => {
      expect(usageColorClass(90)).toBe("bad");
      expect(usageColorClass(95)).toBe("bad");
      expect(usageColorClass(100)).toBe("bad");
    });
  });

  describe("formatMemory", () => {
    it("formats 8 GB correctly", () => {
      expect(formatMemory(8589934592)).toBe("8.0");
    });

    it("formats 16 GB correctly", () => {
      expect(formatMemory(17179869184)).toBe("16.0");
    });

    it("formats 0 bytes correctly", () => {
      expect(formatMemory(0)).toBe("0.0");
    });

    it("formats fractional GB correctly", () => {
      // 4.5 GB = 4831838208
      expect(formatMemory(4831838208)).toBe("4.5");
    });
  });
});
