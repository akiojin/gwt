import { execa } from "execa";
import { createRequire } from "node:module";
import { createLogger } from "../../logging/logger.js";

const require = createRequire(import.meta.url);

/**
 * URL を開く関数の型定義
 * @param url - 開くURL
 */
export type OpenUrlFn = (url: string) => Promise<void> | void;

// PNG icon (16x16) for macOS/Linux - Base64 encoded
const TRAY_ICON_PNG_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAABHNCSVQICAgIfAhkiAAAAAlwSFlzAAAAdgAAAHYBTnsmCAAAABl0RVh0U29mdHdhcmUAd3d3Lmlua3NjYXBlLm9yZ5vuPBoAAADLSURBVDiNpZMxDoJAEEXfLhYewMrOxsLKG3gDj+ANPIKFtScwHoHOxmN4AysLL0BlQ0Ji4W6yLLAgfslkZnbm/0wmA/9G6CogIr4F7IAbcAYOqpoEJBYD+3UADVYAiMgZOAKvboAFwFLENoAtsAdGwAw4qepZRN7A2LIIWCngCjy7t1bVF/D05QBT4K6qOxH5dOwjuwrYApdMvC3wBHzWBTwAr6p6c+wFSBJvEeDsehLVNwZ0sQDqv+kl3xjQxcKNSaZ/s+irNK1fXfwA7LE/RA3w5ggAAAAASUVORK5CYII=";

// ICO icon for Windows - Base64 encoded
const TRAY_ICON_ICO_BASE64 =
  "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQBAMAAADt3eJSAAAABGdBTUEAALGPC/xhBQAAACBjSFJNAAB6JgAAgIQAAPoAAACA6AAAdTAAAOpgAAA6mAAAF3CculE8AAAAJ1BMVEUAAAAAvNQAvNQAvNQAvNQAvNQAvNQAvNQAvNQAvNQAvNQAvNT////J1ubyAAAAC3RSTlMAJYTcgyQJnJ3U3WXfUogAAAABYktHRAyBs1FjAAAAB3RJTUUH6QwMCRccbOpRBQAAAFdJREFUCNdjYEAARuXNriCarXr37t1tQEYmkN69M4GBoRvE2F3AwAqmd29kYIEwNjEwQxibGbghjN0MXDARJpgaqK6tDAzVUHMQJoPtKgPZyuq8S5WBAQBeRj51tvdhawAAACV0RVh0ZGF0ZTpjcmVhdGUAMjAyNS0xMi0xMlQwOToyMzoyOCswMDowMBPEA5UAAAAldEVYdGRhdGU6bW9kaWZ5ADIwMjUtMTItMTJUMDk6MjM6MjgrMDA6MDBimbspAAAAAElFTkSuQmCC";

let trayInitAttempted = false;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
let trayInstance: any = null;

function shouldEnableTray(
  platform: NodeJS.Platform = process.platform,
): boolean {
  // NOTE: `trayicon` is a win32-only dependency.
  if (platform !== "win32") return false;
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
 * @param url - Web UI のURL（メニューから開く）
 * @param opts - オプション設定
 * @param opts.openUrl - URL を開くカスタム関数（テスト用）
 */
export async function startSystemTray(
  url: string,
  opts?: { openUrl?: OpenUrlFn; platform?: NodeJS.Platform },
): Promise<void> {
  if (trayInitAttempted || !shouldEnableTray(opts?.platform)) return;
  trayInitAttempted = true;

  const logger = createLogger({ category: "tray" });
  const open = opts?.openUrl ?? openUrl;

  try {
    // systray2 をCommonJS requireでインポート（ESM互換性のため）
    const SysTray = require("systray2").default;

    const isWindows = process.platform === "win32";
    const isMacOS = process.platform === "darwin";

    // プラットフォームに応じたアイコンを選択
    const iconBase64 = isWindows ? TRAY_ICON_ICO_BASE64 : TRAY_ICON_PNG_BASE64;

    const openDashboardItem = {
      title: "Open Web UI",
      tooltip: "Open gwt Web UI in browser",
      checked: false,
      enabled: true,
    };

    const exitItem = {
      title: "Exit",
      tooltip: "Close the tray icon",
      checked: false,
      enabled: true,
    };

    trayInstance = new SysTray({
      menu: {
        icon: iconBase64,
        isTemplateIcon: isMacOS,
        title: "gwt",
        tooltip: `gwt Web UI - ${url}`,
        items: [openDashboardItem, SysTray.separator, exitItem],
      },
      debug: false,
      copyDir: false,
    });

    trayInstance.onClick((action: { item: { title: string } }) => {
      if (action.item.title === "Open Web UI") {
        open(url);
      } else if (action.item.title === "Exit") {
        trayInstance?.kill(false);
        trayInstance = null;
      }
    });

    await trayInstance.ready();
    logger.info("System tray initialized");
  } catch (err) {
    logger.warn({ err }, "System tray failed to initialize");
  }
}

/**
 * システムトレイアイコンを破棄
 */
export function disposeSystemTray(): void {
  try {
    trayInstance?.kill?.(false);
  } catch {
    // Ignore errors during cleanup
  }
  trayInstance = null;
  trayInitAttempted = false;
}
