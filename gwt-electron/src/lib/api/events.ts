/**
 * WebSocket event client for receiving server-pushed events.
 *
 * Design rules (SPEC-1776):
 * - Single WS connection, multiplexed by event type
 * - Auto-reconnect with exponential backoff
 * - Text frames for structured JSON events
 * - Binary frames for terminal output (future)
 */

type EventCallback = (payload: unknown) => void;

class EventClient {
  private ws: WebSocket | null = null;
  private listeners = new Map<string, Set<EventCallback>>();
  private reconnectAttempt = 0;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private port: number | null = null;
  private disposed = false;

  private static readonly BACKOFF_BASE_MS = 500;
  private static readonly BACKOFF_MAX_MS = 8000;

  /** Connect to the gwt-server WebSocket endpoint. */
  connect(port: number): void {
    this.port = port;
    this.disposed = false;
    this.doConnect();
  }

  /** Disconnect and stop reconnecting. */
  disconnect(): void {
    this.disposed = true;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
  }

  /** Subscribe to a server event. Returns an unsubscribe function. */
  on(event: string, callback: EventCallback): () => void {
    let set = this.listeners.get(event);
    if (!set) {
      set = new Set();
      this.listeners.set(event, set);
    }
    set.add(callback);

    return () => {
      set!.delete(callback);
      if (set!.size === 0) {
        this.listeners.delete(event);
      }
    };
  }

  /** Remove a specific listener. */
  off(event: string, callback: EventCallback): void {
    const set = this.listeners.get(event);
    if (set) {
      set.delete(callback);
      if (set.size === 0) {
        this.listeners.delete(event);
      }
    }
  }

  /** Whether the WebSocket is currently connected. */
  get connected(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }

  private doConnect(): void {
    if (this.disposed || this.port === null) return;

    const url = `ws://127.0.0.1:${this.port}/ws`;
    const ws = new WebSocket(url);

    ws.onopen = () => {
      this.reconnectAttempt = 0;
      this.dispatch("_connected", null);
    };

    ws.onmessage = (event) => {
      if (typeof event.data === "string") {
        try {
          const msg = JSON.parse(event.data) as {
            event: string;
            pane_id?: string;
            payload: unknown;
          };
          this.dispatch(msg.event, msg.payload);
        } catch {
          // Ignore malformed messages.
        }
      }
      // Binary frames (terminal output) handled in future phase.
    };

    ws.onclose = () => {
      this.ws = null;
      this.dispatch("_disconnected", null);
      this.scheduleReconnect();
    };

    ws.onerror = () => {
      // onclose will fire after onerror.
    };

    this.ws = ws;
  }

  private scheduleReconnect(): void {
    if (this.disposed) return;

    const delay = Math.min(
      EventClient.BACKOFF_MAX_MS,
      EventClient.BACKOFF_BASE_MS * Math.pow(2, this.reconnectAttempt),
    );
    this.reconnectAttempt++;

    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.doConnect();
    }, delay);
  }

  private dispatch(event: string, payload: unknown): void {
    const set = this.listeners.get(event);
    if (set) {
      for (const cb of set) {
        try {
          cb(payload);
        } catch {
          // Don't let one listener crash others.
        }
      }
    }
  }
}

/** Singleton event client. */
export const events = new EventClient();
