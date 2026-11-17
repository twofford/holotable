import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { campaigns } from "../api/campaigns";
import { errorMessage } from "../api/client";
import { useCurrentUser } from "../auth/currentUser";
import { useLoad } from "../hooks/useLoad";
import { MapCanvas, type MapCanvasHandle } from "../map/MapCanvas";
import { MapInfo } from "../map/MapInfo";
import { type GridSpec, type ScreenPoint, worldToGrid } from "../map/coords";
import { loadSceneMap, sceneGrid } from "../map/loadScene";
import { type LoadedMap, releaseMap } from "../map/loadUvtt";
import type { PointerInfo } from "../map/panZoom";
import { useConnectionContext } from "../net/connectionContext";
import type { SceneDetail } from "../protocol/generated/SceneDetail";
import type { Token } from "../protocol/generated/Token";
import type { TokenSettings } from "../protocol/generated/TokenSettings";
import { TokenDialog } from "../scene/TokenDialog";
import { renderTokens } from "../scene/renderTokens";
import { SceneStore } from "../scene/sceneStore";
import { type DragState, TokenDragController, tokenAt } from "../scene/tokenDrag";

type Dialog = { mode: "create" } | { mode: "edit"; token: Token } | null;

/**
 * A scene: its map from the server, with pan and zoom, live tokens, and grid
 * settings for GMs.
 */
export function ScenePage() {
  const params = useParams();
  const campaignId = Number(params.campaignId);
  const sceneId = Number(params.sceneId);
  const canvas = useRef<MapCanvasHandle>(null);
  const connection = useConnectionContext();
  const me = useCurrentUser();
  const navigate = useNavigate();

  // The campaign gives members (token owners) and the active scene; the scene is what to draw.
  const [campaign, reloadCampaign] = useLoad(() => campaigns.get(campaignId), [campaignId]);
  const [scene, setScene] = useState<SceneDetail | null>(null);
  const [map, setMap] = useState<LoadedMap | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [gridVisible, setGridVisible] = useState(true);
  const [zoom, setZoom] = useState(100);
  const [snap, setSnap] = useState(true);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [drag, setDrag] = useState<DragState | null>(null);
  const [menu, setMenu] = useState<{ tokenId: number; at: ScreenPoint } | null>(null);
  const [dialog, setDialog] = useState<Dialog>(null);

  // Load the scene and its tiles. Free the tiles when leaving or switching scenes.
  useEffect(() => {
    let cancelled = false;
    let loaded: LoadedMap | null = null;
    setMap(null);
    setError(null);
    (async () => {
      try {
        const detail = await campaigns.scene(campaignId, sceneId);
        const m = await loadSceneMap(campaignId, detail);
        if (cancelled) {
          releaseMap(m);
          return;
        }
        loaded = m;
        setScene(detail);
        setMap(m);
      } catch (err) {
        if (!cancelled) setError(errorMessage(err));
      }
    })();
    return () => {
      cancelled = true;
      if (loaded) releaseMap(loaded);
    };
  }, [campaignId, sceneId]);

  // The GM changed the scene (its name or grid): fetch it again. The picture
  // stays, so there's no need to reload the tiles.
  const refetchScene = useCallback(() => {
    campaigns
      .scene(campaignId, sceneId)
      .then((detail) => {
        setScene(detail);
        canvas.current?.setGrid(sceneGrid(detail.pixelsPerGrid, detail.gridOffsetX, detail.gridOffsetY));
      })
      .catch(() => undefined);
  }, [campaignId, sceneId]);

  // One live store per scene. `useMemo` keeps the same store across renders;
  // the effect joins the scene on mount and leaves it on unmount.
  const store = useMemo(() => new SceneStore(connection, sceneId, refetchScene), [connection, sceneId, refetchScene]);
  useEffect(() => {
    store.start();
    return () => store.stop();
  }, [store]);
  const view = useSyncExternalStore(store.subscribe, store.getSnapshot);

  const isGm = view.role === "gm" || (view.role === null && campaign.kind === "ready" && campaign.data.role === "gm");
  const grid: GridSpec | null = scene ? sceneGrid(scene.pixelsPerGrid, scene.gridOffsetX, scene.gridOffsetY) : null;

  // Players follow the GM to the scene they put everyone on (MAP-010).
  const [activeSceneId, setActiveSceneId] = useState<number | null>(null);
  useEffect(() => {
    if (campaign.kind === "ready") setActiveSceneId(campaign.data.activeSceneId);
  }, [campaign]);
  useEffect(
    () =>
      connection.onMessage((msg) => {
        if (msg.type !== "activeScene" || msg.campaignId !== campaignId) return;
        setActiveSceneId(msg.sceneId);
        if (!isGm && msg.sceneId !== sceneId) navigate(`/campaigns/${campaignId}/scenes/${msg.sceneId}`);
      }),
    [connection, campaignId, sceneId, isGm, navigate],
  );

  // Drop the selection if the token disappears (deleted, or hidden from me).
  useEffect(() => {
    if (selectedId !== null && !view.tokens.some((t) => t.id === selectedId)) setSelectedId(null);
  }, [view.tokens, selectedId]);

  // A refusal shows for a few seconds.
  useEffect(() => {
    if (!view.notice) return;
    const timer = setTimeout(() => store.clearNotice(), 4000);
    return () => clearTimeout(timer);
  }, [view.notice, store]);

  // The drag controller lives as long as the page. It reads the latest values
  // through this ref, so it never works from a stale render.
  const latest = useRef({ tokens: view.tokens, grid, isGm, snap });
  latest.current = { tokens: view.tokens, grid, isGm, snap };
  const dragController = useMemo(
    () =>
      new TokenDragController({
        tokens: () => latest.current.tokens,
        // Only used while the scene is loaded (see `pointerHandler` below).
        grid: () => latest.current.grid ?? sceneGrid(1, 0, 0),
        canMove: (t) => latest.current.isGm || t.ownerId === me.id,
        snapEnabled: () => latest.current.snap,
        select: (id) => {
          setSelectedId(id);
          setMenu(null);
        },
        onDragChange: setDrag,
        drag: (id, x, y) => store.drag(id, x, y),
        move: (id, x, y) => store.move(id, x, y),
        openMenu: (id, at) => latest.current.isGm && setMenu({ tokenId: id, at }),
      }),
    [store, me.id],
  );

  const onTap = useCallback((p: PointerInfo) => {
    setMenu(null);
    const g = latest.current.grid;
    setSelectedId(g ? (tokenAt(latest.current.tokens, g, p.world)?.id ?? null) : null);
  }, []);
  const onContextMenu = useCallback((p: PointerInfo) => {
    const g = latest.current.grid;
    const t = g ? tokenAt(latest.current.tokens, g, p.world) : null;
    if (!t || !latest.current.isGm) return setMenu(null);
    setSelectedId(t.id);
    setMenu({ tokenId: t.id, at: p.screen });
  }, []);

  // Keyboard: Escape clears the selection; Delete removes the selected token (GM).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (dialog || e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;
      if (e.key === "Escape") {
        setSelectedId(null);
        setMenu(null);
      } else if ((e.key === "Delete" || e.key === "Backspace") && isGm && selectedId !== null) {
        store.remove(selectedId);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dialog, isGm, selectedId, store]);

  const rendered = useMemo(
    () =>
      grid
        ? renderTokens({
            tokens: view.tokens,
            grid,
            selectedId,
            drag,
            remote: view.dragging,
            imageUrl: (assetId) => campaigns.tileUrl(campaignId, assetId, 0),
          })
        : [],
    // `grid` is rebuilt every render; its inputs are what matter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [view.tokens, view.dragging, selectedId, drag, scene, campaignId],
  );

  function addToken(settings: TokenSettings) {
    if (!grid) return;
    // New tokens go in the middle of the view, on a whole square.
    const c = canvas.current ? worldToGrid(canvas.current.viewCenter(), grid) : { x: 0, y: 0 };
    store.create(settings, Math.round(c.x - settings.size / 2), Math.round(c.y - settings.size / 2));
    setDialog(null);
  }

  async function playHere() {
    try {
      await campaigns.setActiveScene(campaignId, sceneId);
      setActiveSceneId(sceneId);
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  const menuToken = menu ? view.tokens.find((t) => t.id === menu.tokenId) : undefined;
  const settingsOf = (t: Token): TokenSettings => ({ name: t.name, assetId: t.assetId, size: t.size, ownerId: t.ownerId, hidden: t.hidden });
  const members = campaign.kind === "ready" ? campaign.data.members : [];

  return (
    <div className="map-view">
      <div className="toolbar">
        <Link to={`/campaigns/${campaignId}`} className="button secondary">
          ← {campaign.kind === "ready" ? campaign.data.name : "Campaign"}
        </Link>
        <strong className="scene-title">{scene?.name ?? ""}</strong>
        {activeSceneId === sceneId && <span className="badge now-playing">Now playing</span>}
        <span className="spacer" />
        {isGm && activeSceneId !== sceneId && (
          <button type="button" className="secondary" onClick={playHere} disabled={!scene}>
            Play for everyone
          </button>
        )}
        {isGm && (
          <button type="button" onClick={() => setDialog({ mode: "create" })} disabled={!scene || view.status !== "live"}>
            Add token
          </button>
        )}
        <button type="button" className="secondary" onClick={() => setSnap((v) => !v)} aria-pressed={snap} title="Hold Alt while dropping to skip snapping once">
          Snap {snap ? "on" : "off"}
        </button>
        <button type="button" className="secondary" onClick={() => setGridVisible((v) => !v)} aria-pressed={gridVisible}>
          Grid {gridVisible ? "on" : "off"}
        </button>
        <button type="button" className="secondary" onClick={() => canvas.current?.fit()} disabled={!map}>
          Fit
        </button>
        <span className="zoom" aria-label="Zoom">
          {zoom}%
        </span>
      </div>
      {isGm && scene && (
        <GridSettings
          campaignId={campaignId}
          scene={scene}
          onPreview={(ppg, x, y) => canvas.current?.setGrid(sceneGrid(ppg, x, y))}
          onSaved={setScene}
        />
      )}
      <div className="map-area">
        <MapCanvas
          ref={canvas}
          map={map}
          gridVisible={gridVisible}
          tokens={rendered}
          pointerHandler={view.status === "live" && grid ? dragController : null}
          onTap={onTap}
          onContextMenu={onContextMenu}
          onZoom={setZoom}
        >
          {!map && !error && <div className="overlay center">Loading the map…</div>}
          {error && <div className="overlay center error">{error}</div>}
          {map && <MapInfo map={map} />}
          {map && view.status === "joining" && <div className="overlay top">Joining the table…</div>}
          {view.notice && (
            <div className="toast" role="status">
              {view.notice}
              <button type="button" className="secondary" onClick={() => store.clearNotice()} aria-label="Dismiss">
                ×
              </button>
            </div>
          )}
          {menu && menuToken && (
            <div className="token-menu" role="menu" style={{ left: menu.at.x, top: menu.at.y }}>
              <button type="button" role="menuitem" onClick={() => (setDialog({ mode: "edit", token: menuToken }), setMenu(null))}>
                Edit…
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => (store.update(menuToken.id, { ...settingsOf(menuToken), hidden: !menuToken.hidden }), setMenu(null))}
              >
                {menuToken.hidden ? "Show to players" : "Hide from players"}
              </button>
              <button type="button" role="menuitem" className="danger" onClick={() => (store.remove(menuToken.id), setMenu(null))}>
                Delete
              </button>
            </div>
          )}
        </MapCanvas>
        {view.status === "kicked" && view.kicked && (
          <div className="modal-backdrop">
            <div className="modal">
              <h3>You left the table</h3>
              <p>{view.kicked.reason}</p>
              <Link to="/" className="button" onClick={() => reloadCampaign()}>
                Back to campaigns
              </Link>
            </div>
          </div>
        )}
      </div>
      {dialog && (
        <TokenDialog
          campaignId={campaignId}
          initial={dialog.mode === "edit" ? settingsOf(dialog.token) : null}
          members={members}
          onClose={() => setDialog(null)}
          onSave={(settings) => {
            if (dialog.mode === "create") addToken(settings);
            else {
              store.update(dialog.token.id, settings);
              setDialog(null);
            }
          }}
        />
      )}
    </div>
  );
}

interface GridSettingsProps {
  campaignId: number;
  scene: SceneDetail;
  /** Redraw the grid with these values, without saving. */
  onPreview: (pixelsPerGrid: number, offsetX: number, offsetY: number) => void;
  onSaved: (scene: SceneDetail) => void;
}

/**
 * GM controls to line the grid up with the map picture (MAP-009). Changes show
 * on the map at once; Save stores them for everyone.
 */
function GridSettings({ campaignId, scene, onPreview, onSaved }: GridSettingsProps) {
  const [ppg, setPpg] = useState(scene.pixelsPerGrid);
  const [x, setX] = useState(scene.gridOffsetX);
  const [y, setY] = useState(scene.gridOffsetY);
  const [status, setStatus] = useState<string | null>(null);
  const changed = ppg !== scene.pixelsPerGrid || x !== scene.gridOffsetX || y !== scene.gridOffsetY;

  // Preview every change that makes sense. An empty or zero field doesn't.
  useEffect(() => {
    if (ppg >= 8) onPreview(ppg, x, y);
  }, [ppg, x, y, onPreview]);

  async function save() {
    setStatus(null);
    try {
      onSaved(await campaigns.updateScene(campaignId, scene.id, { pixelsPerGrid: ppg, gridOffsetX: x, gridOffsetY: y }));
      setStatus("Saved.");
    } catch (err) {
      setStatus(errorMessage(err));
    }
  }

  function reset() {
    setPpg(scene.pixelsPerGrid);
    setX(scene.gridOffsetX);
    setY(scene.gridOffsetY);
  }

  const num = (set: (n: number) => void) => (e: React.ChangeEvent<HTMLInputElement>) => set(Number(e.target.value));
  return (
    <div className="toolbar grid-settings">
      <span className="muted">Grid</span>
      <label>
        Square size (px)
        <input type="number" min={8} max={1000} step={0.5} value={ppg} onChange={num(setPpg)} />
      </label>
      <label>
        Offset X
        <input type="number" min={-1000} max={1000} step={0.5} value={x} onChange={num(setX)} />
      </label>
      <label>
        Offset Y
        <input type="number" min={-1000} max={1000} step={0.5} value={y} onChange={num(setY)} />
      </label>
      <button type="button" onClick={save} disabled={!changed}>
        Save grid
      </button>
      <button type="button" className="secondary" onClick={reset} disabled={!changed}>
        Reset
      </button>
      {status && <span className="hint">{status}</span>}
    </div>
  );
}
