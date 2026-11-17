// Turning the scene's tokens (grid squares) into what the renderer draws
// (world pixels), with drags, the drag ghost and selection applied.

import { type GridSpec, grid, gridToWorld } from "../map/coords";
import type { Token } from "../protocol/generated/Token";
import type { RenderToken } from "../render/renderer";
import type { RemoteDrag } from "./sceneStore";
import type { DragState } from "./tokenDrag";

/** Disc colours for tokens without a picture, picked by id so each keeps its colour. */
const COLORS = [0x3b6ea8, 0x8a4fb8, 0x2f8f6b, 0xb8603b, 0xa83b5a, 0x6b7a2f, 0x3b8fa8, 0x7a5a3b];

export interface RenderInput {
  tokens: readonly Token[];
  grid: GridSpec;
  selectedId: number | null;
  /** My own drag in progress. */
  drag: DragState | null;
  /** Other people's drags. */
  remote: ReadonlyMap<number, RemoteDrag>;
  imageUrl(assetId: number): string;
}

export function renderTokens({ tokens, grid: spec, selectedId, drag, remote, imageUrl }: RenderInput): RenderToken[] {
  const out: RenderToken[] = [];
  let dragged: RenderToken | null = null;
  for (const t of tokens) {
    const base = {
      size: t.size * spec.pixelsPerGrid,
      name: t.name,
      imageUrl: t.assetId === null ? null : imageUrl(t.assetId),
      color: COLORS[t.id % COLORS.length]!,
      hidden: t.hidden,
      selected: t.id === selectedId,
    };
    const at = (x: number, y: number) => gridToWorld(grid(x, y), spec);
    if (drag?.tokenId === t.id) {
      // A faint copy stays where the drag began; the token follows the pointer.
      const start = at(t.x, t.y);
      out.push({ ...base, key: `ghost-${t.id}`, x: start.x, y: start.y, selected: false, ghost: true, label: null });
      const now = at(drag.x, drag.y);
      dragged = { ...base, key: `token-${t.id}`, x: now.x, y: now.y, ghost: false, label: `${drag.distanceFt} ft` };
      continue;
    }
    const r = remote.get(t.id);
    const p = r ? at(r.x, r.y) : at(t.x, t.y);
    out.push({ ...base, key: `token-${t.id}`, x: p.x, y: p.y, ghost: false, label: null });
  }
  // The token I'm dragging goes on top of everything.
  if (dragged) out.push(dragged);
  return out;
}
