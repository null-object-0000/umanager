import { describe, expect, it } from "vitest";
import { entryOrPrevious } from "./feed-fallback.mjs";

describe("entryOrPrevious", () => {
  it("keeps a freshly scraped entry unchanged", () => {
    const fresh = { version: "2.0" };
    expect(entryOrPrevious(fresh, { version: "1.0" })).toBe(fresh);
  });

  it("reuses the previous entry when the fresh scrape failed", () => {
    const previous = { version: "1.0", sha256: "abc" };
    const result = entryOrPrevious(null, previous);
    expect(result).toEqual(previous);
    expect(result).not.toBe(previous); // a copy, never aliases the previous feed
  });

  it("returns null when neither fresh nor previous is available", () => {
    expect(entryOrPrevious(null, null)).toBeNull();
    expect(entryOrPrevious(null, undefined)).toBeNull();
    expect(entryOrPrevious(undefined, undefined)).toBeNull();
  });
});
