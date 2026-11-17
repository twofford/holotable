import { describe, expect, it } from "vitest";
import { gridToWorld, grid } from "./coords";
import { sceneGrid } from "./loadScene";

describe("sceneGrid", () => {
  it("with no offset, grid lines start at the image corner", () => {
    expect(gridToWorld(grid(0, 0), sceneGrid(70, 0, 0))).toEqual({ x: 0, y: 0 });
  });

  it("an offset moves every grid line by that many pixels", () => {
    const spec = sceneGrid(50, 10, -5);
    const line3 = gridToWorld(grid(3, 3), spec);
    expect(line3.x).toBeCloseTo(160);
    expect(line3.y).toBeCloseTo(145);
  });
});
