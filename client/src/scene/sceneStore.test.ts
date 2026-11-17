import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClientMsg } from "../protocol/generated/ClientMsg";
import type { ServerMsg } from "../protocol/generated/ServerMsg";
import type { Token } from "../protocol/generated/Token";
import { DRAG_INTERVAL_MS, type SceneLink, SceneStore } from "./sceneStore";

/** A pretend connection: records what's sent and lets tests deliver messages. */
class FakeLink implements SceneLink {
  sent: ClientMsg[] = [];
  isOpen = true;
  private msgListeners = new Set<(m: ServerMsg) => void>();
  private openListeners = new Set<() => void>();
  send(msg: ClientMsg) {
    if (!this.isOpen) return false;
    this.sent.push(msg);
    return true;
  }
  onMessage(l: (m: ServerMsg) => void) {
    this.msgListeners.add(l);
    return () => this.msgListeners.delete(l);
  }
  onOpen(l: () => void) {
    this.openListeners.add(l);
    return () => this.openListeners.delete(l);
  }
  deliver(msg: ServerMsg) {
    for (const l of this.msgListeners) l(msg);
  }
  reopen() {
    for (const l of this.openListeners) l();
  }
  lastOp() {
    const op = [...this.sent].reverse().find((m) => m.type === "op");
    if (op?.type !== "op") throw new Error("no op sent");
    return op;
  }
}

const vex: Token = { id: 7, name: "Vex", assetId: null, x: 1, y: 1, size: 1, ownerId: 2, hidden: false, vision: true, visionRange: null };

function setup() {
  const link = new FakeLink();
  const store = new SceneStore(link, 3);
  store.start();
  link.deliver({ type: "sceneSnapshot", sceneId: 3, epoch: "e1", seq: 5, role: "player", tokens: [vex],
    doors: [{ open: false, locked: false }], explored: { cell: 10, cols: 8, rows: 2, bits: "/wA=" } });
  return { link, store };
}

describe("SceneStore", () => {
  beforeEach(() => vi.useRealTimers());

  it("joins on start and goes live with the snapshot", () => {
    const { link, store } = setup();
    expect(link.sent[0]).toEqual({ type: "joinScene", sceneId: 3, resume: undefined });
    expect(store.getSnapshot().status).toBe("live");
    expect(store.getSnapshot().tokens).toEqual([vex]);
  });

  it("shows my move at once, and keeps it when acknowledged", () => {
    const { link, store } = setup();
    store.move(7, 4, 2);
    expect(store.getSnapshot().tokens[0]).toMatchObject({ x: 4, y: 2 });
    const { reqId } = link.lastOp();
    link.deliver({ type: "event", seq: 6, event: { type: "tokenMoved", tokenId: 7, x: 4, y: 2 } });
    link.deliver({ type: "ack", reqId, seq: 6, tokenId: null });
    expect(store.getSnapshot().tokens[0]).toMatchObject({ x: 4, y: 2 });
  });

  it("puts the token back when the server refuses", () => {
    const { link, store } = setup();
    store.move(7, 9, 2);
    const { reqId } = link.lastOp();
    link.deliver({ type: "reject", reqId, code: "blocked", message: "There's a wall in the way." });
    expect(store.getSnapshot().tokens[0]).toMatchObject({ x: 1, y: 1 });
    expect(store.getSnapshot().notice).toBe("There's a wall in the way.");
  });

  it("rejoins with a resume point after a reconnect", () => {
    const { link } = setup();
    link.deliver({ type: "event", seq: 9, event: { type: "tokenMoved", tokenId: 7, x: 2, y: 2 } });
    link.sent = [];
    link.reopen();
    expect(link.sent[0]).toEqual({ type: "joinScene", sceneId: 3, resume: { epoch: "e1", seq: 9 } });
  });

  it("sends drag updates at most every 50 ms, always including the latest", () => {
    vi.useFakeTimers();
    const { link, store } = setup();
    link.sent = [];
    store.drag(7, 1.1, 1);
    store.drag(7, 1.2, 1);
    store.drag(7, 1.3, 1);
    expect(link.sent).toHaveLength(1);
    vi.advanceTimersByTime(DRAG_INTERVAL_MS);
    expect(link.sent).toHaveLength(2);
    expect(link.sent[1]).toEqual({ type: "drag", tokenId: 7, x: 1.3, y: 1 });
  });

  it("shows other people's drags until the move lands", () => {
    const { link, store } = setup();
    link.deliver({ type: "tokenDragging", tokenId: 7, x: 3, y: 1, userId: 2 });
    expect(store.getSnapshot().dragging.get(7)).toMatchObject({ x: 3, y: 1 });
    link.deliver({ type: "event", seq: 6, event: { type: "tokenMoved", tokenId: 7, x: 3, y: 1 } });
    expect(store.getSnapshot().dragging.size).toBe(0);
  });

  it("joins again after a role change, and stops after a removal", () => {
    const { link, store } = setup();
    link.sent = [];
    link.deliver({ type: "kicked", reason: "Your role changed.", rejoin: true });
    expect(link.sent[0]).toEqual({ type: "joinScene", sceneId: 3, resume: undefined });
    link.deliver({ type: "kicked", reason: "Removed.", rejoin: false });
    expect(store.getSnapshot().status).toBe("kicked");
  });

  it("tracks doors and explored fog", () => {
    const { link, store } = setup();
    expect(Array.from(store.getSnapshot().explored!.bits)).toEqual([255, 0]);
    link.deliver({ type: "event", seq: 6, event: { type: "doorChanged", door: 0, open: true, locked: true } });
    expect(store.getSnapshot().doors[0]).toEqual({ open: true, locked: true });
    link.deliver({ type: "event", seq: 7, event: { type: "exploredReset" } });
    expect(Array.from(store.getSnapshot().explored!.bits)).toEqual([0, 0]);
    store.setDoor(0, false);
    expect(link.sent.at(-1)).toMatchObject({ type: "op", op: { type: "setDoor", door: 0, open: false } });
  });
});
