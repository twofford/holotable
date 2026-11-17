// Coordinate spaces and the camera.
//
// The map uses three coordinate spaces, and mixing them up is the classic VTT bug:
//
// - Screen: CSS pixels inside the map's canvas. (0, 0) is its top-left corner.
//   Pointer events arrive in this space.
// - World: pixels of the map image at 100% zoom. (0, 0) is the image's top-left.
//   The renderer draws everything in this space and lets the camera move it.
// - Grid: squares, as in the UVTT file. (2.5, 1) is halfway across the third
//   column, on the top edge of the second row.
//
// Each space has its own "branded" type. At runtime all three are plain
// { x, y } objects, but TypeScript treats them as different types, so passing a
// ScreenPoint where a WorldPoint is expected is a compile error. The only way to
// move between spaces is through the conversion functions below.

declare const brand: unique symbol;
type Brand<T, B extends string> = T & { readonly [brand]: B };
type XY = { readonly x: number; readonly y: number };

export type ScreenPoint = Brand<XY, "screen">;
export type WorldPoint = Brand<XY, "world">;
export type GridPos = Brand<XY, "grid">;

// Constructors. The `as` cast is the one place a plain object gets its brand.
export const screen = (x: number, y: number) => ({ x, y }) as ScreenPoint;
export const world = (x: number, y: number) => ({ x, y }) as WorldPoint;
export const grid = (x: number, y: number) => ({ x, y }) as GridPos;

/** How grid squares map onto image pixels. Comes from the UVTT `resolution`. */
export interface GridSpec {
  readonly pixelsPerGrid: number;
  /** Grid position of the image's top-left corner. Almost always (0, 0). */
  readonly origin: GridPos;
}

export function gridToWorld(g: GridPos, spec: GridSpec): WorldPoint {
  return world((g.x - spec.origin.x) * spec.pixelsPerGrid, (g.y - spec.origin.y) * spec.pixelsPerGrid);
}

export function worldToGrid(w: WorldPoint, spec: GridSpec): GridPos {
  return grid(w.x / spec.pixelsPerGrid + spec.origin.x, w.y / spec.pixelsPerGrid + spec.origin.y);
}

/** The grid square containing a point: its top-left corner, in whole squares. */
export function snapToSquare(g: GridPos): GridPos {
  return grid(Math.floor(g.x), Math.floor(g.y));
}

/**
 * What part of the world is on screen.
 * `x`, `y`: the world point shown at the screen's top-left corner.
 * `scale`: screen pixels per world pixel (1 = 100%, 0.5 = zoomed out to 50%).
 */
export interface Camera {
  readonly x: number;
  readonly y: number;
  readonly scale: number;
}

/** Zoom limits from the spec (MAP-005): 10% to 400%. */
export const MIN_SCALE = 0.1;
export const MAX_SCALE = 4;

const clampScale = (s: number) => Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));

export function screenToWorld(cam: Camera, p: ScreenPoint): WorldPoint {
  return world(p.x / cam.scale + cam.x, p.y / cam.scale + cam.y);
}

export function worldToScreen(cam: Camera, p: WorldPoint): ScreenPoint {
  return screen((p.x - cam.x) * cam.scale, (p.y - cam.y) * cam.scale);
}

/** Moves the view by a distance in screen pixels, as when dragging the map. */
export function panBy(cam: Camera, dx: number, dy: number): Camera {
  return { ...cam, x: cam.x - dx / cam.scale, y: cam.y - dy / cam.scale };
}

/**
 * Zooms by `factor` around `anchor`, so the world point under the anchor stays
 * under it. That's what makes zooming toward the mouse pointer feel right.
 */
export function zoomAt(cam: Camera, anchor: ScreenPoint, factor: number): Camera {
  const scale = clampScale(cam.scale * factor);
  const fixed = screenToWorld(cam, anchor);
  return { scale, x: fixed.x - anchor.x / scale, y: fixed.y - anchor.y / scale };
}

/**
 * A two-finger gesture from one frame to the next: the fingers moved from
 * (prevA, prevB) to (curA, curB). Spreading the fingers zooms, moving them
 * together pans, and both can happen at once.
 */
export function pinch(cam: Camera, prevA: ScreenPoint, prevB: ScreenPoint, curA: ScreenPoint, curB: ScreenPoint): Camera {
  const prevDist = Math.hypot(prevA.x - prevB.x, prevA.y - prevB.y);
  const curDist = Math.hypot(curA.x - curB.x, curA.y - curB.y);
  const prevMid = screen((prevA.x + prevB.x) / 2, (prevA.y + prevB.y) / 2);
  const curMid = screen((curA.x + curB.x) / 2, (curA.y + curB.y) / 2);
  // Fingers on the same spot give a distance of 0: don't divide by it.
  const factor = prevDist > 0 ? curDist / prevDist : 1;
  const zoomed = zoomAt(cam, prevMid, factor);
  return panBy(zoomed, curMid.x - prevMid.x, curMid.y - prevMid.y);
}

/** A camera that shows the whole map, centred, with a margin. */
export function fitCamera(mapWidth: number, mapHeight: number, viewWidth: number, viewHeight: number, margin = 0.95): Camera {
  const scale = clampScale(Math.min(viewWidth / mapWidth, viewHeight / mapHeight) * margin);
  return {
    scale,
    x: mapWidth / 2 - viewWidth / 2 / scale,
    y: mapHeight / 2 - viewHeight / 2 / scale,
  };
}
