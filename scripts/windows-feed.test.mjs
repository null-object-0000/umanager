import { describe, it, expect } from 'vitest';
import { createHash } from 'node:crypto';
import { validateWindowsUrl, wecomVersion, windowsEntry } from './windows-feed.mjs';
const source = {displayName:'企业微信',profile:'wecom-v1',downloadUrl:'https://work.weixin.qq.com/wework_admin/commdownload?platform=win',downloadHosts:['work.weixin.qq.com','dldir1.qq.com']};
const target = 'https://dldir1.qq.com/wework/work_weixin/WeCom_5.0.10.6015.exe';
describe('signed Windows feed generation', () => {
  it('extracts a four-part version and rejects ambiguous names', () => {
    expect(wecomVersion(target)).toBe('5.0.10.6015');
    expect(() => wecomVersion('https://dldir1.qq.com/latest.exe')).toThrow();
  });
  it('rejects credentials, HTTP, ports and deceptive hosts', () => {
    for (const u of ['http://dldir1.qq.com/x', 'https://dldir1.qq.com.evil.test/x', 'https://dldir1.qq.com:8443/x', 'https://user@dldir1.qq.com/x']) {
      expect(() => validateWindowsUrl(u, source.downloadHosts)).toThrow();
    }
  });
  it('hashes the actual installer and follows only allowlisted redirects', async () => {
    const body = Buffer.alloc(2048); body.write('MZ');
    const calls = [];
    const entry = await windowsEntry(source, async url => {
      calls.push(url);
      return url === source.downloadUrl ? new Response(null, {status:302,headers:{location:target}}) : new Response(body);
    });
    expect(calls).toEqual([source.downloadUrl,target]);
    expect(entry.sha256).toBe(createHash('sha256').update(body).digest('hex'));
    expect(entry.size).toBe(2048);
    expect(entry.downloadUrl).toBe(target);
  });
  it('never contacts an untrusted redirect destination', async () => {
    let requests = 0;
    await expect(windowsEntry(source, async () => { requests++; return new Response(null,{status:302,headers:{location:'https://evil.test/a.exe'}}); })).rejects.toThrow();
    expect(requests).toBe(1);
  });
  it('rejects HTML disguised as an installer', async () => {
    await expect(windowsEntry({...source,downloadUrl:target},async () => new Response('x'.repeat(2048)))).rejects.toThrow();
  });
});
