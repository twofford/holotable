import { describe, expect, it } from "vitest";
import {
  type Camera,
  MAX_SCALE,
  MIN_SCALE,
  fitCamera,
  grid,
  gridToWorld,
  panBy,
  pinch,
  screen,
  screenToWorld,
  snapToSquare,
  world,
  worldToGrid,
  worldToScreen,
  zoomAt,
} from "./coords";

const spec = { pixelsPerGrid: 70, origin: grid(0, 0) };
const cam: Camera = { x: 100, y: 50, scale: 2 };

// toBeCloseTo-style check for points, since floating-point maths rounds.
function expectNear(actual: { x: number; y: number }, expected: { x: number; y: number }) {
  expect(actual.x).toBeCloseTo(expected.x, 9);
  expect(actual.y).toBeCloseTo(expected.y, 9);
}

describe("grid <-> world", () => {
  it("converts squares to image pixels", () => {
    expectNear(gridToWorld(grid(2.5, 1), spec), { x: 175, y: 70 });
  });

  it("respects a non-zero origin", () => {
    const shifted = { pixelsPerGrid: 50, origin: grid(-1, 2) };
    expectNear(gridToWorld(grid(0, 2), shifted), { x: 50, y: 0 });
  });

  it("round-trips", () => {
    const g = grid(7.25, 3.5);
    expectNear(worldToGrid(gridToWorld(g, spec), spec), g);
  });

  it("snaps to the containing square", () => {
    expect(snapToSquare(grid(3.9, 0.1))).toEqual({ x: 3, y: 0 });
  });
});

describe("screen <-> world", () => {
  it("applies offset and scale", () => {
    expectNear(screenToWorld(cam, screen(0, 0)), { x: 100, y: 50 });
    expectNear(screenToWorld(cam, screen(20, 10)), { x: 110, y: 55 });
  });

  it("round-trips", () => {
    const w = world(321, 123);
    expectNear(screenToWorld(cam, worldToScreen(cam, w)), w);
  });
});

describe("camera moves", () => {
  it("pans by screen pixels", () => {
    // Dragging right by 20 px at 200% shows world 10 px further left.
    expect(panBy(cam, 20, 0)).toEqual({ x: 90, y: 50, scale: 2 });
  });

  it("keeps the point under the pointer fixed when zooming", () => {
    const anchor = screen(300, 200);
    const before = screenToWorld(cam, anchor);
    const after = screenToWorld(zoomAt(cam, anchor, 1.5), anchor);
    expectNear(after, before);
  });

  it("clamps zoom to 10%–400%", () => {
    expect(zoomAt(cam, screen(0, 0), 100).scale).toBe(MAX_SCALE);
    expect(zoomAt(cam, screen(0, 0), 0.0001).scale).toBe(MIN_SCALE);
  });

  it("pinch: spreading fingers zooms around their midpoint", () => {
    const a0 = screen(100, 100), b0 = screen(200, 100);
    const a1 = screen(50, 100), b1 = screen(250, 100); // twice as far apart, same midpoint
    const next = pinch(cam, a0, b0, a1, b1);
    expect(next.scale).toBeCloseTo(4);
    expectNear(screenToWorld(next, screen(150, 100)), screenToWorld(cam, screen(150, 100)));
  });

  it("pinch: moving both fingers together pans", () => {
    const next = pinch(cam, screen(0, 0), screen(100, 0), screen(10, 20), screen(110, 20));
    expect(next.scale).toBeCloseTo(2);
    expectNear(next, { x: 95, y: 40 });
  });

  it("pinch: fingers on the same spot don't divide by zero", () => {
    const p = screen(5, 5);
    expect(pinch(cam, p, p, p, p).scale).toBe(2);
  });

  it("fits the whole map into the view, centred", () => {
    const fit = fitCamera(5600, 4200, 1000, 500, 1);
    expect(fit.scale).toBeCloseTo(500 / 4200);
    // The map's centre is at the view's centre.
    expectNear(worldToScreen(fit, world(2800, 2100)), { x: 500, y: 250 });
  });
});
