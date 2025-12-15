import { execa } from "execa";
import { createLogger } from "../../logging/logger.js";

/**
 * URL を開く関数の型定義
 * @param url - 開くURL
 */
export type OpenUrlFn = (url: string) => Promise<void> | void;

const TRAY_ICON_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQBAMAAADt3eJSAAAABGdBTUEAALGPC/xhBQAAACBjSFJNAAB6JgAAgIQAAPoAAACA6AAAdTAAAOpgAAA6mAAAF3CculE8AAAAJ1BMVEUAAAAAvNQAvNQAvNQAvNQAvNQAvNQAvNQAvNQAvNQAvNQAvNT////J1ubyAAAAC3RSTlMAJYTcgyQJnJ3U3WXfUogAAAABYktHRAyBs1FjAAAAB3RJTUUH6QwMCRccbOpRBQAAAFdJREFUCNdjYEAARuXNriCarXr37t1tQEYmkN69M4GBoRvE2F3AwAqmd29kYIEwNjEwQxibGbghjN0MXDARJpgaqK6tDAzVUHMQJoPtKgPZyuq8S5WBAQBeRj51tvdhawAAACV0RVh0ZGF0ZTpjcmVhdGUAMjAyNS0xMi0xMlQwOToyMzoyOCswMDowMBPEA5UAAAAldEVYdGRhdGU6bW9kaWZ5ADIwMjUtMTItMTJUMDk6MjM6MjgrMDA6MDBimbspAAAAAElFTkSuQmCC";

let trayInitAttempted = false;
type TrayHandle = { dispose?: () => void; kill?: () => void };
let trayInstance: TrayHandle | null = null;
let trayInitPromise: Promise<TrayHandle> | null = null;

function shouldEnableTray(): boolean {
  // NOTE: `trayicon` is a win32-only dependency.
  if (process.platform !== "win32") return false;
  if (process.env.GWT_DISABLE_TRAY?.toLowerCase() === "true") return false;
  if (process.env.GWT_DISABLE_TRAY === "1") return false;
  if (process.env.CI) return false;
  return true;
}

/**
 * デフォルトブラウザでURLを開く
 * @param url - 開くURL
 */
export async function openUrl(url: string): Promise<void> {
  const platform = process.platform;
  try {
    if (platform === "win32") {
      await execa("explorer.exe", [url], { windowsHide: true });
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

/**
 * システムトレイアイコンを初期化
 * @param url - Web UI のURL（ダブルクリック時に開く）
 * @param opts - オプション設定
 * @param opts.openUrl - URL を開くカスタム関数（テスト用）
 */
export async function startSystemTray(
  url: string,
  opts?: { openUrl?: OpenUrlFn },
): Promise<void> {
  if (trayInitAttempted || !shouldEnableTray()) return;
  trayInitAttempted = true;

  const logger = createLogger({ category: "tray" });

  try {
    const mod = (await import("trayicon")) as Record<string, unknown> & {
      default?: Record<string, unknown>;
    };
    const create = mod.create ?? mod.default?.create;
    if (typeof create !== "function") {
      throw new Error("trayicon.create not available");
    }

    const icon = Buffer.from(TRAY_ICON_BASE64, "base64");
    const open = opts?.openUrl ?? openUrl;

    const initPromise = Promise.resolve(
      create({
        icon,
        title: "gwt Web UI",
        tooltip: "Double-click to open Web UI",
        action: async () => {
          await open(url);
        },
      }) as TrayHandle,
    );
    trayInitPromise = initPromise;

    void initPromise
      .then((tray) => {
        if (trayInitPromise !== initPromise) {
          tray.dispose?.();
          tray.kill?.();
          return;
        }
        trayInstance = tray;
      })
      .catch((err) => {
        if (trayInitPromise !== initPromise) return;
        logger.warn({ err }, "System tray failed to initialize");
      });
  } catch (err) {
    logger.warn({ err }, "System tray failed to initialize");
  }
}

/**
 * システムトレイアイコンを破棄
 */
export function disposeSystemTray(): void {
  trayInitPromise = null;

  const instance = trayInstance;
  trayInstance = null;

  instance?.dispose?.();
  instance?.kill?.();
}
