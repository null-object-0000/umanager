import { describe, it, expect } from "vitest";
import { selectTarRelease, fetchRestricted, tarEntry } from "./tar-source.mjs";
const source = {
  officialPageUrl: "https://antigravity.google/download", officialPageHosts: ["antigravity.google"],
  downloadHosts: ["storage.googleapis.com"],
  downloadUrlPrefix: "https://storage.googleapis.com/antigravity-public/antigravity-hub/",
  downloadUrlSuffix: "/linux-x64/Antigravity.tar.gz",
};
const url = `${source.downloadUrlPrefix}2.29.0-5723181030440960${source.downloadUrlSuffix}`;
const app = { packageName: "antigravity-app", architecture: "amd64", source };
describe("official tar.gz source", () => {
  it("selects the app x64 link separately from ARM and IDE, deduplicating HTML links", () => {
    const html = `<a href="${url}"></a><a href="${url}"></a><a href="${url.replace('linux-x64', 'linux-arm')}"></a>`;
    expect(selectTarRelease(html, source)).toEqual({ downloadUrl: url, version: "2.29.0" });
    expect(() => selectTarRelease(`${html} ${url.replace('2.29.0', '2.30.0')}`, source)).toThrow();
    expect(() => selectTarRelease(url.replace('2.29.0-', 'bad-'), source)).toThrow();
  });
  it("rejects redirects outside exact HTTPS hosts before fetching them", async () => {
    const requested = [];
    const fetcher = async (url) => { requested.push(url); return new Response(null, { status: 302, headers: { location: "https://storage.googleapis.com.evil.test/payload" } }); };
    await expect(fetchRestricted(url, source.downloadHosts, 100, fetcher)).rejects.toThrow("白名单");
    expect(requested).toHaveLength(1);
    await expect(fetchRestricted("http://storage.googleapis.com/payload", source.downloadHosts, 100, fetcher)).rejects.toThrow();
  });
  it("bounds streaming responses and reuses only matching versioned metadata", async () => {
    await expect(fetchRestricted(url, source.downloadHosts, 2, async () => new Response("123"))).rejects.toThrow("过大");
    let requests = 0;
    const previous = { ...app, source: undefined, downloadUrl: url, version: "2.29.0", size: 100, sha256: "a".repeat(64) };
    const result = await tarEntry(app, previous, async () => { requests++; return new Response(`<a href="${url}">Download</a>`); });
    expect(result).toEqual(previous);
    expect(requests).toBe(1);
  });
});
