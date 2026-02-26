export interface ToastEvent {
  message: string;
  durationMs?: number;
}

type ToastHandler = (event: ToastEvent) => void;

class ToastBus {
  private handlers: ToastHandler[] = [];

  subscribe(handler: ToastHandler): () => void {
    this.handlers.push(handler);
    return () => {
      this.handlers = this.handlers.filter((h) => h !== handler);
    };
  }

  emit(event: ToastEvent): void {
    for (const handler of this.handlers) {
      handler(event);
    }
  }
}

export const toastBus = new ToastBus();
