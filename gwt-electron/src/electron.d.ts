export interface ElectronAPI {
  sidecarPort: number;
  appVersion: string;
  platform: NodeJS.Platform;
  dialog: {
    openDirectory(): Promise<string | null>;
    showMessage(options: { message: string; type: string }): Promise<number>;
  };
  shell: {
    openExternal(url: string): Promise<void>;
  };
  window: {
    setTitle(title: string): void;
    minimize(): void;
    maximize(): void;
    close(): void;
  };
  onMenuAction(callback: (action: string) => void): () => void;
}

declare global {
  interface Window {
    electronAPI: ElectronAPI;
  }
}
