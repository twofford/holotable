import { describe, expect, it } from "vitest";
import { doorAt, renderDoors } from "./doors";

describe("doors", () => {
  const doors = renderDoors(new Float64Array([0, 0, 100, 0, 200, 0, 200, 100]), [
    { open: false, locked: false },
    { open: true, locked: true },
  ]);

  it("pairs lines with states", () => {
    expect(doors[1]).toEqual({ x1: 200, y1: 0, x2: 200, y2: 100, open: true, locked: true });
  });

  it("finds the nearest door within the tolerance", () => {
    expect(doorAt(doors, 50, 5, 10)).toBe(0);
    expect(doorAt(doors, 195, 50, 10)).toBe(1);
    expect(doorAt(doors, 50, 30, 10)).toBeNull();
    expect(doorAt(doors, 120, 0, 25)).toBe(0); // Past the end, but close enough.
  });
});
