// CI-only Windows installer discovery. Never executed by the desktop app.
import { createHash } from 'node:crypto';
const MAX_BYTES = 1024 * 1024 * 1024;

export function validateWindowsUrl(value, hosts) {
  const url = new URL(value);
  if (url.protocol !== 'https:' || url.username || url.password || (url.port && url.port !== '443') || !hosts.includes(url.hostname)) {
    throw new Error('Windows 安装包 URL 不在 HTTPS 精确白名单内');
  }
  return url;
}

export function wecomVersion(url) {
  const match = new URL(url).pathname.match(/\/WeCom_(\d+\.\d+\.\d+\.\d+)\.exe$/);
  if (!match) throw new Error('企业微信下载地址缺少明确版本');
  return match[1];
}

export async function windowsEntry(source, fetcher = fetch) {
  let url = source.downloadUrl;
  let response;
  for (let hop = 0; hop <= 5; hop++) {
    validateWindowsUrl(url, source.downloadHosts);
    response = await fetcher(url, { redirect: 'manual', signal: AbortSignal.timeout(600000) });
    if (response.status >= 300 && response.status < 400) {
      await response.body?.cancel();
      const location = response.headers.get('location');
      if (!location || hop === 5) throw new Error('安装包重定向无效');
      url = new URL(location, url).href;
    } else break;
  }
  if (!response.ok) throw new Error(`Windows 安装包 HTTP ${response.status}`);
  const version = wecomVersion(url);
  const hash = createHash('sha256');
  let size = 0;
  let magic = Buffer.alloc(0);
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > MAX_BYTES) throw new Error('Windows 安装包超出大小限制');
    if (magic.length < 2) magic = Buffer.concat([magic, Buffer.from(chunk)]).subarray(0, 2);
    hash.update(chunk);
  }
  if (magic.toString() !== 'MZ' || size < 1024) throw new Error('下载内容不是 Windows 安装程序');
  return { displayName: source.displayName, profile: source.profile, version, downloadUrl: url,
    downloadHosts: source.downloadHosts, size, sha256: hash.digest('hex') };
}
