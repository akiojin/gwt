import { describe, it, expect, mock } from "bun:test";
import { copyToClipboard } from "../../utils/clipboard.js";

const execaMock = mock();

mock.module("execa", () => ({
  execa: (...args: unknown[]) => execaMock(...args),
}));

describe("copyToClipboard", () => {
  it("uses pbcopy on darwin", async () => {
    execaMock.mockResolvedValue({ stdout: "" });

    await copyToClipboard("hello", {
      platform: "darwin",
      execa: execaMock as unknown as typeof import("execa").execa,
    });

    expect(execaMock).toHaveBeenCalledWith(
      "pbcopy",
      [],
      expect.objectContaining({ input: "hello" }),
    );
  });

  it("falls back to xsel when xclip fails", async () => {
    execaMock.mockImplementation((command: string) => {
      if (command === "xclip") {
        return Promise.reject(new Error("missing"));
      }
      return Promise.resolve({ stdout: "" });
    });

    await copyToClipboard("hello", {
      platform: "linux",
      execa: execaMock as unknown as typeof import("execa").execa,
    });

    expect(execaMock).toHaveBeenCalledWith(
      "xclip",
      ["-selection", "clipboard"],
      expect.objectContaining({ input: "hello" }),
    );
    expect(execaMock).toHaveBeenCalledWith(
      "xsel",
      ["--clipboard", "--input"],
      expect.objectContaining({ input: "hello" }),
    );
  });

  it("uses clip on windows", async () => {
    execaMock.mockResolvedValue({ stdout: "" });

    await copyToClipboard("hello", {
      platform: "win32",
      execa: execaMock as unknown as typeof import("execa").execa,
    });

    expect(execaMock).toHaveBeenCalledWith(
      "cmd",
      ["/c", "clip"],
      expect.objectContaining({ input: "hello" }),
    );
  });
});
