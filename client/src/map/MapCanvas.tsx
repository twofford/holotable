import { type ReactNode, type Ref, useCallback, useEffect, useImperativeHandle, useRef, useState } from "react";
import { type RenderToken, Renderer } from "../render/renderer";
import { type Camera, type GridSpec, type ScreenPoint, type WorldPoint, fitCamera, screen, screenToWorld, worldToScreen } from "./coords";
import type { LoadedMap } from "./loadUvtt";
import { type PointerHandler, type PointerInfo, PanZoomController } from "./panZoom";

/** What a parent can ask the canvas to do, through a ref. */
export interface MapCanvasHandle {
  /** Show the whole map. */
  fit(): void;
  /** Redraw the grid only, e.g. while the GM adjusts it. */
  setGrid(grid: GridSpec): void;
  /** The world point at the middle of the view: where new tokens go. */
  viewCenter(): WorldPoint;
  /** Where a world point is on screen, e.g. to place a menu next to a token. */
  toScreen(p: WorldPoint): ScreenPoint;
}

interface Props {
  /** The map to show. `null` shows an empty table. */
  map: LoadedMap | null;
  gridVisible: boolean;
  /** Called with the rounded zoom percentage when it changes. */
  onZoom?: (percent: number) => void;
  /** Called once the renderer is ready to show maps. */
  onReady?: () => void;
  /** Tokens to draw, already in world pixels. */
  tokens?: readonly RenderToken[];
  /** Gets the first chance at each press, e.g. to drag a token. */
  pointerHandler?: PointerHandler | null;
  /** A tap or click that didn't pan or drag anything. */
  onTap?: (p: PointerInfo) => void;
  /** Right-click on desktop. */
  onContextMenu?: (p: PointerInfo) => void;
  /** Drawn on top of the map (status messages, info panels). */
  children?: ReactNode;
  /** In React 19 `ref` is an ordinary prop for function components. */
  ref?: Ref<MapCanvasHandle>;
}

/**
 * The map: a PixiJS canvas with pan and zoom. Pages decide which map to show
 * and put their own toolbars around it.
 *
 * The camera changes on every frame of a drag, so it lives in a ref, not in
 * React state: updating a ref doesn't re-render.
 */
export function MapCanvas({ map, gridVisible, tokens, pointerHandler, onTap, onContextMenu, onZoom, onReady, children, ref }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const rendererRef = useRef<Renderer | null>(null);
  const cameraRef = useRef<Camera>({ x: 0, y: 0, scale: 1 });
  const mapRef = useRef<LoadedMap | null>(null);
  const [ready, setReady] = useState(false);

  // Keep the latest callbacks in refs, so the long-lived renderer setup below
  // doesn't have to restart when a parent passes a new function.
  const onZoomRef = useRef(onZoom);
  onZoomRef.current = onZoom;
  const onReadyRef = useRef(onReady);
  onReadyRef.current = onReady;
  const handlerRef = useRef(pointerHandler);
  handlerRef.current = pointerHandler;
  const onTapRef = useRef(onTap);
  onTapRef.current = onTap;
  const onContextMenuRef = useRef(onContextMenu);
  onContextMenuRef.current = onContextMenu;

  const setCamera = useCallback((cam: Camera) => {
    cameraRef.current = cam;
    rendererRef.current?.setCamera(cam);
    onZoomRef.current?.(Math.round(cam.scale * 100));
  }, []);

  const fit = useCallback(() => {
    const renderer = rendererRef.current;
    const current = mapRef.current;
    if (!renderer || !current) return;
    const { width, height } = renderer.viewSize;
    setCamera(fitCamera(current.width, current.height, width, height));
  }, [setCamera]);

  // `useImperativeHandle` decides what the parent's ref sees: just these two
  // methods, not the renderer or the DOM.
  useImperativeHandle(
    ref,
    () => ({
      fit,
      setGrid: (grid) => rendererRef.current?.setGrid(grid),
      viewCenter: () => {
        const size = rendererRef.current?.viewSize ?? { width: 0, height: 0 };
        return screenToWorld(cameraRef.current, screen(size.width / 2, size.height / 2));
      },
      toScreen: (p) => worldToScreen(cameraRef.current, p),
    }),
    [fit],
  );

  // Create the renderer and input handling once; tear them down on unmount.
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let cancelled = false;
    let controller: PanZoomController | null = null;

    Renderer.create(host).then((renderer) => {
      // In development React mounts, unmounts and mounts again to catch missing
      // cleanup. If we were unmounted while Pixi was starting, throw this one away.
      if (cancelled) {
        renderer.destroy();
        return;
      }
      rendererRef.current = renderer;
      controller = new PanZoomController(renderer.canvas, {
        getCamera: () => cameraRef.current,
        setCamera,
        getHandler: () => handlerRef.current ?? null,
        onTap: (p) => onTapRef.current?.(p),
        onContextMenu: (p) => onContextMenuRef.current?.(p),
      });
      setReady(true);
      onReadyRef.current?.();
    });

    return () => {
      cancelled = true;
      controller?.destroy();
      rendererRef.current?.destroy();
      rendererRef.current = null;
    };
  }, [setCamera]);

  // Show a new map when it changes, and fit it to the view.
  useEffect(() => {
    const renderer = rendererRef.current;
    if (!renderer || !ready) return;
    mapRef.current = map;
    if (map) {
      renderer.setMap(map);
      fit();
    } else {
      renderer.clearMap();
    }
  }, [map, ready, fit]);

  useEffect(() => {
    rendererRef.current?.setGridVisible(gridVisible);
  }, [gridVisible, ready, map]);

  useEffect(() => {
    if (ready) rendererRef.current?.setTokens(tokens ?? []);
  }, [tokens, ready]);

  return (
    <div className="map-host" ref={hostRef}>
      {children}
    </div>
  );
}
