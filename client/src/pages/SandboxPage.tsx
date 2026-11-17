import { useRef, useState } from "react";
import { Link } from "react-router";
import { MapCanvas, type MapCanvasHandle } from "../map/MapCanvas";
import { type LoadedMap, MapLoadError, loadMap, releaseMap } from "../map/loadUvtt";
import { MapInfo } from "../map/MapInfo";
import { SAMPLE_MAPS } from "../map/samples";

type Status = { kind: "empty" } | { kind: "loading"; name: string } | { kind: "error"; message: string } | { kind: "ready" };

/**
 * Open a map file from this device, without uploading it (the M1 viewer).
 * Handy for checking a map before adding it to a campaign.
 */
export function SandboxPage() {
  const canvas = useRef<MapCanvasHandle>(null);
  const loadCounter = useRef(0);
  const [map, setMap] = useState<LoadedMap | null>(null);
  const [status, setStatus] = useState<Status>({ kind: "empty" });
  const [ready, setReady] = useState(false);
  const [gridVisible, setGridVisible] = useState(true);
  const [zoom, setZoom] = useState(100);

  /** Loads a map from its file text. Only the most recent request wins. */
  async function open(name: string, getText: () => Promise<string>) {
    const myLoad = ++loadCounter.current;
    setStatus({ kind: "loading", name });
    try {
      const loaded = await loadMap(name, await getText());
      if (myLoad !== loadCounter.current) {
        releaseMap(loaded); // A newer load started while this one ran.
        return;
      }
      // `setMap` with a function gets the previous value, so the old map's
      // memory can be freed as the new one replaces it.
      setMap((old) => {
        if (old) releaseMap(old);
        return loaded;
      });
      setStatus({ kind: "ready" });
    } catch (err) {
      if (myLoad !== loadCounter.current) return;
      const message = err instanceof MapLoadError ? err.message : `Couldn't open ${name}: ${String(err)}`;
      setStatus({ kind: "error", message });
    }
  }

  const openFile = (file: File) => open(file.name, () => file.text());

  return (
    <div className="map-view">
      <div className="toolbar">
        <Link to="/" className="button secondary">
          ← Campaigns
        </Link>
        <label className={`button ${ready ? "" : "disabled"}`}>
          Open map…
          <input
            type="file"
            accept=".uvtt,.dd2vtt,.json"
            disabled={!ready}
            onChange={(e) => {
              const file = e.target.files?.[0];
              if (file) openFile(file);
              e.target.value = ""; // So picking the same file again still fires.
            }}
          />
        </label>
        {SAMPLE_MAPS.map((s) => (
          <button
            key={s.name}
            type="button"
            className="secondary"
            disabled={!ready}
            onClick={() => open(s.name, () => fetch(s.url).then((r) => r.text()))}
          >
            {s.label}
          </button>
        ))}
        <span className="spacer" />
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

      <div
        className="map-area"
        onDragOver={(e) => e.preventDefault()}
        onDrop={(e) => {
          e.preventDefault();
          const file = e.dataTransfer.files[0];
          if (file) openFile(file);
        }}
      >
        <MapCanvas ref={canvas} map={map} gridVisible={gridVisible} onZoom={setZoom} onReady={() => setReady(true)}>
          {status.kind === "empty" && (
            <div className="overlay center">
              Open a .uvtt or .dd2vtt file from this device, drop one here, or pick a sample. Nothing is uploaded.
            </div>
          )}
          {status.kind === "loading" && <div className="overlay center">Loading {status.name}…</div>}
          {status.kind === "error" && <div className="overlay center error">{status.message}</div>}
          {status.kind === "ready" && map && <MapInfo map={map} />}
        </MapCanvas>
      </div>
    </div>
  );
}
