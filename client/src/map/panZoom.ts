// Pan and zoom input, with Pointer Events.
//
// Pointer Events give mouse, touch and Apple Pencil one shared API, so a single
// code path handles all of them (spec MAP-006):
// - one pointer dragging (mouse left/middle button, a finger, the Pencil) pans;
// - two fingers pinch to zoom and drag to pan, at the same time;
// - the mouse wheel zooms toward the pointer; a trackpad's two-finger scroll
//   pans and its pinch zooms (macOS reports a trackpad pinch as a wheel event
//   with ctrlKey set).

import { type Camera, type ScreenPoint, type WorldPoint, panBy, pinch, screen, screenToWorld, zoomAt } from "./coords";

/** Where a pointer is, in both spaces, and what's held down. */
export interface PointerInfo {
  readonly screen: ScreenPoint;
  readonly world: WorldPoint;
  readonly altKey: boolean;
  readonly pointerType: string;
}

/**
 * Something on the map that can take over a pointer, such as a token being
 * dragged. `down` returns true to claim the pointer; that pointer then goes to
 * `move` and `up` instead of panning.
 */
export interface PointerHandler {
  down(p: PointerInfo): boolean;
  move(p: PointerInfo): void;
  up(p: PointerInfo): void;
  /** The pointer was lost (a second finger, the browser took over). Undo. */
  cancel(): void;
}

export interface PanZoomOptions {
  readonly getCamera: () => Camera;
  readonly setCamera: (cam: Camera) => void;
  /** Read on every pointer-down, so the current handler is always used. */
  readonly getHandler?: () => PointerHandler | null;
  /** A quick press and release that didn't pan: a tap or click. */
  readonly onTap?: (p: PointerInfo) => void;
  /** Right-click (desktop). Long-press on touch is the handler's job. */
  readonly onContextMenu?: (p: PointerInfo) => void;
}

/** A press that moves less than this, in screen pixels, is a tap, not a pan. */
const TAP_SLOP_PX = 6;
const TAP_MAX_MS = 500;

/** Wheel deltas in "lines" (Firefox with a mouse) are converted at this rate. */
const LINE_HEIGHT_PX = 16;

export class PanZoomController {
  /** Pointers currently down, by pointerId, at their last known position. */
  private readonly pointers = new Map<number, ScreenPoint>();
  /** The pointer a handler claimed, if any. */
  private claimed: number | null = null;
  /** Where and when the current single-pointer press started, to spot taps. */
  private press: { id: number; at: number; start: ScreenPoint; moved: number } | null = null;
  /** Removing every listener at once: `abort()` detaches all that used `signal`. */
  private readonly abort = new AbortController();

  constructor(
    private readonly el: HTMLElement,
    private readonly opts: PanZoomOptions,
  ) {
    const signal = this.abort.signal;
    el.addEventListener("pointerdown", this.onPointerDown, { signal });
    el.addEventListener("pointermove", this.onPointerMove, { signal });
    el.addEventListener("pointerup", this.onPointerEnd, { signal });
    el.addEventListener("pointercancel", this.onPointerEnd, { signal });
    // `passive: false` because we call preventDefault() to stop the page itself
    // from scrolling or zooming.
    el.addEventListener("wheel", this.onWheel, { signal, passive: false });
    // Safari on iPad and macOS fires its own gesture events for pinches. Cancel
    // them so the page doesn't zoom; our pointer handling does the zooming.
    for (const type of ["gesturestart", "gesturechange", "gestureend"]) {
      el.addEventListener(type, (e) => e.preventDefault(), { signal });
    }
    // Right-click opens our menu, not the browser's.
    el.addEventListener(
      "contextmenu",
      (e) => {
        e.preventDefault();
        this.opts.onContextMenu?.(this.info(e, "mouse"));
      },
      { signal },
    );
  }

  destroy(): void {
    this.abort.abort();
  }

  /** Converts an event's position into this element's screen space. */
  private toScreen(e: { clientX: number; clientY: number }): ScreenPoint {
    const rect = this.el.getBoundingClientRect();
    return screen(e.clientX - rect.left, e.clientY - rect.top);
  }

  private info(e: MouseEvent, pointerType: string): PointerInfo {
    const s = this.toScreen(e);
    return { screen: s, world: screenToWorld(this.opts.getCamera(), s), altKey: e.altKey, pointerType };
  }

  // Handlers are arrow-function properties so `this` stays bound to the
  // controller when the browser calls them.

  private onPointerDown = (e: PointerEvent) => {
    // Mouse: left (0) and middle (1); right (2) is the context menu.
    if (e.pointerType === "mouse" && e.button > 1) return;
    if (this.claimed !== null) return; // A token drag is in progress.
    if (this.pointers.size >= 2) return; // Ignore a third finger.
    // Capture: keep receiving this pointer's events even if it leaves the canvas.
    this.el.setPointerCapture(e.pointerId);

    // A first finger or a left click may belong to a token.
    if (this.pointers.size === 0 && e.button === 0 && this.opts.getHandler?.()?.down(this.info(e, e.pointerType))) {
      this.claimed = e.pointerId;
      return;
    }
    if (this.pointers.size === 1) {
      this.press = null; // A second finger: a pinch, not a tap.
    } else {
      this.press = { id: e.pointerId, at: performance.now(), start: this.toScreen(e), moved: 0 };
    }
    this.pointers.set(e.pointerId, this.toScreen(e));
  };

  private onPointerMove = (e: PointerEvent) => {
    if (e.pointerId === this.claimed) {
      this.opts.getHandler?.()?.move(this.info(e, e.pointerType));
      return;
    }
    const prev = this.pointers.get(e.pointerId);
    if (!prev) return;
    const cur = this.toScreen(e);
    if (this.press?.id === e.pointerId) {
      this.press.moved = Math.max(this.press.moved, Math.hypot(cur.x - this.press.start.x, cur.y - this.press.start.y));
    }

    if (this.pointers.size === 1) {
      this.update(panBy(this.opts.getCamera(), cur.x - prev.x, cur.y - prev.y));
    } else {
      // Two pointers: find the other one and treat the pair as a pinch.
      for (const [id, other] of this.pointers) {
        if (id !== e.pointerId) {
          this.update(pinch(this.opts.getCamera(), prev, other, cur, other));
          break;
        }
      }
    }
    this.pointers.set(e.pointerId, cur);
  };

  private onPointerEnd = (e: PointerEvent) => {
    if (e.pointerId === this.claimed) {
      this.claimed = null;
      const handler = this.opts.getHandler?.();
      if (e.type === "pointercancel") handler?.cancel();
      else handler?.up(this.info(e, e.pointerType));
      return;
    }
    this.pointers.delete(e.pointerId);
    const press = this.press;
    if (press?.id === e.pointerId) {
      this.press = null;
      const quick = performance.now() - press.at < TAP_MAX_MS;
      if (e.type === "pointerup" && quick && press.moved < TAP_SLOP_PX) this.opts.onTap?.(this.info(e, e.pointerType));
    }
  };

  private onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const scale = e.deltaMode === WheelEvent.DOM_DELTA_LINE ? LINE_HEIGHT_PX : 1;
    const dx = e.deltaX * scale;
    const dy = e.deltaY * scale;
    const cam = this.opts.getCamera();
    const anchor = this.toScreen(e);

    if (e.ctrlKey) {
      // Trackpad pinch (or Ctrl + wheel): small deltas, so zoom faster per unit.
      this.update(zoomAt(cam, anchor, Math.exp(-dy * 0.01)));
    } else if (looksLikeMouseWheel(e, dx, dy)) {
      this.update(zoomAt(cam, anchor, Math.exp(-dy * 0.0015)));
    } else {
      this.update(panBy(cam, -dx, -dy));
    }
  };

  private update(cam: Camera): void {
    this.opts.setCamera(cam);
  }
}

/**
 * Browsers don't say whether a wheel event came from a mouse wheel or a
 * trackpad, so this guesses. Mouse wheels scroll in "lines", or in large whole
 * steps on one axis; trackpads send small, often fractional, deltas on both
 * axes. A wrong guess only means a trackpad scroll zooms or a wheel pans.
 */
function looksLikeMouseWheel(e: WheelEvent, dx: number, dy: number): boolean {
  if (e.deltaMode === WheelEvent.DOM_DELTA_LINE) return true;
  return dx === 0 && Math.abs(dy) >= 50 && Number.isInteger(dy);
}
