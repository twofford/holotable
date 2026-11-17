// The client's WebSocket connection.
//
// This is a plain class, not a React component. React reads snapshots of its
// state (see `useConnection` in App.tsx); scene code subscribes to its messages
// directly (see `scene/sceneStore.ts`), without going through React at all.

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

type MessageListener = (msg: ServerMsg) => void;

const MAX_LOG = 50;
/** Reconnect delays: 1 s, doubling up to 10 s, with some randomness (SYN-007). */
const RECONNECT_FIRST_MS = 1000;
const RECONNECT_MAX_MS = 10_000;
/** Message types too frequent to be worth showing in the connection log. */
const UNLOGGED = new Set(["drag", "tokenDragging"]);

export class Connection {
  private socket: WebSocket | null = null;
  private listeners = new Set<() => void>();
  private messageListeners = new Set<MessageListener>();
  private openListeners = new Set<() => void>();
  private nextNonce = 1;
  private nextLogId = 1;
  private pendingPings = new Map<number, number>(); // nonce -> performance.now()
  private reconnectTimer: number | undefined;
  private reconnectDelay = RECONNECT_FIRST_MS;
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

  // --- For code that talks to the server over this connection ---

  /** Calls `listener` with every message from the server. Returns an unsubscribe function. */
  onMessage(listener: MessageListener): () => void {
    this.messageListeners.add(listener);
    return () => this.messageListeners.delete(listener);
  }

  /** Calls `listener` each time the connection (re)opens, e.g. to rejoin a scene. */
  onOpen(listener: () => void): () => void {
    this.openListeners.add(listener);
    return () => this.openListeners.delete(listener);
  }

  get isOpen(): boolean {
    return this.socket?.readyState === WebSocket.OPEN;
  }

  /** Sends a message if the connection is open. Returns whether it was sent. */
  send(msg: ClientMsg): boolean {
    if (this.socket?.readyState !== WebSocket.OPEN) return false;
    const text = JSON.stringify(msg);
    this.socket.send(text);
    if (!UNLOGGED.has(msg.type)) this.addLog("out", text);
    return true;
  }

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

    socket.addEventListener("open", () => {
      this.reconnectDelay = RECONNECT_FIRST_MS;
      this.update({ status: "open" });
      for (const listener of this.openListeners) listener();
    });
    socket.addEventListener("message", (event) => this.receive(event.data));
    socket.addEventListener("close", () => {
      if (this.socket !== socket) return; // An old socket we already replaced.
      this.socket = null;
      this.pendingPings.clear();
      this.update({ status: "closed" });
      if (!this.stopped) {
        // Back off, so a server that's down isn't hammered by every client at
        // once. The random part spreads clients out after a restart.
        const delay = this.reconnectDelay * (0.75 + Math.random() * 0.5);
        this.reconnectDelay = Math.min(this.reconnectDelay * 2, RECONNECT_MAX_MS);
        this.reconnectTimer = window.setTimeout(() => this.open(), delay);
      }
    });
  }

  private receive(data: unknown): void {
    if (typeof data !== "string") return;

    // The server is trusted to send valid ServerMsg JSON: its types are generated
    // from the same Rust enum. A parse error still must not crash the client.
    let msg: ServerMsg;
    try {
      msg = JSON.parse(data) as ServerMsg;
    } catch {
      return;
    }
    if (!UNLOGGED.has(msg.type)) this.addLog("in", data);

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
      // Scene messages: handled by the listeners below (the scene store).
      case "sceneSnapshot":
      case "event":
      case "ack":
      case "reject":
      case "tokenDragging":
      case "kicked":
      case "sceneChanged":
      case "activeScene":
        break;
      default: {
        // If a new variant is added in Rust and not handled here, this line
        // stops compiling: `msg` would no longer be `never`.
        const unhandled: never = msg;
        console.warn("unknown message", unhandled);
      }
    }
    for (const listener of this.messageListeners) listener(msg);
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
