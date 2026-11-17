// Finding the door under a tap or click.

/** A door's line in world pixels, and its state. */
export interface RenderDoor {
  readonly x1: number;
  readonly y1: number;
  readonly x2: number;
  readonly y2: number;
  readonly open: boolean;
  readonly locked: boolean;
}

/** Distance from (px, py) to the segment, in the same units. */
export function distanceToSegment(px: number, py: number, d: RenderDoor): number {
  const ex = d.x2 - d.x1;
  const ey = d.y2 - d.y1;
  const len2 = ex * ex + ey * ey;
  const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((px - d.x1) * ex + (py - d.y1) * ey) / len2));
  return Math.hypot(px - (d.x1 + t * ex), py - (d.y1 + t * ey));
}

/** The index of the nearest door within `tolerance` of the point, if any. */
export function doorAt(doors: readonly RenderDoor[], px: number, py: number, tolerance: number): number | null {
  let best: number | null = null;
  let bestDist = tolerance;
  doors.forEach((d, i) => {
    const dist = distanceToSegment(px, py, d);
    if (dist <= bestDist) {
      best = i;
      bestDist = dist;
    }
  });
  return best;
}

/** Builds the doors to draw from their lines (from WebAssembly) and states. */
export function renderDoors(lines: Float64Array, states: readonly { open: boolean; locked: boolean }[]): RenderDoor[] {
  const out: RenderDoor[] = [];
  for (let i = 0; i * 4 + 3 < lines.length; i++) {
    const s = states[i] ?? { open: false, locked: false };
    out.push({ x1: lines[i * 4]!, y1: lines[i * 4 + 1]!, x2: lines[i * 4 + 2]!, y2: lines[i * 4 + 3]!, ...s });
  }
  return out;
}
