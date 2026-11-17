// Loading a scene from the server: its data, then its picture tile by tile.

import { campaigns } from "../api/campaigns";
import type { SceneDetail } from "../protocol/generated/SceneDetail";
import { type GridSpec, grid } from "./coords";
import { type LoadedMap, MapLoadError, type MapTile } from "./loadUvtt";

/**
 * The scene's grid as the renderer wants it. The scene stores an offset in
 * pixels (how far the first grid line is from the image's left/top edge);
 * the renderer wants the grid position of the image's corner, which is the
 * same shift measured in squares, with the sign flipped.
 */
export function sceneGrid(pixelsPerGrid: number, offsetX: number, offsetY: number): GridSpec {
  return { pixelsPerGrid, origin: grid(-offsetX / pixelsPerGrid, -offsetY / pixelsPerGrid) };
}

/**
 * Fetches every tile and decodes each one separately. The full picture is
 * never decoded in one piece, which is what keeps large maps within Safari's
 * memory limits on iPad.
 */
export async function loadSceneMap(campaignId: number, scene: SceneDetail): Promise<LoadedMap> {
  const tiles: MapTile[] = await Promise.all(
    scene.tiles.map(async (t) => {
      const res = await fetch(campaigns.tileUrl(campaignId, scene.assetId, t.index), { credentials: "same-origin" });
      if (!res.ok) throw new MapLoadError(`Couldn't load part of the map (tile ${t.index}).`);
      const bitmap = await createImageBitmap(await res.blob());
      return { bitmap, x: t.x, y: t.y };
    }),
  );
  return {
    name: scene.name,
    uvtt: scene.map,
    grid: sceneGrid(scene.pixelsPerGrid, scene.gridOffsetX, scene.gridOffsetY),
    width: scene.imageWidth,
    height: scene.imageHeight,
    tiles,
    warnings: [],
  };
}
