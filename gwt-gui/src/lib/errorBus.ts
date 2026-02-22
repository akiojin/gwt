export interface StructuredError {
  severity: "info" | "warning" | "error" | "critical";
  code: string;
  message: string;
  command: string;
  category: string;
  suggestions: string[];
  timestamp: string;
}

type ErrorHandler = (error: StructuredError) => void;

class ErrorBus {
  private handlers: ErrorHandler[] = [];
  private sessionFingerprints = new Set<string>();

  subscribe(handler: ErrorHandler): () => void {
    this.handlers.push(handler);
    return () => {
      this.handlers = this.handlers.filter((h) => h !== handler);
    };
  }

  emit(error: StructuredError): void {
    if (this.isSuppressed(error)) return;
    this.sessionFingerprints.add(this.fingerprint(error));
    for (const handler of this.handlers) {
      handler(error);
    }
  }

  isSuppressed(error: StructuredError): boolean {
    return this.sessionFingerprints.has(this.fingerprint(error));
  }

  private fingerprint(error: StructuredError): string {
    return `${error.code}::${error.command}`;
  }

  /** Reset suppression state (for testing) */
  resetSession(): void {
    this.sessionFingerprints.clear();
  }
}

export const errorBus = new ErrorBus();
