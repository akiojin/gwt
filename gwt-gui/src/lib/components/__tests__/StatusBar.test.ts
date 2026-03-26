import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { cleanup, render, waitFor } from "@testing-library/svelte";
import { renderBar, usageColorClass, formatMemory } from "../statusBarHelpers";

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
  beforeEach(() => cleanup());
  afterEach(() => cleanup());

  it("displays the current branch name", async () => {
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      currentBranch: "main",
    });
    await waitFor(() => {
      expect(rendered.getByText("main")).toBeTruthy();
    });
  });

  it("displays --- when no branch is set", async () => {
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      currentBranch: "",
    });
    await waitFor(() => {
      expect(rendered.getByText("---")).toBeTruthy();
    });
  });

  it("displays terminal count when terminals exist", async () => {
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      terminalCount: 3,
    });
    await waitFor(() => {
      expect(rendered.getByText("3 terminals")).toBeTruthy();
    });
  });

  it("hides terminal count when 0", async () => {
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      terminalCount: 0,
    });
    expect(rendered.queryByText(/terminal/)).toBeNull();
  });

  it("displays the project path", async () => {
    const rendered = await renderStatusBar({
      projectPath: "/home/user/my-project",
    });
    await waitFor(() => {
      expect(rendered.getByText("/home/user/my-project")).toBeTruthy();
    });
  });

  it("shows loading environment message when osEnvReady is false", async () => {
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      osEnvReady: false,
    });
    await waitFor(() => {
      expect(rendered.getByText("Loading env...")).toBeTruthy();
    });
  });

  it("renders voice status in the status bar", async () => {
    const rendered = await renderStatusBar({
      projectPath: "/tmp/project",
      voiceInputEnabled: true,
      voiceInputSupported: true,
      voiceInputAvailable: true,
    });

    await waitFor(() => {
      expect(rendered.getByText("Voice: idle")).toBeTruthy();
    });
  });
});

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
    it("returns ok for usage below 70%", () => {
      expect(usageColorClass(0)).toBe("ok");
      expect(usageColorClass(50)).toBe("ok");
      expect(usageColorClass(69)).toBe("ok");
    });

    it("returns warn for 70-89%", () => {
      expect(usageColorClass(70)).toBe("warn");
      expect(usageColorClass(75)).toBe("warn");
      expect(usageColorClass(89)).toBe("warn");
    });

    it("returns bad for 90% and above", () => {
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
      expect(formatMemory(4831838208)).toBe("4.5");
    });
  });
});
