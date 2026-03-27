import { Menu, type BrowserWindow, type MenuItemConstructorOptions } from 'electron';

export function createMenu(mainWindow: BrowserWindow): void {
  const isMac = process.platform === 'darwin';

  function sendMenuAction(action: string): void {
    mainWindow.webContents.send('menu-action', action);
  }

  const template: MenuItemConstructorOptions[] = [
    ...(isMac
      ? [
          {
            label: 'gwt',
            submenu: [
              { role: 'about' as const },
              { type: 'separator' as const },
              { role: 'services' as const },
              { type: 'separator' as const },
              { role: 'hide' as const },
              { role: 'hideOthers' as const },
              { role: 'unhide' as const },
              { type: 'separator' as const },
              { role: 'quit' as const },
            ],
          },
        ]
      : []),
    {
      label: 'File',
      submenu: [
        {
          label: 'Open Project',
          accelerator: 'CmdOrCtrl+O',
          click: () => sendMenuAction('open-project'),
        },
        {
          label: 'Close Project',
          accelerator: 'CmdOrCtrl+W',
          click: () => sendMenuAction('close-project'),
        },
        { type: 'separator' },
        ...(isMac ? [] : [{ role: 'quit' as const }]),
      ],
    },
    {
      label: 'Edit',
      submenu: [
        { role: 'undo' },
        { role: 'redo' },
        { type: 'separator' },
        { role: 'cut' },
        { role: 'copy' },
        { role: 'paste' },
        { role: 'selectAll' },
      ],
    },
    {
      label: 'View',
      submenu: [
        { role: 'toggleDevTools' },
        { role: 'reload' },
      ],
    },
    {
      label: 'Window',
      submenu: [
        { role: 'minimize' },
        { role: 'zoom' },
      ],
    },
    {
      label: 'Help',
      submenu: [
        {
          label: 'About gwt',
          click: () => sendMenuAction('about'),
        },
      ],
    },
  ];

  const menu = Menu.buildFromTemplate(template);
  Menu.setApplicationMenu(menu);
}
