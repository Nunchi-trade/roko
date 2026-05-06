export type WsStatus = 'idle' | 'connecting' | 'connected' | 'reconnecting' | 'failed';

/** Outbound control messages. */
export interface WsSubscribeMsg {
  type: 'subscribe';
  root: string;
  projections: string[];
}
export interface WsUnsubscribeMsg {
  type: 'unsubscribe';
  root: string;
}
export interface WsResizeMsg {
  type: 'resize';
  cols: number;
  rows: number;
}
export interface WsPingMsg {
  type: 'ping';
}
export type WsOutbound = WsSubscribeMsg | WsUnsubscribeMsg | WsResizeMsg | WsPingMsg;

/** Inbound frames from the server. */
export interface WsFrame {
  type: 'state' | 'delta' | 'ack' | 'error' | 'pong';
  channel?: string;
  cursor?: number;
  workflow_id?: string | null;
  workdir?: string;
  data?: unknown;
  event?: unknown;
  message?: string;
}

export interface WsAdapterConfig {
  /** Full WS URL, e.g. `${WS_BASE}/api/workflow/ws` */
  url: string;
  /** Called on every parsed inbound frame. */
  onFrame: (frame: WsFrame) => void;
  /** Called whenever connection status changes. */
  onStatusChange: (status: WsStatus) => void;
  /** Max reconnect attempts. Default: 5. */
  maxRetries?: number;
  /** Max backoff ms. Default: 15_000. */
  maxBackoffMs?: number;
  /** Ping interval ms. Default: 30_000. Set 0 to disable. */
  pingIntervalMs?: number;
}

export class WsAdapter {
  status: WsStatus;
  /** Map of root -> projections[] currently subscribed. */
  readonly subscriptions: Map<string, string[]>;

  private config: WsAdapterConfig;
  private ws: WebSocket | null;
  private destroyed: boolean;
  private retryCount: number;
  private retryTimer: ReturnType<typeof setTimeout> | null;
  private pingTimer: ReturnType<typeof setInterval> | null;
  private sendQueue: WsOutbound[];

  constructor(config: WsAdapterConfig) {
    this.config = config;
    this.status = 'idle';
    this.subscriptions = new Map();
    this.ws = null;
    this.destroyed = false;
    this.retryCount = 0;
    this.retryTimer = null;
    this.pingTimer = null;
    this.sendQueue = [];
  }

  private setStatus(s: WsStatus): void {
    if (s !== this.status) {
      this.status = s;
      this.config.onStatusChange(s);
    }
  }

  /** Open the WebSocket. Idempotent. */
  connect(): void {
    if (this.destroyed || this.status === 'connected' || this.status === 'connecting') {
      return;
    }
    this.setStatus(this.retryCount === 0 ? 'connecting' : 'reconnecting');

    const ws = new WebSocket(this.config.url);
    this.ws = ws;

    ws.onopen = () => {
      if (this.destroyed || ws !== this.ws) return;
      this.retryCount = 0;
      this.setStatus('connected');

      // Flush queued messages
      for (const msg of this.sendQueue) {
        ws.send(JSON.stringify(msg));
      }
      this.sendQueue.length = 0;

      // Start ping interval
      const pingMs = this.config.pingIntervalMs ?? 30_000;
      if (pingMs !== 0) {
        this.pingTimer = setInterval(() => {
          if (ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({ type: 'ping' }));
          }
        }, pingMs);
      }

      // Re-subscribe to all tracked subscriptions
      for (const [root, projections] of this.subscriptions) {
        ws.send(JSON.stringify({ type: 'subscribe', root, projections }));
      }
    };

    ws.onmessage = (e: MessageEvent) => {
      if (this.destroyed || ws !== this.ws) return;
      try {
        const frame = JSON.parse(e.data as string) as WsFrame;
        this.config.onFrame(frame);
      } catch {
        // skip unparseable frames
      }
    };

    ws.onerror = () => {
      // WebSocket fires onerror then onclose — handle reconnect in onclose only.
      // Suppress: browser DevTools already shows WS errors natively.
    };

    ws.onclose = () => {
      if (this.pingTimer !== null) {
        clearInterval(this.pingTimer);
        this.pingTimer = null;
      }
      if (this.destroyed) {
        this.setStatus('idle');
        return;
      }
      this.retryCount += 1;

      const maxRetries = this.config.maxRetries ?? 5;
      if (this.retryCount > maxRetries) {
        this.setStatus('failed');
        return;
      }

      this.setStatus('reconnecting');
      const maxMs = this.config.maxBackoffMs ?? 15_000;
      const delay = Math.min(1000 * 2 ** (this.retryCount - 1), maxMs);
      this.retryTimer = setTimeout(() => this.connect(), delay);
    };
  }

  /** Send a typed outbound message. Queues if not yet connected. */
  send(msg: WsOutbound): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    } else {
      this.sendQueue.push(msg);
    }
  }

  /** Subscribe to projections for a root. Sends subscribe message + stores in map. */
  subscribe(root: string, projections: string[]): void {
    this.subscriptions.set(root, projections);
    this.send({ type: 'subscribe', root, projections });
  }

  /** Unsubscribe from a root. Sends unsubscribe message + removes from map. */
  unsubscribe(root: string): void {
    this.subscriptions.delete(root);
    this.send({ type: 'unsubscribe', root });
  }

  /** Close connection, cancel reconnects. */
  disconnect(): void {
    if (this.retryTimer !== null) {
      clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
    if (this.pingTimer !== null) {
      clearInterval(this.pingTimer);
      this.pingTimer = null;
    }
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
    this.retryCount = 0;
    this.setStatus('idle');
  }

  /** Close + prevent reconnect permanently. */
  destroy(): void {
    this.destroyed = true;
    this.disconnect();
    this.sendQueue.length = 0;
  }
}
