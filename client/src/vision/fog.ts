// Paints the fog of war onto a 2D canvas, which the renderer shows as one
// picture over the map (VIS-004, VIS-005):
// - unexplored: black;
// - explored but not seen now: dimmed;
// - seen now: clear.
//
// The canvas is smaller than the map (at most 2048 px on its longer side) and
// drawn scaled up with smoothing, which also softens the fog's edges.

import type { ExploredFog } from "../scene/sceneStore";

const MAX_SIDE = 2048;
/** How dark explored-but-unseen areas are (0 clear, 1 black). */
const EXPLORED_DARKNESS = 0.6;

export class FogPainter {
  readonly canvas: HTMLCanvasElement;
  /** Canvas pixels per world pixel. */
  private readonly scale: number;
  private readonly ctx: CanvasRenderingContext2D;
  private exploredCanvas: HTMLCanvasElement | null = null;

  constructor(
    readonly worldWidth: number,
    readonly worldHeight: number,
  ) {
    this.scale = Math.min(1, MAX_SIDE / Math.max(worldWidth, worldHeight));
    this.canvas = document.createElement("canvas");
    this.canvas.width = Math.max(1, Math.round(worldWidth * this.scale));
    this.canvas.height = Math.max(1, Math.round(worldHeight * this.scale));
    const ctx = this.canvas.getContext("2d");
    if (!ctx) throw new Error("No 2D canvas available");
    this.ctx = ctx;
  }

  /**
   * Repaints. `visible`: polygons in world pixels (flat x, y lists).
   * `explored`: the player's explored cells, or null to leave everything else black.
   */
  paint(visible: readonly Float64Array[], explored: ExploredFog | null): void {
    const { ctx, canvas } = this;
    ctx.globalCompositeOperation = "source-over";
    ctx.globalAlpha = 1;
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, canvas.width, canvas.height);

    // "destination-out" erases: what's drawn removes that much of the black.
    ctx.globalCompositeOperation = "destination-out";
    if (explored) {
      ctx.globalAlpha = 1 - EXPLORED_DARKNESS;
      ctx.imageSmoothingEnabled = true;
      ctx.drawImage(
        this.exploredImage(explored),
        0,
        0,
        explored.cols * explored.cell * this.scale,
        explored.rows * explored.cell * this.scale,
      );
    }
    ctx.globalAlpha = 1;
    ctx.fillStyle = "#fff";
    for (const poly of visible) {
      if (poly.length < 6) continue;
      ctx.beginPath();
      ctx.moveTo(poly[0]! * this.scale, poly[1]! * this.scale);
      for (let i = 2; i < poly.length; i += 2) ctx.lineTo(poly[i]! * this.scale, poly[i + 1]! * this.scale);
      ctx.closePath();
      ctx.fill();
    }
    ctx.globalCompositeOperation = "source-over";
  }

  /** The explored bits as a tiny picture: one pixel per cell, opaque where explored. */
  private exploredImage(fog: ExploredFog): HTMLCanvasElement {
    let c = this.exploredCanvas;
    if (!c || c.width !== fog.cols || c.height !== fog.rows) {
      c = document.createElement("canvas");
      c.width = fog.cols;
      c.height = fog.rows;
      this.exploredCanvas = c;
    }
    const ctx = c.getContext("2d")!;
    const image = ctx.createImageData(fog.cols, fog.rows);
    const px = image.data;
    const cells = fog.cols * fog.rows;
    for (let i = 0; i < cells; i++) {
      if (fog.bits[i >> 3]! & (1 << (i & 7))) px[i * 4 + 3] = 255;
    }
    ctx.putImageData(image, 0, 0);
    return c;
  }
}
