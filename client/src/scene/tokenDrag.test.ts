import { afterEach, describe, expect, it, vi } from "vitest";
import { type GridSpec, grid, screen, world } from "../map/coords";
import type { PointerInfo } from "../map/panZoom";
import type { Token } from "../protocol/generated/Token";
import { type DragState, TokenDragController, distanceFeet, tokenAt } from "./tokenDrag";

const spec: GridSpec = { pixelsPerGrid: 50, origin: grid(0, 0) };
const mine: Token = { id: 1, name: "Vex", assetId: null, x: 2, y: 2, size: 1, ownerId: 2, hidden: false };
const theirs: Token = { id: 2, name: "Thug", assetId: null, x: 5, y: 5, size: 2, ownerId: null, hidden: false };

/** A pointer at world (x, y); screen = world, for simplicity. */
const at = (x: number, y: number, extra: Partial<PointerInfo> = {}): PointerInfo => ({
  screen: screen(x, y),
  world: world(x, y),
  altKey: false,
  pointerType: "mouse",
  ...extra,
});

function setup(snap = true) {
  const calls = {
    moves: [] as [number, number, number][],
    drags: 0,
    selected: null as number | null,
    drag: null as DragState | null,
    menu: null as number | null,
  };
  const ctl = new TokenDragController({
    tokens: () => [mine, theirs],
    grid: () => spec,
    canMove: (t) => t.ownerId === 2,
    snapEnabled: () => snap,
    select: (id) => (calls.selected = id),
    onDragChange: (d) => (calls.drag = d),
    drag: () => calls.drags++,
    move: (id, x, y) => calls.moves.push([id, x, y]),
    openMenu: (id) => (calls.menu = id),
  });
  return { ctl, calls };
}

afterEach(() => vi.useRealTimers());

describe("tokenAt", () => {
  it("finds the token under a point, using its size", () => {
    expect(tokenAt([mine, theirs], spec, { x: 110, y: 110 })?.id).toBe(1);
    expect(tokenAt([mine, theirs], spec, { x: 340, y: 340 })?.id).toBe(2); // 2×2 token spans 250–350.
    expect(tokenAt([mine, theirs], spec, { x: 10, y: 10 })).toBeNull();
  });
});

describe("distanceFeet", () => {
  it("counts squares, with diagonals as one", () => {
    expect(distanceFeet(2, 2, 5, 2)).toBe(15);
    expect(distanceFeet(2, 2, 5, 4)).toBe(15);
    expect(distanceFeet(2, 2, 2, 2)).toBe(0);
  });
});

describe("TokenDragController", () => {
  it("drags my token and snaps the drop to the grid", () => {
    const { ctl, calls } = setup();
    expect(ctl.down(at(110, 110))).toBe(true);
    ctl.move(at(170, 115));
    expect(calls.drag).toMatchObject({ tokenId: 1, distanceFt: 5 });
    ctl.up(at(172, 118));
    expect(calls.moves).toEqual([[1, 3, 2]]);
    expect(calls.drag).toBeNull();
    expect(calls.drags).toBeGreaterThan(0);
  });

  it("holding Alt (or snapping off) drops exactly where released", () => {
    const { ctl, calls } = setup();
    ctl.down(at(110, 110));
    ctl.move(at(175, 110));
    ctl.up(at(175, 110, { altKey: true }));
    expect(calls.moves[0]![1]).toBeCloseTo(3.3);
  });

  it("selects tokens I can't move without claiming the pointer", () => {
    const { ctl, calls } = setup();
    expect(ctl.down(at(300, 300))).toBe(false);
    expect(calls.selected).toBe(2);
    expect(ctl.down(at(10, 10))).toBe(false); // Empty map: pan.
  });

  it("a tap selects without moving", () => {
    const { ctl, calls } = setup();
    ctl.down(at(110, 110));
    ctl.move(at(112, 111)); // Within the slop.
    ctl.up(at(112, 111));
    expect(calls.moves).toEqual([]);
    expect(calls.selected).toBe(1);
  });

  it("a long press on touch opens the menu instead of dragging", () => {
    vi.useFakeTimers();
    const { ctl, calls } = setup();
    ctl.down(at(110, 110, { pointerType: "touch" }));
    vi.advanceTimersByTime(600);
    expect(calls.menu).toBe(1);
    ctl.up(at(110, 110, { pointerType: "touch" }));
    expect(calls.moves).toEqual([]);
  });
});
