import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import test from "node:test";

import { Builder, By, Capabilities, until } from "selenium-webdriver";
import { createWindowsTauriFixture } from "./support/fixture.mjs";

const DRIVER_URL =
  process.env.GWT_TAURI_E2E_DRIVER_URL ?? "http://127.0.0.1:4444/";
const DRIVER_PORT = Number(new URL(DRIVER_URL).port || "4444");
const APP_PATH =
  process.env.GWT_TAURI_E2E_APPLICATION ??
  path.resolve("..", "target", "debug", "gwt-tauri.exe");
const TAURI_DRIVER =
  process.env.GWT_TAURI_E2E_TAURI_DRIVER ?? "tauri-driver";
const TEST_TIMEOUT_MS = 90_000;

async function exists(targetPath) {
  try {
    await fs.access(targetPath);
    return true;
  } catch {
    return false;
  }
}

function commandExists(command, args = ["--version"]) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    stdio: "pipe",
  });
  return result.status === 0;
}

async function waitForPort(port, timeoutMs) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/status`);
      if (response.ok) return;
    } catch {
      // Ignore connection errors until timeout.
    }
    await delay(250);
  }
  throw new Error(`Timed out waiting for tauri-driver on port ${port}`);
}

function spawnTauriDriver(env) {
  return spawn(TAURI_DRIVER, [], {
    env,
    stdio: "inherit",
  });
}

async function startDriver(applicationPath, env) {
  const driverProcess = spawnTauriDriver(env);
  try {
    await waitForPort(DRIVER_PORT, 20_000);
    const capabilities = new Capabilities()
      .setBrowserName("wry")
      .set("tauri:options", {
        application: applicationPath,
      });
    const driver = await new Builder()
      .usingServer(DRIVER_URL)
      .withCapabilities(capabilities)
      .build();
    return { driver, driverProcess };
  } catch (error) {
    driverProcess.kill();
    throw error;
  }
}

async function stopDriver(driver, driverProcess) {
  try {
    if (driver) {
      await driver.quit();
    }
  } finally {
    if (driverProcess && driverProcess.pid && !driverProcess.killed) {
      if (process.platform === "win32") {
        spawnSync(
          "taskkill",
          ["/PID", String(driverProcess.pid), "/T", "/F"],
          {
            encoding: "utf8",
            stdio: "ignore",
          },
        );
        await delay(250);
      } else {
        driverProcess.kill();
      }
    }
  }
}

async function waitForElement(driver, locator, timeout = 15_000) {
  await driver.wait(until.elementLocated(locator), timeout);
  const element = await driver.findElement(locator);
  await driver.wait(until.elementIsVisible(element), timeout);
  return element;
}

async function waitForTextElement(driver, text, timeout = 15_000) {
  const locator = By.xpath(`//*[contains(normalize-space(.), "${text}")]`);
  return waitForElement(driver, locator, timeout);
}

async function setSelectValue(driver, id, value) {
  await driver.executeScript(
    ({ elementId, nextValue }) => {
      const element = document.getElementById(elementId);
      if (!(element instanceof HTMLSelectElement)) {
        throw new Error(`Select not found: ${elementId}`);
      }
      element.value = nextValue;
      element.dispatchEvent(new Event("change", { bubbles: true }));
      element.dispatchEvent(new Event("input", { bubbles: true }));
    },
    { elementId: id, nextValue: value },
  );
}

async function readTerminalSnapshot(driver) {
  return driver.executeScript(() => {
    const container = document.querySelector(
      ".terminal-wrapper.active .terminal-container",
    );
    if (!(container instanceof HTMLElement)) return null;

    const terminal = container.__gwtTerminal;
    const active = terminal?.buffer?.active;
    if (!terminal || !active || typeof active.getLine !== "function") {
      return null;
    }

    const lines = [];
    const lineCount = typeof active.length === "number" ? active.length : 0;
    const start = Math.max(0, lineCount - 30);
    for (let index = start; index < lineCount; index += 1) {
      const line = active.getLine(index);
      lines.push(line ? line.translateToString(true) : "");
    }

    return {
      paneId: container.dataset.paneId ?? "",
      rows: terminal.rows ?? -1,
      cols: terminal.cols ?? -1,
      cursorX: active.cursorX ?? -1,
      cursorY: active.cursorY ?? -1,
      lines,
    };
  });
}

async function waitForTerminal(driver, predicate, timeout = 20_000) {
  await driver.wait(async () => {
    const snapshot = await readTerminalSnapshot(driver);
    return snapshot && predicate(snapshot);
  }, timeout);
  return readTerminalSnapshot(driver);
}

async function writeToTerminal(driver, paneId, text) {
  const result = await driver.executeAsyncScript(
    (targetPaneId, payload, done) => {
      const invoke = window.__TAURI_INTERNALS__?.invoke;
      if (typeof invoke !== "function") {
        done({ ok: false, error: "Tauri invoke is unavailable" });
        return;
      }
      const bytes = Array.from(new TextEncoder().encode(payload));
      invoke("write_terminal", { paneId: targetPaneId, data: bytes })
        .then(() => done({ ok: true }))
        .catch((error) => done({ ok: false, error: String(error) }));
    },
    paneId,
    text,
  );

  if (!result?.ok) {
    throw new Error(result?.error ?? "write_terminal failed");
  }
}

test(
  "Windows Tauri E2E launches PowerShell-selected agent via stable wrapper",
  { timeout: TEST_TIMEOUT_MS },
  async (t) => {
    if (process.platform !== "win32") {
      t.skip("Windows-only Tauri WebDriver E2E");
      return;
    }

    if (!(await exists(APP_PATH))) {
      t.skip(`Tauri application binary not found: ${APP_PATH}`);
      return;
    }
    if (!commandExists(TAURI_DRIVER)) {
      t.skip(`tauri-driver is not available on PATH: ${TAURI_DRIVER}`);
      return;
    }
    if (!commandExists("msedgedriver", ["--version"])) {
      t.skip("msedgedriver is not available on PATH");
      return;
    }

    const fixture = createWindowsTauriFixture();
    const homeDir = fixture.homeDir;
    const appDataDir = path.join(homeDir, "AppData", "Roaming");
    const localAppDataDir = path.join(homeDir, "AppData", "Local");
    await fs.mkdir(appDataDir, { recursive: true });
    await fs.mkdir(localAppDataDir, { recursive: true });

    const env = {
      ...process.env,
      HOME: homeDir,
      USERPROFILE: homeDir,
      APPDATA: appDataDir,
      LOCALAPPDATA: localAppDataDir,
      PATH: `${fixture.binDir};${process.env.PATH ?? ""}`,
    };

    let driver;
    let driverProcess;
    try {
      ({ driver, driverProcess } = await startDriver(APP_PATH, env));

      const recentProjectButton = await waitForElement(
        driver,
        By.css("button.recent-item"),
      );
      await recentProjectButton.click();

      const branchItem = await waitForElement(
        driver,
        By.xpath(
          '//*[contains(@class, "branch-item")][contains(., "feature/tauri-e2e")]',
        ),
        20_000,
      );
      await branchItem.click();

      const launchButton = await waitForTextElement(driver, "Launch Agent...");
      await launchButton.click();

      await waitForTextElement(driver, "Launch Agent");
      const advancedButton = await waitForTextElement(driver, "Advanced");
      await advancedButton.click();
      await waitForElement(driver, By.id("shell-select"));
      await setSelectValue(driver, "shell-select", "powershell");

      const launchAction = await waitForElement(
        driver,
        By.xpath(
          '//button[normalize-space(text())="Launch" and not(@disabled)]',
        ),
      );
      await launchAction.click();

      const snapshot = await waitForTerminal(driver, (terminal) => {
        const lines = terminal.lines.map((line) => line.trim());
        return (
          lines.includes("WRAPPER:cmd") &&
          lines.includes("E2E-AGENT-READY>") &&
          !lines.some((line) => line.includes("ABCDEFGHIJKLMNOPQRSTUVWXYZ"))
        );
      });

      assert.ok(snapshot, "terminal snapshot should be available");
      assert.ok(snapshot.paneId.length > 0, "paneId should be populated");
      assert.ok(snapshot.cols > 0, "terminal should report columns");
      assert.ok(snapshot.rows > 0, "terminal should report rows");

      await writeToTerminal(driver, snapshot.paneId, "hello from webdriver\r");
      const echoSnapshot = await waitForTerminal(driver, (terminal) =>
        terminal.lines.some((line) => line.includes("ECHO:hello from webdriver")),
      );
      assert.ok(
        echoSnapshot.lines.some((line) =>
          line.includes("ECHO:hello from webdriver"),
        ),
        "terminal should echo interactive input",
      );
    } finally {
      await stopDriver(driver, driverProcess);
      fixture.cleanup();
    }
  },
);
