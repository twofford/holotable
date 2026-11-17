// The map renderer. This is the only file that imports PixiJS: the rest of the
// client talks to this small class, so Pixi could be swapped for raw WebGL later
// without touching anything else.

import { Application, Container, Graphics, Sprite, Text, Texture } from "pixi.js";
import type { Camera, GridSpec } from "../map/coords";
import type { LoadedMap } from "../map/loadUvtt";

export class Renderer {
  /** Everything drawn in world space. The camera moves and scales this container. */
  private readonly world = new Container();
  private readonly mapLayer = new Container();
  private readonly gridLayer = new Graphics();
  private readonly tokenLayer = new Container();
  /** One display object per token key, reused while its look doesn't change. */
  private readonly tokenNodes = new Map<string, { node: Container; look: string }>();
  /** Token pictures by URL: a texture once loaded, or the load in progress. */
  private readonly textures = new Map<string, Texture | "loading" | "failed">();
  private lastTokens: readonly RenderToken[] = [];
  /** Size of the current map, in world pixels; the grid is drawn over this area. */
  private mapSize = { width: 0, height: 0 };

  // `private constructor` + `static async create` is a common pattern when setup
  // must await something: a constructor can't be async.
  private constructor(private readonly app: Application) {
    this.world.addChild(this.mapLayer, this.gridLayer, this.tokenLayer);
    app.stage.addChild(this.world);
  }

  /** Creates a renderer that fills `host` and follows its size. */
  static async create(host: HTMLElement): Promise<Renderer> {
    const app = new Application();
    await app.init({
      resizeTo: host,
      background: "#0d0f14",
      antialias: true,
      // Draw at the screen's real pixel density (2× or 3× on iPads), but keep
      // CSS sizes in ordinary CSS pixels.
      resolution: window.devicePixelRatio,
      autoDensity: true,
    });
    app.canvas.classList.add("map-canvas");
    host.appendChild(app.canvas);
    return new Renderer(app);
  }

  /** The canvas element, for attaching input handlers. */
  get canvas(): HTMLCanvasElement {
    return this.app.canvas;
  }

  /** The drawing area's size in CSS pixels. */
  get viewSize(): { width: number; height: number } {
    return { width: this.app.screen.width, height: this.app.screen.height };
  }

  /** Replaces the map picture and grid. */
  setMap(map: LoadedMap): void {
    this.clearMap();
    for (const tile of map.tiles) {
      const sprite = new Sprite(Texture.from(tile.bitmap));
      sprite.position.set(tile.x, tile.y);
      this.mapLayer.addChild(sprite);
    }
    this.mapSize = { width: map.width, height: map.height };
    this.setGrid(map.grid);
  }

  clearMap(): void {
    // `destroy({ texture: true })` also frees each tile's GPU texture.
    for (const child of this.mapLayer.removeChildren()) child.destroy({ texture: true });
    this.gridLayer.clear();
    this.mapSize = { width: 0, height: 0 };
  }

  setGridVisible(visible: boolean): void {
    this.gridLayer.visible = visible;
  }

  /** Moves the view. Pixi redraws on its next frame. */
  setCamera(cam: Camera): void {
    this.world.scale.set(cam.scale);
    this.world.position.set(-cam.x * cam.scale, -cam.y * cam.scale);
  }

  /**
   * Shows exactly these tokens. Tokens whose look hasn't changed are only
   * moved, so calling this on every frame of a drag stays cheap.
   */
  setTokens(tokens: readonly RenderToken[]): void {
    this.lastTokens = tokens;
    const seen = new Set<string>();
    for (const t of tokens) {
      seen.add(t.key);
      const look = this.lookOf(t);
      let entry = this.tokenNodes.get(t.key);
      if (!entry || entry.look !== look) {
        entry?.node.destroy({ children: true });
        entry = { node: this.buildToken(t), look };
        this.tokenNodes.set(t.key, entry);
        this.tokenLayer.addChild(entry.node);
      }
      entry.node.position.set(t.x, t.y);
    }
    for (const [key, entry] of this.tokenNodes) {
      if (!seen.has(key)) {
        entry.node.destroy({ children: true });
        this.tokenNodes.delete(key);
      }
    }
    // Draw in the order given, so the caller decides what's on top.
    tokens.forEach((t, i) => {
      const node = this.tokenNodes.get(t.key)?.node;
      if (node) this.tokenLayer.setChildIndex(node, i);
    });
  }

  /** Everything about a token's appearance except where it is. */
  private lookOf(t: RenderToken): string {
    const texture = t.imageUrl ? this.texture(t.imageUrl) : null;
    const ready = texture instanceof Texture;
    return [t.size, t.name, t.imageUrl, ready, t.color, t.hidden, t.selected, t.ghost, t.label].join("|");
  }

  /** The texture for a token picture, starting the download the first time. */
  private texture(url: string): Texture | "loading" | "failed" {
    const known = this.textures.get(url);
    if (known) return known;
    this.textures.set(url, "loading");
    fetch(url, { credentials: "same-origin" })
      .then((res) => (res.ok ? res.blob() : Promise.reject(new Error(String(res.status)))))
      .then((blob) => createImageBitmap(blob))
      .then((bitmap) => {
        this.textures.set(url, Texture.from(bitmap));
        this.setTokens(this.lastTokens); // Redraw with the picture.
      })
      .catch(() => this.textures.set(url, "failed"));
    return "loading";
  }

  /** Builds a token: a disc with a picture or an initial, a ring, and a name label. */
  private buildToken(t: RenderToken): Container {
    const node = new Container();
    const r = t.size / 2;
    node.alpha = t.ghost ? 0.35 : t.hidden ? 0.5 : 1;

    const disc = new Graphics().circle(r, r, r * 0.92).fill({ color: t.color });
    node.addChild(disc);
    const texture = t.imageUrl ? this.texture(t.imageUrl) : null;
    if (texture instanceof Texture) {
      const sprite = new Sprite(texture);
      sprite.width = t.size;
      sprite.height = t.size;
      // A circular mask crops the square picture to the disc.
      const mask = new Graphics().circle(r, r, r * 0.92).fill({ color: 0xffffff });
      sprite.mask = mask;
      node.addChild(sprite, mask);
    } else {
      const initial = new Text({
        text: t.name.slice(0, 1).toUpperCase(),
        style: { fill: 0xffffff, fontSize: t.size * 0.45, fontWeight: "700", fontFamily: "system-ui, sans-serif" },
      });
      initial.anchor.set(0.5);
      initial.position.set(r, r);
      node.addChild(initial);
    }

    const ringColor = t.selected ? 0x00e5ff : 0x0d0f14;
    node.addChild(
      new Graphics().circle(r, r, r * 0.92).stroke({ color: ringColor, width: Math.max(2, t.size * (t.selected ? 0.07 : 0.04)) }),
    );

    if (!t.ghost) {
      const fontSize = Math.max(10, t.size * 0.2);
      const name = new Text({ text: t.name, style: { fill: 0xffffff, fontSize, fontFamily: "system-ui, sans-serif" } });
      name.anchor.set(0.5, 0);
      name.position.set(r, t.size + 2);
      const pad = fontSize * 0.25;
      const bg = new Graphics()
        .roundRect(r - name.width / 2 - pad, t.size + 2 - pad / 2, name.width + pad * 2, name.height + pad, pad)
        .fill({ color: 0x0d0f14, alpha: 0.7 });
      node.addChild(bg, name);
    }
    if (t.label) {
      const fontSize = Math.max(12, t.size * 0.25);
      const label = new Text({ text: t.label, style: { fill: 0x00e5ff, fontSize, fontWeight: "700", fontFamily: "system-ui, sans-serif" } });
      label.anchor.set(0.5, 1);
      label.position.set(r, -4);
      node.addChild(label);
    }
    return node;
  }

  destroy(): void {
    for (const { node } of this.tokenNodes.values()) node.destroy({ children: true });
    this.tokenNodes.clear();
    for (const t of this.textures.values()) if (t instanceof Texture) t.destroy(true);
    this.clearMap();
    this.app.destroy(true, { children: true });
  }

  /**
   * Draws a line on every grid square boundary that falls inside the map.
   * Called on its own when the GM adjusts the grid, without reloading the picture.
   */
  setGrid(grid: GridSpec): void {
    const { pixelsPerGrid: ppg, origin } = grid;
    const { width, height } = this.mapSize;
    const g = this.gridLayer;
    g.clear();
    // Grid lines sit at whole grid numbers. With a fractional origin the first
    // line isn't at pixel 0, so work out where the first and last lines fall.
    for (let k = Math.ceil(origin.x); (k - origin.x) * ppg <= width + 0.5; k++) {
      const x = (k - origin.x) * ppg;
      g.moveTo(x, 0).lineTo(x, height);
    }
    for (let k = Math.ceil(origin.y); (k - origin.y) * ppg <= height + 0.5; k++) {
      const y = (k - origin.y) * ppg;
      g.moveTo(0, y).lineTo(width, y);
    }
    // `pixelLine` keeps lines exactly 1 screen pixel wide at every zoom level.
    g.stroke({ color: 0xffffff, alpha: 0.35, width: 1, pixelLine: true });
  }
}

/** A token as the renderer needs it: already in world pixels. */
export interface RenderToken {
  /** Unique per drawn object (a token and its drag ghost are two). */
  key: string;
  /** Top-left corner and side length, in world pixels. */
  x: number;
  y: number;
  size: number;
  name: string;
  imageUrl: string | null;
  /** Disc colour, shown when there's no picture or while it loads. */
  color: number;
  /** Hidden from players: drawn faded for the GM. */
  hidden: boolean;
  selected: boolean;
  /** The faint copy left at the start of a drag. */
  ghost: boolean;
  /** Text above the token, such as the drag distance. */
  label: string | null;
}
