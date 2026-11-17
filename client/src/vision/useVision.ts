// Vision on the page: builds the scene's sight (WebAssembly), keeps its doors
// in step with the live scene, and paints the fog of war.

import { useEffect, useMemo, useRef, useState } from "react";
import type { FogLayer } from "../map/MapCanvas";
import type { SceneDetail } from "../protocol/generated/SceneDetail";
import type { Token } from "../protocol/generated/Token";
import type { SceneView } from "../scene/sceneStore";
import type { DragState } from "../scene/tokenDrag";
import { type RenderDoor, renderDoors } from "./doors";
import { FogPainter } from "./fog";
import { type SceneSight, type Viewer, addExplored, anySees, loadSight, sceneSight, syncDoors, visionPolygons } from "./sight";

interface Options {
  scene: SceneDetail | null;
  view: SceneView;
  /** Whose eyes to use: the player themselves, or the player a GM previews. `null`: no fog. */
  viewerId: number | null;
  /** Players accumulate explored fog; a GM's preview doesn't. */
  explore: boolean;
  drag: DragState | null;
}

export interface Vision {
  doors: readonly RenderDoor[];
  fog: FogLayer | null;
  /** In a GM preview: the tokens that player would see. `null` when not previewing. */
  visibleIds: ReadonlySet<number> | null;
  /** The viewer has no token that can see. */
  blind: boolean;
  /** The WebAssembly module failed to load. */
  error: string | null;
}

export function useVision({ scene, view, viewerId, explore, drag }: Options): Vision {
  const [sight, setSight] = useState<SceneSight | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Build the scene's sight once WebAssembly is ready; rebuild when the
  // scene's grid changes. WebAssembly memory isn't garbage-collected like
  // JavaScript objects, so the old one is freed explicitly.
  useEffect(() => {
    if (!scene) return;
    let built: SceneSight | null = null;
    let cancelled = false;
    loadSight()
      .then(() => {
        if (cancelled) return;
        built = sceneSight(scene);
        setSight(built);
      })
      .catch((e: unknown) => setError(`Line of sight couldn't start: ${String(e)}`));
    return () => {
      cancelled = true;
      setSight(null);
      built?.free();
    };
  }, [scene]);

  // Door states first: everything below depends on which doors are open.
  const doorStates = useMemo(() => {
    if (sight) syncDoors(sight, view.doors);
    return view.doors;
  }, [sight, view.doors]);

  const doors = useMemo(() => (sight ? renderDoors(sight.doorLines(), doorStates) : []), [sight, doorStates]);

  const placement = scene
    ? { pixelsPerGrid: scene.pixelsPerGrid, offsetX: scene.gridOffsetX, offsetY: scene.gridOffsetY }
    : null;

  // Where each token is drawn right now: my own drag, someone else's, or still.
  const positioned = useMemo(
    () =>
      view.tokens.map((t: Token): Viewer => {
        if (drag?.tokenId === t.id) return { token: t, x: drag.x, y: drag.y };
        const r = view.dragging.get(t.id);
        return r ? { token: t, x: r.x, y: r.y } : { token: t, x: t.x, y: t.y };
      }),
    [view.tokens, view.dragging, drag],
  );

  const viewers = useMemo(
    () => (viewerId === null ? [] : positioned.filter((v) => v.token.ownerId === viewerId && !v.token.hidden)),
    [positioned, viewerId],
  );

  const polygons = useMemo(
    () => (sight && placement && viewerId !== null ? visionPolygons(sight, viewers, placement) : []),
    // `placement` is rebuilt every render; `scene` is what it comes from.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [sight, viewers, scene, viewerId, doorStates],
  );

  const visibleIds = useMemo(() => {
    if (!sight || !placement || viewerId === null || explore) return null;
    const ids = new Set<number>();
    for (const v of positioned) {
      if (v.token.hidden) continue;
      if (v.token.ownerId === viewerId || anySees(sight, viewers, v, placement)) ids.add(v.token.id);
    }
    return ids;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sight, positioned, viewers, viewerId, explore, scene, doorStates]);

  const painter = useMemo(() => (scene ? new FogPainter(scene.imageWidth, scene.imageHeight) : null), [scene]);
  const version = useRef(0);
  const [fog, setFog] = useState<FogLayer | null>(null);

  useEffect(() => {
    if (!painter || !sight || viewerId === null) {
      setFog(null);
      return;
    }
    const explored = explore ? view.explored : null;
    // Explored fog grows from where tokens are, not from mid-drag positions:
    // the server only counts a drop, so the two stay the same.
    if (explored && !drag) addExplored(explored, polygons);
    painter.paint(polygons, explored);
    version.current += 1;
    setFog({ canvas: painter.canvas, width: painter.worldWidth, height: painter.worldHeight, version: version.current });
  }, [painter, sight, viewerId, explore, view.explored, polygons, drag]);

  return { doors, fog, visibleIds, blind: viewerId !== null && polygons.length === 0, error };
}
