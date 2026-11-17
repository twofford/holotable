// The client's WebSocket connection.
//
// This is a plain class, not a React component. React only reads snapshots of its
// state (see `useConnection` in App.tsx). Later the map canvas will read the same
// connection without going through React at all.

import type { ClientMsg } from "../protocol/generated/ClientMsg";
import type { ServerMsg } from "../protocol/generated/ServerMsg";

export type Status = "connecting" | "open" | "closed";

export interface LogEntry {
  id: number;
  at: Date;
  direction: "in" | "out";
  text: string;
}

// `readonly` stops accidental mutation. Each change makes a new snapshot object,
// which is what lets React notice the change.
export interface ConnectionState {
  readonly status: Status;
  readonly serverVersion: string | null;
  readonly lastRttMs: number | null;
  readonly log: readonly LogEntry[];
}

const MAX_LOG = 50;
const RECONNECT_MS = 2000;

export class Connection {
  private socket: WebSocket | null = null;
  private listeners = new Set<() => void>();
  private nextNonce = 1;
  private nextLogId = 1;
  private pendingPings = new Map<number, number>(); // nonce -> performance.now()
  private reconnectTimer: number | undefined;
  private stopped = false;

  private state: ConnectionState = {
    status: "closed",
    serverVersion: null,
    lastRttMs: null,
    log: [],
  };

  constructor(private readonly url: string) {}

  /** Builds the /ws URL from the page's own address, so it works in dev and production. */
  static forCurrentPage(): Connection {
    const scheme = location.protocol === "https:" ? "wss" : "ws";
    return new Connection(`${scheme}://${location.host}/ws`);
  }

  // --- Subscription API used by React's useSyncExternalStore ---

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): ConnectionState => this.state;

  // --- Lifecycle ---

  start(): void {
    this.stopped = false;
    this.open();
  }

  stop(): void {
    this.stopped = true;
    window.clearTimeout(this.reconnectTimer);
    this.socket?.close();
    this.socket = null;
  }

  sendPing(): void {
    const nonce = this.nextNonce++;
    this.pendingPings.set(nonce, performance.now());
    this.send({ type: "ping", nonce });
  }

  // --- Internals ---

  private open(): void {
    this.update({ status: "connecting" });
    const socket = new WebSocket(this.url);
    this.socket = socket;

    socket.addEventListener("open", () => this.update({ status: "open" }));
    socket.addEventListener("message", (event) => this.receive(event.data));
    socket.addEventListener("close", () => {
      if (this.socket !== socket) return; // An old socket we already replaced.
      this.socket = null;
      this.pendingPings.clear();
      this.update({ status: "closed" });
      if (!this.stopped) {
        this.reconnectTimer = window.setTimeout(() => this.open(), RECONNECT_MS);
      }
    });
  }

  private send(msg: ClientMsg): void {
    if (this.socket?.readyState !== WebSocket.OPEN) return;
    const text = JSON.stringify(msg);
    this.socket.send(text);
    this.addLog("out", text);
  }

  private receive(data: unknown): void {
    if (typeof data !== "string") return;
    this.addLog("in", data);

    // The server is trusted to send valid ServerMsg JSON: its types are generated
    // from the same Rust enum. A parse error still must not crash the client.
    let msg: ServerMsg;
    try {
      msg = JSON.parse(data) as ServerMsg;
    } catch {
      return;
    }

    // `msg.type` narrows the union: inside each case, TypeScript knows which
    // fields exist.
    switch (msg.type) {
      case "welcome":
        this.update({ serverVersion: msg.serverVersion });
        break;
      case "ping":
        this.send({ type: "pong", nonce: msg.nonce });
        break;
      case "pong": {
        const sentAt = this.pendingPings.get(msg.nonce);
        if (sentAt !== undefined) {
          this.pendingPings.delete(msg.nonce);
          this.update({ lastRttMs: Math.round(performance.now() - sentAt) });
        }
        break;
      }
      case "error":
        console.warn("server error:", msg.message);
        break;
      default: {
        // If a new variant is added in Rust and not handled here, this line
        // stops compiling: `msg` would no longer be `never`.
        const unhandled: never = msg;
        console.warn("unknown message", unhandled);
      }
    }
  }

  private addLog(direction: LogEntry["direction"], text: string): void {
    const entry: LogEntry = { id: this.nextLogId++, at: new Date(), direction, text };
    this.update({ log: [entry, ...this.state.log].slice(0, MAX_LOG) });
  }

  private update(patch: Partial<ConnectionState>): void {
    this.state = { ...this.state, ...patch };
    for (const listener of this.listeners) listener();
  }
}
