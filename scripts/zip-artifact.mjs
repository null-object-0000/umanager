// This source kind accepts exact hosts only. Validate every redirect before
// sending a request, and bound streamed bytes even without Content-Length.
export async function fetchZipArtifact(url, hosts, fetcher = fetch, maxBytes = 1024 ** 3) {
  let target = new URL(url);
  for (let hop = 0; hop < 10; hop += 1) {
    if (target.protocol !== "https:" || target.username || target.password || !hosts.includes(target.hostname)) {
      throw new Error("ZIP 下载地址或重定向不属于 HTTPS 精确域名白名单");
    }
    const response = await fetcher(target.toString(), {
      redirect: "manual", headers: { "User-Agent": "UManager-feed/1.0" }, signal: AbortSignal.timeout(10 * 60_000),
    });
    if ([301, 302, 303, 307, 308].includes(response.status)) {
      await response.body?.cancel();
      const location = response.headers.get("location");
      if (!location) throw new Error("ZIP 重定向缺少 Location");
      target = new URL(location, target);
      continue;
    }
    if (!response.ok) { await response.body?.cancel(); throw new Error(`ZIP HTTP ${response.status}`); }
    if (Number(response.headers.get("content-length")) > maxBytes) {
      await response.body?.cancel(); throw new Error("ZIP 超过大小上限");
    }
    const chunks = [];
    let size = 0;
    for await (const chunk of response.body) {
      size += chunk.length;
      if (size > maxBytes) throw new Error("ZIP 超过大小上限");
      chunks.push(Buffer.from(chunk));
    }
    return Buffer.concat(chunks, size);
  }
  throw new Error("ZIP 重定向次数过多");
}
