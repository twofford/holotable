// Line of sight in the browser: the Rust code from holotable-core, compiled to
// WebAssembly (crates/wasm). `npm run wasm` builds it into ../wasm/.
//
// A .wasm module has to be downloaded and compiled before its functions can be
// called, so `loadSight()` returns a promise; everything else here assumes it
// has resolved.

import init, { SceneSight, rasterize } from "../wasm/holotable_wasm";
import type { SceneDetail } from "../protocol/generated/SceneDetail";
import type { Token } from "../protocol/generated/Token";
import type { DoorState } from "../protocol/generated/DoorState";
import type { ExploredFog } from "../scene/sceneStore";

export { SceneSight };

let loading: Promise<unknown> | null = null;

/** Downloads and starts the WebAssembly module, once. */
export function loadSight(): Promise<void> {
  // `init()` fetches holotable_wasm_bg.wasm next to the generated JS; Vite
  // copies it into the build because the glue refers to it with
  // `new URL(..., import.meta.url)`.
  loading ??= init();
  return loading.then(() => undefined);
}

/** A scene's walls and doors in WebAssembly memory. Call `free()` when done. */
export function sceneSight(scene: SceneDetail): SceneSight {
  return new SceneSight(
    JSON.stringify(scene.map),
    scene.imageWidth,
    scene.imageHeight,
    scene.pixelsPerGrid,
    scene.gridOffsetX,
    scene.gridOffsetY,
  );
}

export function syncDoors(sight: SceneSight, doors: readonly DoorState[]): void {
  doors.forEach((d, i) => sight.setDoor(i, d.open, d.locked));
}

/** Grid placement of a token, in world (image) pixels. */
export interface Placement {
  readonly pixelsPerGrid: number;
  readonly offsetX: number;
  readonly offsetY: number;
}

/** Feet per grid square, as in holotable-core. */
const FEET_PER_SQUARE = 5;

function eye(t: Token, x: number, y: number, p: Placement) {
  const half = t.size / 2;
  return {
    x: (x + half) * p.pixelsPerGrid + p.offsetX,
    y: (y + half) * p.pixelsPerGrid + p.offsetY,
    range: t.visionRange === null ? 0 : (t.visionRange / FEET_PER_SQUARE) * p.pixelsPerGrid,
  };
}

/** A token and where it's drawn right now (it may be mid-drag). */
export interface Viewer {
  readonly token: Token;
  readonly x: number;
  readonly y: number;
}

/** Each seeing token's visibility polygon, as flat x, y lists. */
export function visionPolygons(sight: SceneSight, viewers: readonly Viewer[], p: Placement): Float64Array[] {
  return viewers
    .filter((v) => v.token.vision)
    .map((v) => {
      const e = eye(v.token, v.x, v.y, p);
      return sight.polygon(e.x, e.y, e.range);
    });
}

/** Can any of the viewers see the target token at (x, y)? Same rule as the server's. */
export function anySees(sight: SceneSight, viewers: readonly Viewer[], target: Viewer, p: Placement): boolean {
  const half = (target.token.size * p.pixelsPerGrid) / 2;
  const cx = (target.x + target.token.size / 2) * p.pixelsPerGrid + p.offsetX;
  const cy = (target.y + target.token.size / 2) * p.pixelsPerGrid + p.offsetY;
  return viewers.some((v) => {
    if (!v.token.vision) return false;
    const e = eye(v.token, v.x, v.y, p);
    return sight.seesSquare(e.x, e.y, e.range, cx, cy, half);
  });
}

/** Adds the polygons to explored fog. Returns whether anything new was seen. */
export function addExplored(fog: ExploredFog, polygons: readonly Float64Array[]): boolean {
  let changed = false;
  for (const poly of polygons) changed = rasterize(poly, fog.cell, fog.cols, fog.rows, fog.bits) || changed;
  return changed;
}
