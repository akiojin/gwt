import { execa } from "execa";
import { createLogger } from "../../logging/logger.js";

export type OpenUrlFn = (url: string) => Promise<void> | void;

const TRAY_ICON_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQBAMAAADt3eJSAAAABGdBTUEAALGPC/xhBQAAACBjSFJNAAB6JgAAgIQAAPoAAACA6AAAdTAAAOpgAAA6mAAAF3CculE8AAAAJ1BMVEUAAAAAvNQAvNQAvNQAvNQAvNQAvNQAvNQAvNQAvNQAvNQAvNT////J1ubyAAAAC3RSTlMAJYTcgyQJnJ3U3WXfUogAAAABYktHRAyBs1FjAAAAB3RJTUUH6QwMCRccbOpRBQAAAFdJREFUCNdjYEAARuXNriCarXr37t1tQEYmkN69M4GBoRvE2F3AwAqmd29kYIEwNjEwQxibGbghjN0MXDARJpgaqK6tDAzVUHMQJoPtKgPZyuq8S5WBAQBeRj51tvdhawAAACV0RVh0ZGF0ZTpjcmVhdGUAMjAyNS0xMi0xMlQwOToyMzoyOCswMDowMBPEA5UAAAAldEVYdGRhdGU6bW9kaWZ5ADIwMjUtMTItMTJUMDk6MjM6MjgrMDA6MDBimbspAAAAAElFTkSuQmCC";

let trayInitialized = false;

function shouldEnableTray(): boolean {
  if (process.env.GWT_DISABLE_TRAY?.toLowerCase() === "true") return false;
  if (process.env.GWT_DISABLE_TRAY === "1") return false;
  if (process.env.CI) return false;
  if (process.platform === "linux") {
    if (!process.env.DISPLAY && !process.env.WAYLAND_DISPLAY) return false;
  }
  return true;
}

export async function openUrl(url: string): Promise<void> {
  const platform = process.platform;
  try {
    if (platform === "win32") {
      await execa("cmd", ["/c", "start", "", url], {
        windowsHide: true,
        shell: true,
      });
      return;
    }
    if (platform === "darwin") {
      await execa("open", [url]);
      return;
    }
    await execa("xdg-open", [url], { stdio: "ignore" });
  } catch {
    // Ignore errors to avoid disrupting CLI/Web UI
  }
}

export async function startSystemTray(
  url: string,
  opts?: { openUrl?: OpenUrlFn },
): Promise<void> {
  if (trayInitialized || !shouldEnableTray()) return;
  trayInitialized = true;

  const logger = createLogger({ category: "tray" });

  try {
    const mod = (await import("trayicon")) as unknown as {
      create?: (options: {
        icon: Buffer;
        title?: string;
        tooltip?: string;
        action?: () => unknown;
      }) => unknown;
      default?: unknown;
    };
    const create =
      (mod as any).create ??
      ((mod as any).default && (mod as any).default.create);
    if (typeof create !== "function") {
      throw new Error("trayicon.create not available");
    }

    const icon = Buffer.from(TRAY_ICON_BASE64, "base64");
    const open = opts?.openUrl ?? openUrl;

    create({
      icon,
      title: "gwt Web UI",
      tooltip: "Double-click to open Web UI",
      action: async () => {
        await open(url);
      },
    });
  } catch (err) {
    logger.warn({ err }, "System tray failed to initialize");
  }
}

