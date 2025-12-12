declare module "trayicon" {
  export interface TrayIconMenuItem {
    text: string;
    action?: () => unknown;
    disabled?: boolean;
    checked?: boolean;
  }

  export interface TrayIconOptions {
    icon: Buffer;
    title?: string;
    tooltip?: string;
    action?: () => unknown;
    menu?: TrayIconMenuItem[];
  }

  export interface TrayIconInstance {
    setIcon?: (icon: Buffer) => void;
    setTitle?: (title: string) => void;
    dispose?: () => void;
  }

  export function create(options: TrayIconOptions): TrayIconInstance;

  const trayicon: {
    create: typeof create;
  };

  export default trayicon;
}
