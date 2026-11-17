// Dragging tokens with mouse, finger or Pencil (TOK-002, TOK-003, TOK-006).
//
// The pan/zoom controller asks this handler first whenever a pointer goes
// down. If the pointer is on a token the user may move, the handler claims
// it: the token follows the pointer, and on release it snaps to the grid
// (unless Alt is held or snapping is off) and the move is sent to the server.
// Holding still on a token for half a second opens its menu instead: the
// iPad's stand-in for right-click.

import type { Token } from "../protocol/generated/Token";
import { type GridSpec, type ScreenPoint, grid, gridToWorld, world, worldToGrid } from "../map/coords";
import type { PointerHandler, PointerInfo } from "../map/panZoom";

export interface DragState {
  readonly tokenId: number;
  /** Where the token was when the drag began, in grid squares. */
  readonly startX: number;
  readonly startY: number;
  /** Where it is now (following the pointer), in grid squares. */
  readonly x: number;
  readonly y: number;
  /** How far it will have moved once dropped, in feet. */
  readonly distanceFt: number;
}

export interface TokenDragDeps {
  tokens(): readonly Token[];
  grid(): GridSpec;
  canMove(t: Token): boolean;
  snapEnabled(): boolean;
  select(tokenId: number | null): void;
  onDragChange(drag: DragState | null): void;
  /** Live position for everyone else (throttled by the store). */
  drag(tokenId: number, x: number, y: number): void;
  /** The final move. */
  move(tokenId: number, x: number, y: number): void;
  openMenu(tokenId: number, at: ScreenPoint): void;
}

/** Feet per grid square. */
export const FEET_PER_SQUARE = 5;
const LONG_PRESS_MS = 500;
/** A press that moves less than this (screen px) hasn't started a drag. */
const DRAG_SLOP_PX = 6;

/** The top-most token under a point, if any. Later tokens are drawn on top. */
export function tokenAt(tokens: readonly Token[], spec: GridSpec, w: { x: number; y: number }): Token | null {
  for (let i = tokens.length - 1; i >= 0; i--) {
    const t = tokens[i]!;
    const corner = gridToWorld(grid(t.x, t.y), spec);
    const side = t.size * spec.pixelsPerGrid;
    if (w.x >= corner.x && w.x <= corner.x + side && w.y >= corner.y && w.y <= corner.y + side) return t;
  }
  return null;
}

/**
 * Distance between two token positions, in feet: the larger of the column and
 * row differences, in whole squares, so a diagonal step counts as one square.
 * (Simple and common; New York 2069 can change this rule later.)
 */
export function distanceFeet(fromX: number, fromY: number, toX: number, toY: number): number {
  const squares = Math.max(Math.abs(Math.round(toX) - Math.round(fromX)), Math.abs(Math.round(toY) - Math.round(fromY)));
  return squares * FEET_PER_SQUARE;
}

export class TokenDragController implements PointerHandler {
  private active: {
    token: Token;
    /** Pointer position relative to the token's corner, in world pixels. */
    grabX: number;
    grabY: number;
    startScreen: ScreenPoint;
    moved: boolean;
    x: number;
    y: number;
  } | null = null;
  private longPress: ReturnType<typeof setTimeout> | undefined;

  constructor(private readonly deps: TokenDragDeps) {}

  down(p: PointerInfo): boolean {
    const spec = this.deps.grid();
    const token = tokenAt(this.deps.tokens(), spec, p.world);
    if (!token) return false;
    this.deps.select(token.id);
    if (!this.deps.canMove(token)) return false; // Selected; the press pans as usual.

    const corner = gridToWorld(grid(token.x, token.y), spec);
    this.active = {
      token,
      grabX: p.world.x - corner.x,
      grabY: p.world.y - corner.y,
      startScreen: p.screen,
      moved: false,
      x: token.x,
      y: token.y,
    };
    // Touch and Pencil have no right button: a long press opens the menu.
    if (p.pointerType !== "mouse") {
      this.longPress = setTimeout(() => {
        const a = this.active;
        if (a && !a.moved) {
          this.active = null;
          this.deps.onDragChange(null);
          this.deps.openMenu(a.token.id, a.startScreen);
        }
      }, LONG_PRESS_MS);
    }
    return true;
  }

  move(p: PointerInfo): void {
    const a = this.active;
    if (!a) return;
    if (!a.moved) {
      if (Math.hypot(p.screen.x - a.startScreen.x, p.screen.y - a.startScreen.y) < DRAG_SLOP_PX) return;
      a.moved = true;
      clearTimeout(this.longPress);
    }
    const g = worldToGrid(world(p.world.x - a.grabX, p.world.y - a.grabY), this.deps.grid());
    a.x = g.x;
    a.y = g.y;
    const target = this.target(a.x, a.y, p.altKey);
    this.deps.onDragChange({
      tokenId: a.token.id,
      startX: a.token.x,
      startY: a.token.y,
      x: a.x,
      y: a.y,
      distanceFt: distanceFeet(a.token.x, a.token.y, target.x, target.y),
    });
    this.deps.drag(a.token.id, a.x, a.y);
  }

  up(p: PointerInfo): void {
    clearTimeout(this.longPress);
    const a = this.active;
    this.active = null;
    if (!a) return;
    this.deps.onDragChange(null);
    if (!a.moved) return; // A tap: the token is selected, nothing moves.
    const target = this.target(a.x, a.y, p.altKey);
    if (target.x !== a.token.x || target.y !== a.token.y) this.deps.move(a.token.id, target.x, target.y);
  }

  cancel(): void {
    clearTimeout(this.longPress);
    this.active = null;
    this.deps.onDragChange(null);
  }

  /** Where a drop at (x, y) lands: the nearest square, unless snapping is off. */
  private target(x: number, y: number, altKey: boolean): { x: number; y: number } {
    if (altKey || !this.deps.snapEnabled()) return { x, y };
    return { x: Math.round(x), y: Math.round(y) };
  }
}
