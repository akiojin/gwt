import { execa } from "execa";

export interface ClipboardOptions {
  platform?: NodeJS.Platform;
  execa?: typeof execa;
}

export async function copyToClipboard(
  text: string,
  options: ClipboardOptions = {},
): Promise<void> {
  const runner = options.execa ?? execa;
  const platform = options.platform ?? process.platform;

  if (platform === "win32") {
    await runner("cmd", ["/c", "clip"], { input: text, windowsHide: true });
    return;
  }

  if (platform === "darwin") {
    await runner("pbcopy", [], { input: text });
    return;
  }

  try {
    await runner("xclip", ["-selection", "clipboard"], { input: text });
    return;
  } catch {
    await runner("xsel", ["--clipboard", "--input"], { input: text });
  }
}
