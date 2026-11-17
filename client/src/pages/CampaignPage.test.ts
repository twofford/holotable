import { describe, expect, it } from "vitest";
import { formatBytes } from "./CampaignPage";

describe("formatBytes", () => {
  it("uses KB below a megabyte and MB above", () => {
    expect(formatBytes(11341)).toBe("11 KB");
    expect(formatBytes(200)).toBe("1 KB");
    expect(formatBytes(1147680)).toBe("1.1 MB");
  });
});
