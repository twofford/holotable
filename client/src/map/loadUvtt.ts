// Loading a UVTT file in the browser.
//
// For M1 the client parses the JSON itself. The checks here are a light copy of
// the Rust ones in holotable-core; from M3 the server checks every upload with
// the Rust parser, and from M5 the client can call it through WASM.

import type { UvttMap } from "../protocol/generated/UvttMap";
import { type GridSpec, grid } from "./coords";

/** A map ready to draw: the parsed file plus its picture cut into tiles. */
export interface LoadedMap {
  readonly name: string;
  readonly uvtt: UvttMap;
  readonly grid: GridSpec;
  /** Size of the map picture in world pixels. */
  readonly width: number;
  readonly height: number;
  readonly tiles: readonly MapTile[];
  /** Problems that don't stop the map from loading. */
  readonly warnings: readonly string[];
}

/** One piece of the map picture, placed at (x, y) in world pixels. */
export interface MapTile {
  readonly bitmap: ImageBitmap;
  readonly x: number;
  readonly y: number;
}

/**
 * Largest tile side, in pixels. Older iPads limit GPU textures to 4096 px, and
 * smaller tiles also let the GPU skip tiles that are off screen.
 */
export const TILE_SIZE = 2048;

export class MapLoadError extends Error {}

/** Parses UVTT JSON and fills in fields that some exporters leave out. */
export function parseUvtt(text: string): UvttMap {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    throw new MapLoadError("This file isn't valid JSON, so it isn't a UVTT map.");
  }
  if (typeof raw !== "object" || raw === null) {
    throw new MapLoadError("This file isn't a UVTT map.");
  }
  // From here we read fields one by one and check them, so `raw` is treated as
  // a loose record rather than trusted to match UvttMap.
  const r = raw as Record<string, any>;
  const res = r.resolution;
  const ppg = res?.pixels_per_grid;
  const size = res?.map_size;
  if (typeof ppg !== "number" || ppg <= 0) {
    throw new MapLoadError("resolution.pixels_per_grid is missing or not a positive number.");
  }
  if (typeof size?.x !== "number" || typeof size?.y !== "number" || size.x <= 0 || size.y <= 0) {
    throw new MapLoadError("resolution.map_size is missing or not positive.");
  }
  if (typeof r.image !== "string" || r.image.trim() === "") {
    throw new MapLoadError("The file has no map image.");
  }
  return {
    format: typeof r.format === "number" ? r.format : 0,
    resolution: {
      map_origin: { x: res.map_origin?.x ?? 0, y: res.map_origin?.y ?? 0 },
      map_size: { x: size.x, y: size.y },
      pixels_per_grid: ppg,
    },
    line_of_sight: r.line_of_sight ?? [],
    objects_line_of_sight: r.objects_line_of_sight ?? [],
    portals: r.portals ?? [],
    lights: r.lights ?? [],
    environment: r.environment ?? null,
    image: r.image,
  };
}

/** Guesses the picture's type from the start of its base64 text. */
function imageMimeType(base64: string): string {
  if (base64.startsWith("iVBOR")) return "image/png";
  if (base64.startsWith("UklGR")) return "image/webp";
  if (base64.startsWith("/9j/")) return "image/jpeg";
  throw new MapLoadError("The map image isn't a PNG, WebP or JPEG.");
}

/**
 * Decodes the picture and cuts it into tiles no larger than TILE_SIZE.
 *
 * `createImageBitmap` decodes without using a <canvas>, so Safari's canvas-size
 * limit doesn't apply. The full-size bitmap is freed with `close()` as soon as
 * the tiles exist, rather than waiting for garbage collection.
 */
export async function decodeTiles(base64: string): Promise<{ tiles: MapTile[]; width: number; height: number }> {
  const compact = base64.replace(/\s/g, "");
  // Turning a data: URL into a Blob avoids building a huge string of bytes by hand.
  const blob = await (await fetch(`data:${imageMimeType(compact)};base64,${compact}`)).blob();

  let full: ImageBitmap;
  try {
    full = await createImageBitmap(blob);
  } catch {
    throw new MapLoadError("The browser couldn't decode the map image. It may be damaged, or too large for this device.");
  }

  const { width, height } = full;
  const jobs: Promise<MapTile>[] = [];
  for (let y = 0; y < height; y += TILE_SIZE) {
    for (let x = 0; x < width; x += TILE_SIZE) {
      const w = Math.min(TILE_SIZE, width - x);
      const h = Math.min(TILE_SIZE, height - y);
      jobs.push(createImageBitmap(full, x, y, w, h).then((bitmap) => ({ bitmap, x, y })));
    }
  }
  try {
    return { tiles: await Promise.all(jobs), width, height };
  } finally {
    full.close();
  }
}

/** Parses a UVTT file and prepares its picture for drawing. */
export async function loadMap(name: string, text: string): Promise<LoadedMap> {
  const uvtt = parseUvtt(text);
  const { tiles, width, height } = await decodeTiles(uvtt.image);
  const res = uvtt.resolution;

  const warnings: string[] = [];
  const expectedW = Math.round(res.map_size.x * res.pixels_per_grid);
  const expectedH = Math.round(res.map_size.y * res.pixels_per_grid);
  if (Math.abs(expectedW - width) > 1 || Math.abs(expectedH - height) > 1) {
    warnings.push(
      `The image is ${width}×${height} px, but the grid says ${expectedW}×${expectedH} px. The grid may not line up.`,
    );
  }

  return {
    name,
    uvtt,
    grid: { pixelsPerGrid: res.pixels_per_grid, origin: grid(res.map_origin.x, res.map_origin.y) },
    width,
    height,
    tiles,
    warnings,
  };
}

/** Frees the tiles' memory. Call when a map is replaced or the view closes. */
export function releaseMap(map: LoadedMap): void {
  for (const tile of map.tiles) tile.bitmap.close();
}
