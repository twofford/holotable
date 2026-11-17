import { describe, expect, it } from "vitest";
import { MapLoadError, parseUvtt } from "./loadUvtt";

const minimal = {
  format: 0.3,
  resolution: { map_origin: { x: 0, y: 0 }, map_size: { x: 2, y: 1 }, pixels_per_grid: 50 },
  image: "iVBORw0KGgo=",
};

describe("parseUvtt", () => {
  it("fills in missing optional fields", () => {
    const map = parseUvtt(JSON.stringify(minimal));
    expect(map.line_of_sight).toEqual([]);
    expect(map.portals).toEqual([]);
    expect(map.environment).toBeNull();
  });

  it("rejects text that isn't JSON", () => {
    expect(() => parseUvtt("not json")).toThrow(MapLoadError);
  });

  it("rejects a zero grid size", () => {
    const bad = { ...minimal, resolution: { ...minimal.resolution, pixels_per_grid: 0 } };
    expect(() => parseUvtt(JSON.stringify(bad))).toThrow(/pixels_per_grid/);
  });

  it("rejects a file with no image", () => {
    expect(() => parseUvtt(JSON.stringify({ ...minimal, image: "" }))).toThrow(/image/);
  });
});
