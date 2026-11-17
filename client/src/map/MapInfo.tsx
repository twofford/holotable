import type { LoadedMap } from "./loadUvtt";

/** The small panel in the corner of the map: size, tiles, walls, doors, lights. */
export function MapInfo({ map }: { map: LoadedMap }) {
  const { map_size } = map.uvtt.resolution;
  const tiles = map.tiles.length;
  return (
    <div className="overlay info">
      <strong>{map.name}</strong> · {map_size.x}×{map_size.y} squares · {map.width}×{map.height} px · {tiles} tile
      {tiles === 1 ? "" : "s"} · {map.uvtt.line_of_sight.length} walls · {map.uvtt.portals.length} doors ·{" "}
      {map.uvtt.lights.length} lights
      {map.warnings.map((w) => (
        <div key={w} className="warning">
          {w}
        </div>
      ))}
    </div>
  );
}
