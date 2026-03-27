import { contextBridge, ipcRenderer } from 'electron';

let sidecarPort = 0;

ipcRenderer.on('sidecar-port', (_event, port: number) => {
  sidecarPort = port;
});

contextBridge.exposeInMainWorld('electronAPI', {
  get sidecarPort() {
    return sidecarPort;
  },
  appVersion: '', // Will be set after app ready
  platform: process.platform,

  dialog: {
    openDirectory(): Promise<string | null> {
      return ipcRenderer.invoke('dialog:openDirectory');
    },
    showMessage(options: { message: string; type: string }): Promise<number> {
      return ipcRenderer.invoke('dialog:showMessage', options);
    },
  },

  shell: {
    openExternal(url: string): Promise<void> {
      return ipcRenderer.invoke('shell:openExternal', url);
    },
  },

  window: {
    setTitle(title: string): void {
      ipcRenderer.send('window:setTitle', title);
    },
    minimize(): void {
      ipcRenderer.send('window:minimize');
    },
    maximize(): void {
      ipcRenderer.send('window:maximize');
    },
    close(): void {
      ipcRenderer.send('window:close');
    },
  },

  onMenuAction(callback: (action: string) => void): () => void {
    const handler = (_event: Electron.IpcRendererEvent, action: string) => {
      callback(action);
    };
    ipcRenderer.on('menu-action', handler);
    return () => {
      ipcRenderer.removeListener('menu-action', handler);
    };
  },
});
