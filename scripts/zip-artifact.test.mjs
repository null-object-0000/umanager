import { describe, expect, it, vi } from "vitest";
import { fetchZipArtifact } from "./zip-artifact.mjs";

describe("ZIP artifact transport", () => {
  it("follows an allowed HTTPS redirect", async () => {
    const fetcher = vi.fn().mockResolvedValueOnce(new Response(null, { status: 302, headers: { location: "/archive.zip" } }))
      .mockResolvedValueOnce(new Response("zip bytes"));
    expect((await fetchZipArtifact("https://vendor.test/latest", ["vendor.test"], fetcher)).toString()).toBe("zip bytes");
    expect(fetcher.mock.calls[1][0]).toBe("https://vendor.test/archive.zip");
    expect(fetcher.mock.calls.every(([, options]) => options.redirect === "manual")).toBe(true);
  });
  it("never requests a redirect to HTTP or an unlisted host", async () => {
    for (const location of ["http://vendor.test/file", "https://vendor.test.attacker.test/file", "https://other.test/file"]) {
      const fetcher = vi.fn().mockResolvedValue(new Response(null, { status: 302, headers: { location } }));
      await expect(fetchZipArtifact("https://vendor.test/file", ["vendor.test"], fetcher)).rejects.toThrow();
      expect(fetcher).toHaveBeenCalledTimes(1);
    }
  });
  it("bounds both declared and chunked response sizes", async () => {
    for (const headers of [{}, { "content-length": "20" }]) {
      const fetcher = vi.fn().mockResolvedValue(new Response("oversized", { headers }));
      await expect(fetchZipArtifact("https://vendor.test/file", ["vendor.test"], fetcher, 4)).rejects.toThrow("大小上限");
    }
  });
});
