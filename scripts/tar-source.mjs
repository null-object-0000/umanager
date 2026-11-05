// CI-only resolver. Desktop clients consume signed metadata, never this HTML.
import { createHash } from "node:crypto";

export function selectTarRelease(html, source) {
  const links = [...new Set((html.match(/https:\/\/[^\s<>"']+/g) || [])
    .map((url) => url.replaceAll("&amp;", "&"))
    .filter((url) => url.startsWith(source.downloadUrlPrefix) && url.endsWith(source.downloadUrlSuffix)))];
  if (links.length !== 1) throw new Error(`期望唯一的官方 tar.gz 链接，实际 ${links.length}`);
  const url = links[0];
  const segment = url.slice(source.downloadUrlPrefix.length).split("/")[0];
  const version = source.downloadVersionPattern
    ? new RegExp(source.downloadVersionPattern).exec(url)?.[1]
    : /^(\d+\.\d+\.\d+)(?:-\d+)?$/.exec(segment)?.[1];
  if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) throw new Error("官方 tar.gz 地址缺少可识别版本");
  return { downloadUrl: url, version };
}

export async function fetchRestricted(url, hosts, maxBytes, fetcher = fetch) {
  for (let hop = 0; hop < 6; hop++) {
    const parsed = new URL(url);
    if (parsed.protocol !== "https:" || parsed.username || parsed.password || !hosts.includes(parsed.hostname)) {
      throw new Error("tar.gz 来源不在 HTTPS 精确域名白名单内");
    }
    const response = await fetcher(url, { redirect: "manual", signal: AbortSignal.timeout(30 * 60 * 1000) });
    if ([301, 302, 303, 307, 308].includes(response.status)) {
      await response.body?.cancel();
      url = new URL(response.headers.get("location"), url).href;
      continue;
    }
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    if (Number(response.headers.get("content-length")) > maxBytes) throw new Error("响应过大");
    let size = 0;
    const chunks = [];
    for await (const chunk of response.body) {
      size += chunk.length;
      if (size > maxBytes) throw new Error("响应过大");
      chunks.push(chunk);
    }
    return Buffer.concat(chunks);
  }
  throw new Error("重定向过多");
}

export async function tarEntry(app, previousEntry, fetcher = fetch) {
  const source = app.source;
  const page = await fetchRestricted(source.officialPageUrl, source.officialPageHosts, 4 * 1024 * 1024, fetcher);
  const release = selectTarRelease(page.toString("utf8"), source);
  if (previousEntry?.downloadUrl === release.downloadUrl && previousEntry.version === release.version
      && previousEntry.packageName === app.packageName && previousEntry.architecture === app.architecture
      && previousEntry.size > 0 && /^[a-f0-9]{64}$/.test(previousEntry.sha256 || "")) {
    return { ...previousEntry };
  }
  const bytes = await fetchRestricted(release.downloadUrl, source.downloadHosts, 1024 ** 3, fetcher);
  if (bytes[0] !== 0x1f || bytes[1] !== 0x8b) throw new Error("下载内容不是 gzip");
  return {
    packageName: app.packageName, architecture: app.architecture,
    ...release, size: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex"),
    websiteVersion: release.version, assetName: decodeURIComponent(new URL(release.downloadUrl).pathname.split("/").at(-1)),
  };
}
