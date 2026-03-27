import { ipcMain, dialog, shell, type BrowserWindow } from 'electron';

export function registerIpcHandlers(mainWindow: BrowserWindow): void {
  ipcMain.handle('dialog:openDirectory', async () => {
    const result = await dialog.showOpenDialog(mainWindow, {
      properties: ['openDirectory'],
    });

    if (result.canceled || result.filePaths.length === 0) {
      return null;
    }

    return result.filePaths[0];
  });

  ipcMain.handle(
    'dialog:showMessage',
    async (_event, options: { message: string; type: string }) => {
      const result = await dialog.showMessageBox(mainWindow, {
        message: options.message,
        type: options.type as 'none' | 'info' | 'error' | 'question' | 'warning',
      });

      return result.response;
    },
  );

  ipcMain.handle('shell:openExternal', async (_event, url: string) => {
    await shell.openExternal(url);
  });

  ipcMain.on('window:setTitle', (_event, title: string) => {
    mainWindow.setTitle(title);
  });

  ipcMain.on('window:minimize', () => {
    mainWindow.minimize();
  });

  ipcMain.on('window:maximize', () => {
    if (mainWindow.isMaximized()) {
      mainWindow.unmaximize();
    } else {
      mainWindow.maximize();
    }
  });

  ipcMain.on('window:close', () => {
    mainWindow.close();
  });
}
