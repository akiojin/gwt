import { app, BrowserWindow } from 'electron';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSidecar, killSidecar } from './sidecar.js';
import { createMenu } from './menu.js';
import { createTray } from './tray.js';
import { registerIpcHandlers } from './ipc.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const isDev = !app.isPackaged;

let mainWindow: BrowserWindow | null = null;
let sidecarPort: number | null = null;

function createWindow(): BrowserWindow {
  const win = new BrowserWindow({
    width: 1200,
    height: 800,
    minWidth: 800,
    minHeight: 600,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
    show: false,
    titleBarStyle: 'hiddenInset',
  });

  win.once('ready-to-show', () => {
    win.show();
  });

  if (isDev) {
    win.loadURL('http://127.0.0.1:5173');
    win.webContents.openDevTools();
  } else {
    win.loadFile(path.join(__dirname, '..', 'dist', 'index.html'));
  }

  return win;
}

// Single instance lock
const gotTheLock = app.requestSingleInstanceLock();

if (!gotTheLock) {
  app.quit();
} else {
  app.on('second-instance', () => {
    if (mainWindow) {
      if (mainWindow.isMinimized()) mainWindow.restore();
      mainWindow.focus();
    }
  });

  app.whenReady().then(async () => {
    try {
      sidecarPort = await spawnSidecar();
      console.log(`Sidecar started on port ${sidecarPort}`);
    } catch (err) {
      console.error('Failed to start sidecar:', err);
    }

    mainWindow = createWindow();

    createMenu(mainWindow);
    createTray(mainWindow);
    registerIpcHandlers(mainWindow);

    // Pass sidecar port to renderer via webContents
    mainWindow.webContents.once('did-finish-load', () => {
      mainWindow?.webContents.send('sidecar-port', sidecarPort);
    });
  });

  app.on('window-all-closed', () => {
    if (process.platform !== 'darwin') {
      app.quit();
    }
  });

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      mainWindow = createWindow();
    }
  });

  app.on('before-quit', () => {
    killSidecar();
  });
}

export { sidecarPort };
