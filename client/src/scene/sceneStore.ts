// The client's copy of one scene's live state.
//
// It keeps two layers:
// - `confirmed`: tokens exactly as the server last described them;
// - `pending`: my own changes that the server hasn't answered yet.
//
// What the page shows is `confirmed` with `pending` laid on top, so my own
// moves appear at once (optimistic updates, SYN-004). When the server answers,
// the pending change is dropped: on `Ack` the matching event has already
// updated `confirmed`; on `Reject`, dropping it puts the token back where the
// server says it is.

import type { ClientMsg } from "../protocol/generated/ClientMsg";
import type { DoorState } from "../protocol/generated/DoorState";
import type { Role } from "../protocol/generated/Role";
import type { SceneEvent } from "../protocol/generated/SceneEvent";
import type { SceneOp } from "../protocol/generated/SceneOp";
import type { ServerMsg } from "../protocol/generated/ServerMsg";
import type { Token } from "../protocol/generated/Token";
import type { TokenSettings } from "../protocol/generated/TokenSettings";

/** What the store needs from a connection. `Connection` has all of it; tests pass a fake. */
export interface SceneLink {
  send(msg: ClientMsg): boolean;
  onMessage(listener: (msg: ServerMsg) => void): () => void;
  onOpen(listener: () => void): () => void;
  readonly isOpen: boolean;
}

export interface RemoteDrag {
  readonly x: number;
  readonly y: number;
  readonly userId: number;
  readonly at: number;
}

/** A player's explored fog: one bit per cell (see `vision::ExploredLayout`). */
export interface ExploredFog {
  readonly cell: number;
  readonly cols: number;
  readonly rows: number;
  /** Changed in place as the player's tokens see more. */
  readonly bits: Uint8Array;
}

export interface SceneView {
  readonly status: "joining" | "live" | "kicked";
  readonly role: Role | null;
  readonly tokens: readonly Token[];
  /** Every door's state, by its index in the map's portals. */
  readonly doors: readonly DoorState[];
  /** Players only: what they've explored. `null` for GMs. */
  readonly explored: ExploredFog | null;
  /** Tokens other people are dragging right now, by token id. */
  readonly dragging: ReadonlyMap<number, RemoteDrag>;
  readonly kicked: { reason: string; rejoin: boolean } | null;
  /** The last refusal from the server, to show the user. */
  readonly notice: string | null;
}

type Pending =
  | { kind: "move"; tokenId: number; x: number; y: number }
  | { kind: "update"; tokenId: number; settings: TokenSettings }
  | { kind: "remove"; tokenId: number }
  | { kind: "create" }
  /** Doors and fog: shown when the server's event arrives, not before. */
  | { kind: "other" };

/** At most one drag update every 50 ms: 20 a second (SYN-006). */
export const DRAG_INTERVAL_MS = 50;
/** Forget someone else's drag if no update arrives for this long. */
const DRAG_STALE_MS = 1500;

export class SceneStore {
  private confirmed = new Map<number, Token>();
  private pending = new Map<number, Pending>();
  private dragging = new Map<number, RemoteDrag>();
  private epoch: string | null = null;
  private seq = 0;
  private nextReq = 1;
  private listeners = new Set<() => void>();
  private unsubscribe: (() => void)[] = [];
  private dragTimer: ReturnType<typeof setTimeout> | undefined;
  private queuedDrag: { tokenId: number; x: number; y: number } | null = null;
  private lastDragSent = 0;
  private staleTimer: ReturnType<typeof setInterval> | undefined;

  private view: SceneView = {
    status: "joining",
    role: null,
    tokens: [],
    doors: [],
    explored: null,
    dragging: new Map(),
    kicked: null,
    notice: null,
  };

  constructor(
    private readonly link: SceneLink,
    readonly sceneId: number,
    private readonly onSceneChanged: () => void = () => {},
    private readonly now: () => number = () => Date.now(),
  ) {}

  // --- React's useSyncExternalStore ---

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): SceneView => this.view;

  // --- Lifecycle ---

  start(): void {
    this.unsubscribe = [
      this.link.onMessage((msg) => this.receive(msg)),
      // After a reconnect, join again and catch up (SYN-007). Changes that
      // weren't answered before the drop are forgotten: if the server applied
      // them, their events arrive with the catch-up.
      this.link.onOpen(() => {
        this.pending.clear();
        this.join();
      }),
    ];
    if (this.link.isOpen) this.join();
    this.staleTimer = setInterval(() => this.dropStaleDrags(), 500);
  }

  stop(): void {
    for (const unsubscribe of this.unsubscribe) unsubscribe();
    this.unsubscribe = [];
    clearTimeout(this.dragTimer);
    clearInterval(this.staleTimer);
    this.link.send({ type: "leaveScene" });
  }

  // --- Actions ---

  move(tokenId: number, x: number, y: number): void {
    // A final move supersedes any drag update still waiting to be sent.
    clearTimeout(this.dragTimer);
    this.queuedDrag = null;
    this.sendOp({ type: "moveToken", tokenId, x, y }, { kind: "move", tokenId, x, y });
  }

  create(settings: TokenSettings, x: number, y: number): void {
    this.sendOp({ type: "createToken", settings, x, y }, { kind: "create" });
  }

  update(tokenId: number, settings: TokenSettings): void {
    this.sendOp({ type: "updateToken", tokenId, settings }, { kind: "update", tokenId, settings });
  }

  remove(tokenId: number): void {
    this.sendOp({ type: "deleteToken", tokenId }, { kind: "remove", tokenId });
  }

  setDoor(door: number, open: boolean): void {
    this.sendOp({ type: "setDoor", door, open }, { kind: "other" });
  }

  lockDoor(door: number, locked: boolean): void {
    this.sendOp({ type: "lockDoor", door, locked }, { kind: "other" });
  }

  resetExplored(): void {
    this.sendOp({ type: "resetExplored" }, { kind: "other" });
  }

  /** Shows a token moving to others while it's dragged, at most every 50 ms. */
  drag(tokenId: number, x: number, y: number): void {
    this.queuedDrag = { tokenId, x, y };
    const wait = this.lastDragSent + DRAG_INTERVAL_MS - this.now();
    if (wait <= 0) {
      this.flushDrag();
    } else if (this.dragTimer === undefined) {
      this.dragTimer = setTimeout(() => this.flushDrag(), wait);
    }
  }

  clearNotice(): void {
    this.set({ notice: null });
  }

  // --- Internals ---

  private join(): void {
    const resume = this.epoch === null ? undefined : { epoch: this.epoch, seq: this.seq };
    this.link.send({ type: "joinScene", sceneId: this.sceneId, resume });
  }

  private sendOp(op: SceneOp, pending: Pending): void {
    const reqId = this.nextReq++;
    if (!this.link.send({ type: "op", reqId, op })) {
      this.set({ notice: "Not connected. Your change wasn't sent." });
      return;
    }
    this.pending.set(reqId, pending);
    this.refresh();
  }

  private flushDrag(): void {
    this.dragTimer = undefined;
    const drag = this.queuedDrag;
    if (!drag) return;
    this.queuedDrag = null;
    this.lastDragSent = this.now();
    this.link.send({ type: "drag", tokenId: drag.tokenId, x: drag.x, y: drag.y });
  }

  private receive(msg: ServerMsg): void {
    switch (msg.type) {
      case "sceneSnapshot":
        if (msg.sceneId !== this.sceneId) return;
        this.confirmed = new Map(msg.tokens.map((t) => [t.id, t]));
        this.epoch = msg.epoch;
        this.seq = msg.seq;
        this.set({
          status: "live",
          role: msg.role,
          kicked: null,
          doors: msg.doors,
          explored: msg.explored ? { ...msg.explored, bits: decodeBase64(msg.explored.bits) } : null,
        });
        this.refresh();
        break;
      case "event":
        this.seq = msg.seq;
        this.applyEvent(msg.event);
        this.refresh();
        break;
      case "ack":
        this.pending.delete(msg.reqId);
        this.refresh();
        break;
      case "reject":
        this.pending.delete(msg.reqId);
        this.set({ notice: msg.message });
        this.refresh();
        break;
      case "tokenDragging":
        this.dragging.set(msg.tokenId, { x: msg.x, y: msg.y, userId: msg.userId, at: this.now() });
        this.refresh();
        break;
      case "kicked":
        this.pending.clear();
        this.epoch = null;
        if (msg.rejoin) {
          // A role change: join again for a fresh view with the new role.
          this.set({ status: "joining" });
          this.join();
        } else {
          this.set({ status: "kicked", kicked: { reason: msg.reason, rejoin: msg.rejoin } });
        }
        break;
      case "sceneChanged":
        if (msg.sceneId === this.sceneId) this.onSceneChanged();
        break;
      default:
        break; // Not about this scene.
    }
  }

  private applyEvent(event: SceneEvent): void {
    switch (event.type) {
      case "tokenCreated":
      case "tokenUpdated":
        this.confirmed.set(event.token.id, event.token);
        break;
      case "tokenMoved": {
        const t = this.confirmed.get(event.tokenId);
        if (t) this.confirmed.set(t.id, { ...t, x: event.x, y: event.y });
        this.dragging.delete(event.tokenId);
        break;
      }
      case "tokenRemoved":
        this.confirmed.delete(event.tokenId);
        this.dragging.delete(event.tokenId);
        break;
      case "doorChanged": {
        const doors = [...this.view.doors];
        doors[event.door] = { open: event.open, locked: event.locked };
        this.set({ doors });
        break;
      }
      case "exploredReset": {
        const e = this.view.explored;
        if (e) this.set({ explored: { ...e, bits: new Uint8Array(e.bits.length) } });
        break;
      }
    }
  }

  private dropStaleDrags(): void {
    const cutoff = this.now() - DRAG_STALE_MS;
    let changed = false;
    for (const [id, d] of this.dragging) {
      if (d.at < cutoff) {
        this.dragging.delete(id);
        changed = true;
      }
    }
    if (changed) this.refresh();
  }

  /** Rebuilds the view: confirmed tokens with my pending changes on top. */
  private refresh(): void {
    const tokens = new Map(this.confirmed);
    // Map iteration follows insertion order, so later requests win.
    for (const p of this.pending.values()) {
      if (p.kind === "create" || p.kind === "other") continue; // Shown once the server answers.
      const t = tokens.get(p.tokenId);
      if (!t) continue;
      if (p.kind === "move") tokens.set(t.id, { ...t, x: p.x, y: p.y });
      else if (p.kind === "update") tokens.set(t.id, { ...t, ...p.settings });
      else tokens.delete(t.id);
    }
    this.set({ tokens: [...tokens.values()], dragging: new Map(this.dragging) });
  }

  private set(patch: Partial<SceneView>): void {
    this.view = { ...this.view, ...patch };
    for (const listener of this.listeners) listener();
  }
}

function decodeBase64(text: string): Uint8Array {
  const raw = atob(text);
  const bytes = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
  return bytes;
}
