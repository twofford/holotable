import { describe, expect, it } from "vitest";
import { grid } from "../map/coords";
import type { Token } from "../protocol/generated/Token";
import { renderTokens } from "./renderTokens";

const spec = { pixelsPerGrid: 50, origin: grid(-0.5, 0) }; // Grid offset 25 px right.
const a: Token = { id: 1, name: "A", assetId: 7, x: 2, y: 3, size: 1, ownerId: null, hidden: false };
const b: Token = { id: 2, name: "B", assetId: null, x: 0, y: 0, size: 2, ownerId: null, hidden: true };
const base = { tokens: [a, b], grid: spec, selectedId: 1, drag: null, remote: new Map(), imageUrl: (id: number) => `/img/${id}` };

describe("renderTokens", () => {
  it("places tokens in world pixels, with the grid offset", () => {
    const [ra, rb] = renderTokens(base);
    expect(ra).toMatchObject({ key: "token-1", x: 125, y: 150, size: 50, imageUrl: "/img/7", selected: true });
    expect(rb).toMatchObject({ x: 25, y: 0, size: 100, imageUrl: null, hidden: true, selected: false });
  });

  it("draws my drag on top with a ghost and a distance", () => {
    const drag = { tokenId: 1, startX: 2, startY: 3, x: 4.5, y: 3, distanceFt: 15 };
    const out = renderTokens({ ...base, drag });
    expect(out.map((t) => t.key)).toEqual(["ghost-1", "token-2", "token-1"]);
    expect(out[2]).toMatchObject({ x: 250, label: "15 ft" });
  });

  it("moves tokens other people are dragging", () => {
    const remote = new Map([[2, { x: 1, y: 1, userId: 9, at: 0 }]]);
    expect(renderTokens({ ...base, remote })[1]).toMatchObject({ x: 75, y: 50 });
  });
});
